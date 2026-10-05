//! The upload loop: polls the log folder (`watcher.rs`), sends what's due to the server one file
//! at a time, keeps `uploads.json` up to date and tells the window. Runs for as long as the app
//! does, window open or not.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::history::{self, Page, QueueSnapshot};
use crate::server::{self, Host, TokenCheck, UploadOutcome};
use crate::uploads::{self, Answer, Record, Sent};
use crate::watcher::{self, Due, Timing, Tracker};
use crate::{config, log_folder, log_scan, Store};

/// The event the window listens to for `UploadStatus`.
pub const STATUS_EVENT: &str = "upload-status";

/// What's stopping uploads, if anything.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Problem {
    /// The settings file can't be used (the window says why). Uploads wait until it's fixed.
    Settings,
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
    /// The server and log folder this is about, as the settings were when the poll began. The
    /// window drops a status for others: a poll that was still running when the host changed them.
    pub server_url: String,
    pub log_folder: Option<PathBuf>,
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
    /// Why `uploads.json` couldn't be read, while it can't. It holds when uploads started, so
    /// it isn't written over and nothing is uploaded until it's fixed or deleted.
    record_error: Mutex<Option<String>>,
    status: Mutex<UploadStatus>,
    /// The ranked files waiting to be uploaded, as of the last poll.
    queue: Mutex<QueueSnapshot>,
    /// Failed uploads the host asked to retry now, by file name.
    retries: Mutex<Vec<String>>,
    wake: Notify,
    /// Ask the server for the host and the listed matches' status at the next poll.
    refresh: AtomicBool,
    /// A token was saved, or a check found the token works: forget a rejection at the next poll.
    token_ok: AtomicBool,
    /// Goes up each time the server, the log folder or the token changes (`changed`): a poll
    /// that began before stops sending.
    generation: AtomicU64,
}

impl Uploader {
    /// Loads the record of past uploads from `record_path`. A missing one is a first run; one that
    /// can't be read is reported (`save`) until it can.
    pub fn new(record_path: PathBuf) -> Self {
        let (record, record_error) = match uploads::load(&record_path) {
            Ok(record) => (record, None),
            Err(e) => (Record::default(), Some(e)),
        };
        Self {
            record_path,
            record: Mutex::new(record),
            record_error: Mutex::new(record_error),
            status: Mutex::new(UploadStatus::default()),
            queue: Mutex::new(QueueSnapshot::default()),
            retries: Mutex::new(Vec::new()),
            wake: Notify::new(),
            refresh: AtomicBool::new(false),
            token_ok: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }
    }

    /// The server, the log folder or the token changed: what a poll under way still has to send
    /// waits for the next one, which starts now with the new settings.
    pub fn changed(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.wake();
    }

    /// Whether nothing changed (`changed`) since `generation` was read.
    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::Relaxed) == generation
    }

    /// The host saved a token, or "Check again" found it works: uploads stopped by the server
    /// turning it down earlier try again.
    pub fn token_ok(&self) {
        self.token_ok.store(true, Ordering::Relaxed);
        self.wake();
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

    /// A token was saved for `server_url`: uploads to it start now, unless they already had.
    /// Saved at once; if that fails, the loop reports it and tries again before any upload.
    pub fn start(&self, server_url: &str) {
        self.record
            .lock()
            .unwrap()
            .start(server_url, SystemTime::now());
        let _ = self.save();
        self.wake();
    }

    /// Writes what changed in the record to `uploads.json`. An error until that works, and while
    /// the file can't be read (it's read again each time, and never written over meanwhile).
    fn save(&self) -> Result<(), String> {
        let mut error = self.record_error.lock().unwrap();
        if error.is_some() {
            match uploads::load(&self.record_path) {
                Ok(record) => *self.record.lock().unwrap() = record,
                Err(e) => {
                    *error = Some(e.clone());
                    return Err(format!(
                        "{e}. Uploads wait until it's fixed, or deleted to start the record afresh"
                    ));
                }
            }
            *error = None;
        }
        uploads::save(&self.record_path, &mut self.record.lock().unwrap())
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
            let interval = app.state::<Store>().get().secs(&config::POLL_INTERVAL_SECS);
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
    /// The server that answered "too many uploads" (`429`), and until when nothing is sent to it.
    held: Option<(String, Instant)>,
}

impl Run {
    /// Whether nothing may be uploaded to `server_url` at `now`: it answered `429` and its
    /// `Retry-After` isn't over yet.
    fn held(&self, server_url: &str, now: Instant) -> bool {
        matches!(&self.held, Some((url, until)) if url == server_url && now < *until)
    }

    /// Whether the server turned this token down (`Some(revoked)`), unless the host has saved a
    /// token or seen it work since (`Uploader::token_ok`).
    fn rejection(&mut self, uploader: &Uploader, server_url: &str, token: &str) -> Option<bool> {
        if uploader.token_ok.swap(false, Ordering::Relaxed) {
            self.rejected = None;
        }
        match &self.rejected {
            Some((url, bad, revoked)) if url == server_url && bad == token => Some(*revoked),
            _ => {
                self.rejected = None;
                None
            }
        }
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
        // Before the settings: a change after this stops what's left of this poll's uploads.
        let generation = uploader.generation.load(Ordering::Relaxed);
        // First: a settings file fixed by hand is read again here.
        let settings_error = store.load_error();
        let settings = store.get();
        let server_url = settings.server_url().to_string();
        self.tracker.timing = Timing::new(&settings);
        let timeout = settings.secs(&config::REQUEST_TIMEOUT_SECS);
        let refresh_every = settings.secs(&config::STATUS_REFRESH_SECS);
        let mut status = UploadStatus {
            server_url: server_url.clone(),
            log_folder: log_folder::current(settings.log_folder.as_deref()).map(|f| f.path),
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

        if settings_error.is_some() {
            status.problem = Some(Problem::Settings);
            return finish(self, None, status);
        }
        // Uploads to a server start once it has a token, folder or not. `save_token` starts them;
        // this is for a token from before the record was (a first run, or a deleted record).
        let token = store.tokens.get(&server_url);
        if let Ok(Some(_)) = token {
            let mut record = uploader.record.lock().unwrap();
            record.start(&server_url, SystemTime::now());
        }
        // Nothing is uploaded until the record, with when uploads started, is saved.
        if let Err(message) = uploader.save() {
            status.problem = Some(Problem::Local { message });
            return finish(self, None, status);
        }

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

        let token = match token {
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
        if let Some(revoked) = self.rejection(&uploader, &server_url, &token) {
            status.problem = Some(Problem::TokenRejected { revoked });
            return finish(self, polled, status);
        }

        let (done, problem) = self
            .send_due(
                &uploader,
                generation,
                &server_url,
                &token,
                &poll.due,
                timeout,
            )
            .await;
        status.waiting = status.waiting.saturating_sub(done);
        status.problem = problem;
        status.retrying = self.retrying.clone();
        if status.problem.is_none() && uploader.is_current(generation) {
            if let Err(problem) = self
                .refresh(
                    &uploader,
                    generation,
                    &server_url,
                    &token,
                    refresh_every,
                    timeout,
                )
                .await
            {
                status.problem = Some(problem);
            }
        }
        status.host = self.known_host(&server_url, &token);
        finish(self, polled, status)
    }

    /// Every `every` (`STATUS_REFRESH_SECS`), or when asked: asks the server whose token it is (the
    /// trust may have changed) and the status now of the listed matches (an admin may have
    /// accepted one), and records it. Failing to ask isn't a problem: it's asked again next time.
    async fn refresh(
        &mut self,
        uploader: &Uploader,
        generation: u64,
        server_url: &str,
        token: &str,
        every: Duration,
        timeout: Duration,
    ) -> Result<(), Problem> {
        let asked = uploader.refresh.swap(false, Ordering::Relaxed);
        let due = match &self.refreshed {
            Some((url, t, at)) if url == server_url && t == token => asked || at.elapsed() >= every,
            _ => {
                self.host = None;
                true
            }
        };
        if !due {
            return Ok(());
        }
        self.refreshed = Some((server_url.to_string(), token.to_string(), Instant::now()));

        match server::check_token(server_url, token, timeout).await {
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
        if !keys.is_empty() && uploader.is_current(generation) {
            match server::match_states(server_url, token, &keys, timeout).await {
                Ok(states) => {
                    let mut record = uploader.record.lock().unwrap();
                    record.update_states(server_url, &states);
                }
                Err(message) => eprintln!("Couldn't refresh the match status: {message}"),
            }
        }
        // A save that fails is tried again at the next poll, before any upload.
        uploader
            .save()
            .map_err(|message| Problem::Local { message })
    }

    /// Uploads the files `due`, oldest first, until one stops every upload, the server says to
    /// hold off (`429`), or the settings or token changed since `generation` (the rest waits for
    /// the next poll, with the new ones). How many are done with, and the problem if one came up.
    async fn send_due(
        &mut self,
        uploader: &Uploader,
        generation: u64,
        server_url: &str,
        token: &str,
        due: &[Due],
        timeout: Duration,
    ) -> (usize, Option<Problem>) {
        let mut done = 0;
        for due in due {
            if !uploader.is_current(generation) || self.held(server_url, Instant::now()) {
                break;
            }
            match self.send(uploader, server_url, token, due, timeout).await {
                Ok(true) => done += 1,
                Ok(false) => {}
                Err(problem) => return (done, Some(problem)),
            }
        }
        (done, None)
    }

    /// Uploads one file and records what came of it: `true` when it's done with, `false` when it
    /// waits to be retried. A problem that stops every upload is an error.
    async fn send(
        &mut self,
        uploader: &Uploader,
        server_url: &str,
        token: &str,
        due: &Due,
        timeout: Duration,
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
            let sent = server::upload(
                server_url,
                token,
                &due.name,
                started_at.as_deref(),
                bytes,
                timeout,
            );
            match sent.await {
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
                    let timing = self.tracker.timing;
                    let wait = after.unwrap_or_else(|| timing.backoff(1));
                    self.held = Some((server_url.to_string(), timing.later(now, wait)));
                    self.tracker
                        .failed(&due.name, now, Some(wait), message.clone());
                    self.retrying = Some(message);
                    return Ok(false);
                }
            }
        };
        self.tracker.sent(&due.name);
        self.retrying = None;
        uploader.record.lock().unwrap().put(
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
        uploader
            .save()
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
    const TIMEOUT: Duration = Duration::from_secs(config::REQUEST_TIMEOUT_SECS.default);

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

    const SERVER: &str = "https://genjiball.us";

    #[test]
    fn saving_a_token_starts_uploads_to_its_server_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uploads.json");
        let uploader = Uploader::new(path.clone());
        uploader.start(SERVER);
        let started = uploads::load(&path).unwrap().started(SERVER).unwrap();
        // Saving a token again (or another one) doesn't move it.
        std::thread::sleep(Duration::from_millis(1100));
        uploader.start(SERVER);
        assert_eq!(uploads::load(&path).unwrap().started(SERVER), Some(started));
        // Nor does a restart.
        let uploader = Uploader::new(path.clone());
        uploader.start(SERVER);
        assert_eq!(
            uploader.record.lock().unwrap().started(SERVER),
            Some(started)
        );
    }

    #[test]
    fn a_record_that_cant_be_read_is_never_written_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uploads.json");
        fs::write(&path, "{ broken").unwrap();
        let uploader = Uploader::new(path.clone());
        uploader.start(SERVER);
        let error = uploader.save().unwrap_err();
        assert!(error.contains("isn't a valid upload record"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ broken");

        // Fixed by hand: its `started` is kept, not the one from while it couldn't be read.
        let mut fixed = Record::default();
        let then = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        fixed.start(SERVER, then);
        uploads::save(&path, &mut fixed).unwrap();
        uploader.save().unwrap();
        assert_eq!(uploader.record.lock().unwrap().started(SERVER), Some(then));
    }

    #[test]
    fn a_deleted_record_that_couldnt_be_read_starts_afresh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uploads.json");
        fs::write(&path, "{ broken").unwrap();
        let uploader = Uploader::new(path.clone());
        assert!(uploader.save().is_err());
        fs::remove_file(&path).unwrap();
        uploader.save().unwrap();
        uploader.start(SERVER);
        assert!(uploads::load(&path).unwrap().started(SERVER).is_some());
    }

    #[test]
    fn a_rejected_token_is_tried_again_once_it_works_or_is_saved_again() {
        let dir = tempfile::tempdir().unwrap();
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let mut run = Run {
            rejected: Some((SERVER.into(), "t".into(), true)),
            ..Run::default()
        };
        assert_eq!(run.rejection(&uploader, SERVER, "t"), Some(true));
        assert_eq!(run.rejection(&uploader, SERVER, "t"), Some(true), "still");
        // "Check again" found it works, or the host saved it again.
        uploader.token_ok();
        assert_eq!(run.rejection(&uploader, SERVER, "t"), None);

        // Another token, or another server, isn't the one turned down.
        run.rejected = Some((SERVER.into(), "t".into(), false));
        assert_eq!(run.rejection(&uploader, SERVER, "other"), None);
        assert_eq!(run.rejected, None);
    }

    #[test]
    fn a_change_of_settings_stops_the_uploads_left_in_a_poll() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let uploader = std::sync::Arc::new(Uploader::new(dir.path().join("uploads.json")));
        let mut run = Run::default();
        let generation = uploader.generation.load(Ordering::Relaxed);
        // The host switches servers while the first upload is on its way.
        let changed = uploader.clone();
        let stored = r#"{"result":"stored","matches":[]}"#;
        let server = server::test_server(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{stored}",
                stored.len()
            ),
            EXAMPLE,
            move || changed.changed(),
        );
        let first = due(&logs, EXAMPLE);
        let second = Due {
            name: "Log-2026-10-02-21-00-00.txt".into(),
            path: logs.join("Log-2026-10-02-21-00-00.txt"),
        };
        fs::write(&second.path, EXAMPLE).unwrap();
        let (done, problem) = block_on(run.send_due(
            &uploader,
            generation,
            &server,
            "t",
            &[first.clone(), second.clone()],
            TIMEOUT,
        ));
        // The first went through; the second wasn't sent (not even tried: nothing failed).
        assert_eq!((done, problem), (1, None));
        assert_eq!(run.retrying, None);
        let record = uploader.record.lock().unwrap();
        assert!(record.get(&server, &first.name).is_some());
        assert!(record.get(&server, &second.name).is_none());
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
            || {},
        );
        let start = Instant::now();
        let sent = block_on(run.send(&uploader, &server, "t", &due(dir.path(), EXAMPLE), TIMEOUT));
        assert_eq!(sent, Ok(false));
        // Two hours, past `RETRY_MAX_SECS`, for every file to that server; not another server.
        assert!(run.held(
            &server,
            start + Duration::from_secs(config::RETRY_MAX_SECS.default + 60)
        ));
        assert!(!run.held(&server, Instant::now() + Duration::from_secs(7200)));
        assert!(!run.held("https://test.genjiball.us", start));
    }
}
