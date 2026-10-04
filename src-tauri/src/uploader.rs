//! The upload loop: polls the log folder (`watcher.rs`), sends what's due to the server one file
//! at a time, keeps `uploads.json` up to date and tells the window. Runs for as long as the app
//! does, window open or not.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::server::{self, UploadOutcome};
use crate::uploads::{self, Answer, RecentUpload, Record, Sent};
use crate::watcher::{self, Due, Tracker};
use crate::{config, log_folder, log_scan, Store};

/// The event the window listens to for `UploadStatus`.
pub const STATUS_EVENT: &str = "upload-status";

/// What's stopping uploads, if anything.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Problem {
    NoFolder,
    FolderUnreadable {
        message: String,
    },
    NoToken,
    /// The server turned the token down. Uploads wait until the host enters another one.
    TokenRejected {
        revoked: bool,
    },
    /// The credential store or the upload record failed.
    Local {
        message: String,
    },
}

/// What the window shows about uploads.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadStatus {
    pub problem: Option<Problem>,
    /// Ranked logs not uploaded as they are now: a match being played, or one waiting to retry.
    pub waiting: usize,
    /// Why the last upload failed, while it waits to be retried.
    pub retrying: Option<String>,
    pub recent: Vec<RecentUpload>,
}

pub struct Uploader {
    record_path: PathBuf,
    record: Mutex<Record>,
    status: Mutex<UploadStatus>,
    wake: Notify,
}

impl Uploader {
    /// Loads the record of past uploads from `record_path`. A broken record is started afresh: the
    /// server answers files it already has with `duplicate`.
    pub fn new(record_path: PathBuf) -> Self {
        let record = uploads::load(&record_path).unwrap_or_else(|e| {
            eprintln!("{e}. Starting a new upload record");
            Record::default()
        });
        Self {
            record_path,
            record: Mutex::new(record),
            status: Mutex::new(UploadStatus::default()),
            wake: Notify::new(),
        }
    }

    pub fn status(&self) -> UploadStatus {
        self.status.lock().unwrap().clone()
    }

    /// Polls now rather than at the next interval: a setting or the token changed.
    pub fn wake(&self) {
        self.wake.notify_one();
    }
}

/// Starts the loop. Call once, after `Store` and `Uploader` are managed.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut run = Run::default();
        loop {
            let status = run.tick(&app).await;
            let uploader = app.state::<Uploader>();
            let changed = {
                let mut current = uploader.status.lock().unwrap();
                let changed = *current != status;
                *current = status.clone();
                changed
            };
            if changed {
                let _ = app.emit(STATUS_EVENT, status);
            }
            // Until the next poll, or until woken.
            let interval = Duration::from_secs(config::POLL_INTERVAL_SECS);
            let _ = tokio::time::timeout(interval, uploader.wake.notified()).await;
        }
    });
}

#[derive(Default)]
struct Run {
    tracker: Tracker,
    /// The server and token that were turned down, so they aren't tried again. Memory only.
    rejected: Option<(String, String, bool)>,
    retrying: Option<String>,
}

impl Run {
    async fn tick(&mut self, app: &AppHandle) -> UploadStatus {
        let store = app.state::<Store>();
        let uploader = app.state::<Uploader>();
        let settings = store.get();
        let server_url = settings.server_url().to_string();
        let mut status = UploadStatus {
            retrying: self.retrying.clone(),
            ..UploadStatus::default()
        };
        let finish = |mut status: UploadStatus| {
            status.recent = uploader
                .record
                .lock()
                .unwrap()
                .recent(&server_url, config::RECENT_UPLOADS_SHOWN);
            status
        };

        let Some(folder) = log_folder::current(settings.log_folder.as_deref()).filter(|f| f.exists)
        else {
            status.problem = Some(Problem::NoFolder);
            return finish(status);
        };
        let poll = {
            let record = uploader.record.lock().unwrap();
            self.tracker
                .poll(&folder.path, Instant::now(), SystemTime::now(), |name| {
                    record.get(&server_url, name).map(|s| watcher::Sent {
                        size: s.size,
                        match_ends: s.match_ends,
                    })
                })
        };
        let poll = match poll {
            Ok(poll) => poll,
            Err(e) => {
                status.problem = Some(Problem::FolderUnreadable {
                    message: e.to_string(),
                });
                return finish(status);
            }
        };
        status.waiting = poll.waiting;

        let token = match store.tokens.get(&server_url) {
            Ok(Some(token)) => token,
            Ok(None) => {
                status.problem = Some(Problem::NoToken);
                return finish(status);
            }
            Err(message) => {
                status.problem = Some(Problem::Local { message });
                return finish(status);
            }
        };
        if let Some((url, bad, revoked)) = &self.rejected {
            if *url == server_url && *bad == token {
                status.problem = Some(Problem::TokenRejected { revoked: *revoked });
                return finish(status);
            }
            self.rejected = None;
        }

        for due in poll.due {
            match self.send(&uploader, &server_url, &token, &due).await {
                Ok(true) => status.waiting = status.waiting.saturating_sub(1),
                Ok(false) => {}
                Err(problem) => {
                    status.problem = Some(problem);
                    break;
                }
            }
        }
        status.retrying = self.retrying.clone();
        finish(status)
    }

    /// Uploads one file and records what came of it: `true` when it's done with, `false` when it
    /// waits to be retried. A problem that stops every upload is an error.
    async fn send(
        &mut self,
        uploader: &Uploader,
        server_url: &str,
        token: &str,
        due: &Due,
    ) -> Result<bool, Problem> {
        let bytes = match fs::read(&due.path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.tracker.failed(&due.name, Instant::now(), None);
                self.retrying = Some(format!("Couldn't read {}: {e}", due.name));
                return Ok(false);
            }
        };
        let size = bytes.len() as u64;
        let match_ends = log_scan::scan(&String::from_utf8_lossy(&bytes)).match_ends;
        let answer = if size > config::MAX_UPLOAD_BYTES {
            Answer::Refused {
                error: "too_large".into(),
                message: format!(
                    "Over the server's {} KB limit",
                    config::MAX_UPLOAD_BYTES / 1024
                ),
            }
        } else {
            let started_at = started_at(due);
            match server::upload(server_url, token, &due.name, started_at.as_deref(), bytes).await {
                UploadOutcome::Stored(answer) => Answer::Answered(answer),
                UploadOutcome::Refused { error, message } => Answer::Refused { error, message },
                UploadOutcome::TokenRejected { revoked } => {
                    self.rejected = Some((server_url.to_string(), token.to_string(), revoked));
                    return Err(Problem::TokenRejected { revoked });
                }
                UploadOutcome::Retry { message, after } => {
                    self.tracker.failed(&due.name, Instant::now(), after);
                    self.retrying = Some(message);
                    return Ok(false);
                }
            }
        };
        self.tracker.sent(&due.name);
        self.retrying = None;
        let mut record = uploader.record.lock().unwrap();
        record.put(
            server_url,
            &due.name,
            Sent {
                size,
                match_ends,
                at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                answer,
            },
        );
        uploads::save(&uploader.record_path, &record)
            .map(|()| true)
            .map_err(|message| Problem::Local { message })
    }
}

/// When the file was started: from its name, else when it was created on this PC.
fn started_at(due: &Due) -> Option<String> {
    log_scan::started_at_header(&due.name).or_else(|| {
        let created = fs::metadata(&due.path).ok()?.created().ok()?;
        Some(DateTime::<Local>::from(created).to_rfc3339_opts(SecondsFormat::Secs, false))
    })
}
