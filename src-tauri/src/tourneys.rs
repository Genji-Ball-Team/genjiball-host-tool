//! Tourneys (#8, #9, #10), the loop: asks the server for the tourney lobbies the host is assigned
//! to (`GET /api/host/tourneys`) every `TOURNEY_POLL_SECS`, and again when a code window opens,
//! shortly before a start, after a tourney match's end is uploaded and on "Check again". Tells the
//! host with a notification when a lobby's code becomes available, when it's about to start and
//! when its verify screenshot is due (`tourney::notices`). Runs for as long as the app does,
//! apart from the upload loop. The pure parts are in `tourney.rs`.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::server::{self, TourneyLobby, TourneysError};
use crate::tourney::{self, Notice};
use crate::uploader::Uploader;
use crate::{config, Store};

/// The event the window listens to for `TourneysStatus`.
pub const STATUS_EVENT: &str = "tourneys-status";

/// Why the list isn't there, or isn't up to date.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TourneyProblem {
    /// The settings file can't be used (the window says why).
    Settings,
    NoToken,
    TokenRejected {
        revoked: bool,
    },
    /// The server couldn't be asked (offline, an older server). The list is the last one read.
    Failed {
        message: String,
    },
}

/// A lobby as the window shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyView {
    #[serde(flatten)]
    pub lobby: TourneyLobby,
    /// Its lobby key, from its code values now or seen earlier: the upload history names the
    /// lobby of a tourney match by it. `None` while the tool hasn't seen it.
    pub lobby_key: Option<String>,
    /// This tool uploaded the end of its match (`MATCH_END ROUNDS`).
    pub match_uploaded: bool,
    /// Its verify screenshot is due (`tourney::needs_screenshot`).
    pub needs_screenshot: bool,
}

/// What the window shows about the host's tourneys.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TourneysStatus {
    /// The server this is about, as the settings were at the check.
    pub server_url: String,
    /// Soonest start first.
    pub lobbies: Vec<LobbyView>,
    /// When the server last answered with the list (RFC 3339), `None` while it hasn't.
    pub checked_at: Option<String>,
    pub problem: Option<TourneyProblem>,
}

#[derive(Default)]
pub struct Tourneys {
    status: Mutex<TourneysStatus>,
    /// One check at a time: the loop's, or the window's "Check again".
    run: tokio::sync::Mutex<Run>,
    wake: Notify,
}

impl Tourneys {
    pub fn status(&self) -> TourneysStatus {
        self.status.lock().unwrap().clone()
    }

    /// Asks the server again now: a tourney match was uploaded, or a setting or the token changed.
    pub fn refresh(&self) {
        self.wake.notify_one();
    }
}

/// Starts the loop. Call once, after `Store`, `Uploader` and `Tourneys` are managed.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let wait = check(&app).await;
            let tourneys = app.state::<Tourneys>();
            let _ = tokio::time::timeout(wait, tourneys.wake.notified()).await;
        }
    });
}

/// Asks the server for the host's lobbies now, tells the window and the host what's new, and
/// returns how long until the next check.
pub async fn check(app: &AppHandle) -> Duration {
    let tourneys = app.state::<Tourneys>();
    let mut run = tourneys.run.lock().await;
    let (status, wait) = run.tick(app).await;
    let changed = {
        let mut current = tourneys.status.lock().unwrap();
        let changed = *current != status;
        *current = status.clone();
        changed
    };
    if changed {
        let _ = app.emit(STATUS_EVENT, status);
    }
    wait
}

#[derive(Default)]
struct Run {
    /// The server and token the list is for. Memory only.
    target: Option<(String, String)>,
    lobbies: Vec<TourneyLobby>,
    checked_at: Option<String>,
    /// What the host was told already, per server and lobby: once each, for as long as the tool
    /// runs.
    told: HashSet<(String, i64, Notice)>,
}

impl Run {
    async fn tick(&mut self, app: &AppHandle) -> (TourneysStatus, Duration) {
        let store = app.state::<Store>();
        let uploader = app.state::<Uploader>();
        let settings_error = store.load_error();
        let settings = store.get();
        let server_url = settings.server_url().to_string();
        let every = settings.secs(&config::TOURNEY_POLL_SECS);
        let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
        let mut status = TourneysStatus {
            server_url: server_url.clone(),
            ..TourneysStatus::default()
        };
        let token = match settings_error {
            Some(_) => Err(TourneyProblem::Settings),
            None => match store.tokens.get(&server_url) {
                Ok(Some(token)) => Ok(token),
                Ok(None) => Err(TourneyProblem::NoToken),
                Err(message) => Err(TourneyProblem::Failed { message }),
            },
        };
        let token = match token {
            Ok(token) => token,
            Err(problem) => {
                self.target = None;
                self.lobbies.clear();
                self.checked_at = None;
                status.problem = Some(problem);
                return (status, every);
            }
        };
        let target = (server_url.clone(), token.clone());
        if self.target.as_ref() != Some(&target) {
            self.target = Some(target);
            self.lobbies.clear();
            self.checked_at = None;
        }

        match server::host_tourneys(&server_url, &token, timeout).await {
            Ok(list) => {
                uploader.learn_lobbies(&server_url, &list.lobbies);
                if list.lobbies != self.lobbies {
                    log::info!(
                        "Tourney lobbies assigned to you on {server_url}: {}",
                        list.lobbies.len()
                    );
                }
                self.lobbies = list.lobbies;
                self.checked_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));
            }
            Err(TourneysError::TokenRejected { revoked }) => {
                log::warn!("{server_url} turned the token down for the tourney list");
                self.lobbies.clear();
                self.checked_at = None;
                status.problem = Some(TourneyProblem::TokenRejected { revoked });
            }
            Err(TourneysError::Failed { message }) => {
                log::warn!("Couldn't read your tourneys from {server_url}: {message}");
                status.problem = Some(TourneyProblem::Failed { message });
            }
        }

        let now = Utc::now();
        let notice = Duration::from_secs(config::TOURNEY_START_NOTICE_SECS);
        status.checked_at = self.checked_at.clone();
        status.lobbies = self
            .lobbies
            .iter()
            .map(|lobby| {
                let key = lobby
                    .code
                    .as_ref()
                    .map(|c| c.lobby_key.clone())
                    .or_else(|| uploader.lobby_key(&server_url, lobby.id));
                let uploaded = uploader.tourney_ended(&server_url, lobby.id, key.as_deref());
                LobbyView {
                    lobby: lobby.clone(),
                    lobby_key: key,
                    match_uploaded: uploaded,
                    needs_screenshot: tourney::needs_screenshot(lobby, uploaded),
                }
            })
            .collect();
        // Only from a fresh answer: a stale list may be out of date.
        if status.problem.is_none() {
            for view in &status.lobbies {
                for due in tourney::notices(&view.lobby, view.match_uploaded, now, notice) {
                    if self.told.insert((server_url.clone(), view.lobby.id, due)) {
                        tell(app, due, &view.lobby);
                    }
                }
            }
        }
        // Just after the next moment something is due, if that's sooner than the usual check.
        let wait = tourney::next_wake(&self.lobbies, now, notice)
            .and_then(|at| (at - now).to_std().ok())
            .map_or(every, |until| (until + Duration::from_secs(1)).min(every));
        (status, wait)
    }
}

/// Shows the host a notification, through the notification plugin's Rust API (the window has no
/// notification permission).
fn tell(app: &AppHandle, notice: Notice, lobby: &TourneyLobby) {
    use tauri_plugin_notification::NotificationExt;
    let (title, body) = tourney::notice_text(notice, lobby);
    log::info!("Telling the host: {title}. {body}");
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("Couldn't show a notification: {e}");
    }
}
