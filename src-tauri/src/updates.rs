//! The tool updating itself from GitHub releases (`tauri-plugin-updater`): looks for a newer
//! signed installer at startup and every `UPDATE_CHECK_SECS`, and installs it when the host asks.
//! The checks run here, not in the window, so they go on from the tray.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{config, Store};

/// What the window shows about updates.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The version waiting to be installed, as the last check found it.
    pub available: Option<String>,
    /// When the last check ended (ISO 8601), whether or not it worked.
    pub checked_at: Option<String>,
    /// Why the last check failed (offline, no release published yet). Never stops the tool: the
    /// window shows it quietly.
    pub error: Option<String>,
}

/// The status after a check. A check that fails keeps the update an earlier one found: being
/// offline now doesn't make it go away.
fn after_check(
    before: &UpdateStatus,
    result: Result<Option<String>, String>,
    now: String,
) -> UpdateStatus {
    match result {
        Ok(available) => UpdateStatus {
            available,
            checked_at: Some(now),
            error: None,
        },
        Err(error) => UpdateStatus {
            available: before.available.clone(),
            checked_at: Some(now),
            error: Some(error),
        },
    }
}

/// Whether the next check is due: never checked, or `interval` has passed since the last one.
fn due(last_check: Option<Instant>, now: Instant, interval: Duration) -> bool {
    match last_check {
        None => true,
        Some(last) => now.saturating_duration_since(last) >= interval,
    }
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<UpdateStatus>,
    /// The update the last check found, for `install`.
    update: Mutex<Option<Update>>,
    last_check: Mutex<Option<Instant>>,
    /// A check or an install is running: a second one waits for it.
    busy: AtomicBool,
}

impl Updates {
    pub fn status(&self) -> UpdateStatus {
        self.status.lock().unwrap().clone()
    }

    /// Takes `busy` until the guard is dropped, or `None` if something else holds it.
    fn begin(&self) -> Option<Busy<'_>> {
        (!self.busy.swap(true, Ordering::AcqRel)).then_some(Busy(&self.busy))
    }
}

struct Busy<'a>(&'a AtomicBool);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Looks for an update now and tells the window the result. Returns the status as it stands if a
/// check or install is already running.
pub async fn check(app: &AppHandle) -> UpdateStatus {
    let updates = app.state::<Updates>();
    let Some(_busy) = updates.begin() else {
        return updates.status();
    };
    let timeout = app
        .state::<Store>()
        .get()
        .secs(&config::REQUEST_TIMEOUT_SECS);
    // The timeout wraps the check rather than going on the updater: there it would also cut the
    // download of the installer short.
    let found = match tokio::time::timeout(timeout, look(app)).await {
        Ok(result) => result,
        Err(_) => Err("the update server didn't answer in time".to_string()),
    };
    let version = found
        .as_ref()
        .map(|update| update.as_ref().map(|u| u.version.clone()))
        .map_err(Clone::clone);
    let now = chrono::Local::now().to_rfc3339();
    let status = {
        let mut status = updates.status.lock().unwrap();
        *status = after_check(&status, version, now);
        status.clone()
    };
    if let Ok(update) = found {
        *updates.update.lock().unwrap() = update;
    }
    *updates.last_check.lock().unwrap() = Some(Instant::now());
    let _ = app.emit("update-status", &status);
    status
}

async fn look(app: &AppHandle) -> Result<Option<Update>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    updater.check().await.map_err(|e| e.to_string())
}

/// Downloads the update the last check found, checks its signature and runs the installer, which
/// ends the tool and starts the new version. Only returns when that didn't work.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let update = updates
        .update
        .lock()
        .unwrap()
        .clone()
        .ok_or("There's no update to install: check for updates first")?;
    let _busy = updates
        .begin()
        .ok_or("An update check or install is already running")?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| format!("Couldn't install the update: {e}"))?;
    // Windows' installer ends the tool; elsewhere the new version needs a start.
    app.restart();
}

/// Checks at startup and then every `UPDATE_CHECK_SECS`, read from the settings each time.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let interval = app.state::<Store>().get().secs(&config::UPDATE_CHECK_SECS);
            let last = *app.state::<Updates>().last_check.lock().unwrap();
            if due(last, Instant::now(), interval) {
                check(&app).await;
            }
            tokio::time::sleep(Duration::from_secs(config::UPDATE_TICK_SECS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(available: Option<&str>) -> UpdateStatus {
        UpdateStatus {
            available: available.map(String::from),
            checked_at: Some("then".into()),
            error: None,
        }
    }

    #[test]
    fn a_found_update_replaces_the_old_status() {
        let after = after_check(&status(None), Ok(Some("0.3.0".into())), "now".into());
        assert_eq!(
            after,
            UpdateStatus {
                available: Some("0.3.0".into()),
                checked_at: Some("now".into()),
                error: None
            }
        );
    }

    #[test]
    fn an_up_to_date_check_clears_the_update_and_the_error() {
        let before = UpdateStatus {
            error: Some("offline".into()),
            ..status(Some("0.2.0"))
        };
        let after = after_check(&before, Ok(None), "now".into());
        assert_eq!(after.available, None);
        assert_eq!(after.error, None);
    }

    #[test]
    fn a_failed_check_keeps_the_update_found_before() {
        let after = after_check(&status(Some("0.3.0")), Err("offline".into()), "now".into());
        assert_eq!(after.available.as_deref(), Some("0.3.0"));
        assert_eq!(after.error.as_deref(), Some("offline"));
        assert_eq!(after.checked_at.as_deref(), Some("now"));
    }

    #[test]
    fn a_check_is_due_at_the_start_and_once_the_interval_has_passed() {
        let start = Instant::now();
        let interval = Duration::from_secs(config::UPDATE_CHECK_SECS.default);
        assert!(due(None, start, interval));
        assert!(!due(Some(start), start, interval));
        assert!(!due(
            Some(start),
            start + interval - Duration::from_secs(1),
            interval
        ));
        assert!(due(Some(start), start + interval, interval));
    }

    #[test]
    fn only_one_check_or_install_runs_at_a_time() {
        let updates = Updates::default();
        let first = updates.begin();
        assert!(first.is_some());
        assert!(updates.begin().is_none());
        drop(first);
        assert!(updates.begin().is_some());
    }
}
