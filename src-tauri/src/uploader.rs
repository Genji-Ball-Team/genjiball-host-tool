//! The upload loop: polls the log folder (`watcher.rs`), sends what's due to the server one file
//! at a time, keeps `uploads.json` up to date and tells the window. Runs for as long as the app
//! does, window open or not.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::history::{self, Page, QueueSnapshot};
use crate::server::{self, Host, TokenCheck, UploadOutcome};
use crate::uploads::{self, Answer, Record, Sent};
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
    /// The first page of the upload history. The window asks for the others.
    pub history: Page,
    /// Goes up whenever any page of the history changes: the window then asks for the page it
    /// shows again.
    pub history_revision: u64,
    /// Whose token it is, as the server last said: the window shows an untrusted host.
    pub host: Option<Host>,
}

pub struct Uploader {
    record_path: PathBuf,
    record: Mutex<Record>,
    status: Mutex<UploadStatus>,
    /// The ranked files waiting to be uploaded, as of the last poll.
    queue: Mutex<QueueSnapshot>,
    /// Failed uploads the host asked to retry now, by file name.
    retries: Mutex<Vec<String>>,
    wake: Notify,
    /// Ask the server for the host and the listed matches' status at the next poll.
    refresh: AtomicBool,
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
            queue: Mutex::new(QueueSnapshot::default()),
            retries: Mutex::new(Vec::new()),
            wake: Notify::new(),
            refresh: AtomicBool::new(false),
        }
    }

    pub fn status(&self) -> UploadStatus {
        self.status.lock().unwrap().clone()
    }

    /// Page `page` of the upload history for `server_url`, with the files waiting in `folder` (the
    /// log folder now, `None` when there's none).
    pub fn history(&self, server_url: &str, folder: Option<&Path>, page: usize) -> Page {
        let queue = self.queue.lock().unwrap();
        history::page(
            &self.record.lock().unwrap(),
            server_url,
            queue.queue(server_url, folder),
            page,
            config::UPLOADS_PAGE_SIZE,
        )
    }

    /// See `UploadStatus::history_revision`. Both parts only go up.
    fn history_revision(&self) -> u64 {
        let queue = self.queue.lock().unwrap().revision();
        queue + self.record.lock().unwrap().revision()
    }

    /// Whether a match uploaded to `server_url` has this id on the site and is public there.
    pub fn has_public_match(&self, server_url: &str, match_id: i64) -> bool {
        self.record
            .lock()
            .unwrap()
            .has_public_match(server_url, match_id)
    }

    /// Tries a failed upload of `file` again now, through the usual queue: its backoff is dropped
    /// and the loop woken. A file that isn't a failed upload (it was uploaded since) is left alone.
    pub fn retry(&self, file: String) {
        self.retries.lock().unwrap().push(file);
        self.wake();
    }

    /// Polls now rather than at the next interval: a setting or the token changed.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Asks the server for the host and the listed matches' status now, not at the next refresh.
    pub fn refresh(&self) {
        self.refresh.store(true, Ordering::Relaxed);
        self.wake();
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
    /// The server and token the host and the match statuses were last asked with, and when.
    refreshed: Option<(String, String, Instant)>,
    host: Option<Host>,
    /// A refreshed status that couldn't be saved: saved at the next refresh even if unchanged.
    unsaved: bool,
    /// The server that answered "too many uploads" (`429`), and until when nothing is sent to it.
    held: Option<(String, Instant)>,
}

impl Run {
    /// Whether nothing may be uploaded to `server_url` at `now`: it answered `429` and its
    /// `Retry-After` isn't over yet.
    fn held(&self, server_url: &str, now: Instant) -> bool {
        matches!(&self.held, Some((url, until)) if url == server_url && now < *until)
    }

    /// Whose token it is, only if the server was last asked with this server and token.
    fn known_host(&self, server_url: &str, token: &str) -> Option<Host> {
        match &self.refreshed {
            Some((url, t, _)) if url == server_url && t == token => self.host.clone(),
            _ => None,
        }
    }

    async fn tick(&mut self, app: &AppHandle) -> UploadStatus {
        let store = app.state::<Store>();
        let uploader = app.state::<Uploader>();
        let settings = store.get();
        let server_url = settings.server_url().to_string();
        let mut status = UploadStatus {
            retrying: self.retrying.clone(),
            ..UploadStatus::default()
        };
        // `folder`: the folder just polled, `None` when there's none to poll.
        let finish = |run: &Run, folder: Option<&Path>, mut status: UploadStatus| {
            {
                let mut queue = uploader.queue.lock().unwrap();
                match folder {
                    Some(folder) => queue.set(&server_url, folder, run.tracker.queue()),
                    None => queue.clear(),
                }
            }
            status.history = uploader.history(&server_url, folder, 0);
            status.history_revision = uploader.history_revision();
            status
        };

        let Some(folder) = log_folder::current(settings.log_folder.as_deref()).filter(|f| f.exists)
        else {
            status.problem = Some(Problem::NoFolder);
            return finish(self, None, status);
        };
        let polled = Some(folder.path.as_path());
        // The retries the host asked for, before the poll picks what's due.
        for file in std::mem::take(&mut *uploader.retries.lock().unwrap()) {
            self.tracker.retry(&file);
        }
        let poll = {
            let record = uploader.record.lock().unwrap();
            // Before the first token for this server, count only what's written from now on.
            let since = record.started(&server_url).unwrap_or_else(SystemTime::now);
            self.tracker.poll(
                &folder.path,
                Instant::now(),
                SystemTime::now(),
                since,
                |name| {
                    record.get(&server_url, name).map(|s| watcher::Sent {
                        size: s.size,
                        match_ends: s.match_ends,
                    })
                },
            )
        };
        let poll = match poll {
            Ok(poll) => poll,
            Err(e) => {
                status.problem = Some(Problem::FolderUnreadable {
                    message: e.to_string(),
                });
                return finish(self, None, status);
            }
        };
        status.waiting = poll.waiting;

        let token = match store.tokens.get(&server_url) {
            Ok(Some(token)) => token,
            Ok(None) => {
                // The token was forgotten: whose it was no longer matters.
                self.host = None;
                self.refreshed = None;
                status.problem = Some(Problem::NoToken);
                return finish(self, polled, status);
            }
            Err(message) => {
                status.problem = Some(Problem::Local { message });
                return finish(self, polled, status);
            }
        };
        status.host = self.known_host(&server_url, &token);
        {
            let mut record = uploader.record.lock().unwrap();
            if record.start(&server_url, SystemTime::now()) {
                if let Err(message) = uploads::save(&uploader.record_path, &record) {
                    status.problem = Some(Problem::Local { message });
                    drop(record);
                    return finish(self, polled, status);
                }
            }
        }
        if let Some((url, bad, revoked)) = &self.rejected {
            if *url == server_url && *bad == token {
                status.problem = Some(Problem::TokenRejected { revoked: *revoked });
                return finish(self, polled, status);
            }
            self.rejected = None;
        }

        for due in poll.due {
            if self.held(&server_url, Instant::now()) {
                break;
            }
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
        if status.problem.is_none() {
            if let Err(problem) = self.refresh(&uploader, &server_url, &token).await {
                status.problem = Some(problem);
            }
        }
        status.host = self.known_host(&server_url, &token);
        finish(self, polled, status)
    }

    /// Every `STATUS_REFRESH_SECS`, or when asked: asks the server whose token it is (the trust
    /// may have changed) and the status now of the listed matches (an admin may have accepted
    /// one), and records it. Failing to ask isn't a problem: it's asked again next time.
    async fn refresh(
        &mut self,
        uploader: &Uploader,
        server_url: &str,
        token: &str,
    ) -> Result<(), Problem> {
        let asked = uploader.refresh.swap(false, Ordering::Relaxed);
        let due = match &self.refreshed {
            Some((url, t, at)) if url == server_url && t == token => {
                asked || at.elapsed() >= Duration::from_secs(config::STATUS_REFRESH_SECS)
            }
            _ => {
                self.host = None;
                true
            }
        };
        if !due {
            return Ok(());
        }
        self.refreshed = Some((server_url.to_string(), token.to_string(), Instant::now()));

        match server::check_token(server_url, token).await {
            TokenCheck::Ok { host } => self.host = Some(host),
            check @ (TokenCheck::Unknown | TokenCheck::Revoked) => {
                let revoked = check == TokenCheck::Revoked;
                self.host = None;
                self.rejected = Some((server_url.to_string(), token.to_string(), revoked));
                return Err(Problem::TokenRejected { revoked });
            }
            // Keep what the server said last.
            TokenCheck::Unreachable { .. } => {}
        }

        let keys = uploader
            .record
            .lock()
            .unwrap()
            .recent_match_keys(server_url, config::MAX_STATUS_KEYS);
        if !keys.is_empty() {
            match server::match_states(server_url, token, &keys).await {
                Ok(states) => {
                    let mut record = uploader.record.lock().unwrap();
                    if record.update_states(server_url, &states) {
                        self.unsaved = true;
                    }
                }
                Err(message) => eprintln!("Couldn't refresh the match status: {message}"),
            }
        }

        // Saved last, so a failing save doesn't stop the checks above. It's tried again next time.
        if self.unsaved {
            let saved = uploads::save(&uploader.record_path, &uploader.record.lock().unwrap());
            self.unsaved = saved.is_err();
            saved.map_err(|message| Problem::Local { message })?;
        }
        Ok(())
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
        // Only its complete lines: the game may be halfway through writing the next one.
        let bytes = match fs::read(&due.path) {
            Ok(mut bytes) => {
                bytes.truncate(log_scan::complete_lines(&bytes).len());
                bytes
            }
            Err(e) => {
                let message = format!("Couldn't read {}: {e}", due.name);
                self.tracker
                    .failed(&due.name, Instant::now(), None, message.clone());
                self.retrying = Some(message);
                return Ok(false);
            }
        };
        let size = bytes.len() as u64;
        let scan = log_scan::scan(&String::from_utf8_lossy(&bytes));
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
                UploadOutcome::Stored(answer) => {
                    // The answer has no match id on the site: the status refresh brings it, so
                    // ask for one at the end of this poll rather than in `STATUS_REFRESH_SECS`.
                    if !answer.matches.is_empty() {
                        uploader.refresh.store(true, Ordering::Relaxed);
                    }
                    Answer::Answered(answer)
                }
                UploadOutcome::Refused { error, message } => Answer::Refused { error, message },
                UploadOutcome::TokenRejected { revoked } => {
                    self.rejected = Some((server_url.to_string(), token.to_string(), revoked));
                    return Err(Problem::TokenRejected { revoked });
                }
                UploadOutcome::Retry { message, after } => {
                    self.tracker
                        .failed(&due.name, Instant::now(), after, message.clone());
                    self.retrying = Some(message);
                    return Ok(false);
                }
                UploadOutcome::RateLimited { message, after } => {
                    // Every upload to this server waits, not only this file.
                    let now = Instant::now();
                    let wait = after.unwrap_or_else(|| watcher::backoff(1));
                    self.held = Some((server_url.to_string(), watcher::later(now, wait)));
                    self.tracker
                        .failed(&due.name, now, Some(wait), message.clone());
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
                match_ends: scan.match_ends,
                at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                players: scan.players,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::async_runtime::block_on;

    const EXAMPLE: &str = include_str!("../tests/fixtures/ranked-log-example.txt");

    /// A log file in `dir` holding `text`, due to upload.
    fn due(dir: &Path, text: &str) -> Due {
        let name = "Log-2026-10-02-20-15-33.txt";
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        Due {
            name: name.into(),
            path,
        }
    }

    #[test]
    fn too_many_uploads_holds_the_whole_server_as_long_as_it_says() {
        let dir = tempfile::tempdir().unwrap();
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let mut run = Run::default();
        let server = server::test_server(
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 7200\r\nContent-Length: 0\r\n\r\n"
                .into(),
            EXAMPLE,
        );
        let start = Instant::now();
        let sent = block_on(run.send(&uploader, &server, "t", &due(dir.path(), EXAMPLE)));
        assert_eq!(sent, Ok(false));
        // Two hours, past `RETRY_MAX_SECS`, for every file to that server; not another server.
        assert!(run.held(
            &server,
            start + Duration::from_secs(config::RETRY_MAX_SECS + 60)
        ));
        assert!(!run.held(&server, Instant::now() + Duration::from_secs(7200)));
        assert!(!run.held("https://test.genjiball.us", start));
    }
}
