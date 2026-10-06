//! The upload loop: polls the log folder (`watcher.rs`), sends what's due to the server one file
//! at a time, keeps `uploads.json` up to date and tells the window. Runs for as long as the app
//! does, window open or not.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::afk::{AfkStatus, AfkStore};
use crate::debug::{Attempt, Outcome, Recent};
use crate::history::{self, Page, QueueSnapshot};
use crate::log_scan::{RoundStart, Scan};
use crate::server::{self, Host, TokenCheck, TourneyLobby, UploadInfo, UploadOutcome};
use crate::tourneys::Tourneys;
use crate::uploads::{self, Answer, Record, Sent, SentTourney};
use crate::watcher::{self, Due, Queued, Timing, Tracker};
use crate::{config, log_folder, log_scan, tourney, Store};

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
    /// The host has no home region on the server and didn't pick one: the server won't store a
    /// match without one. Uploads wait until the host picks a region (or an admin sets theirs).
    NoRegion,
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
    /// The region the host picked (`Settings::region`) when the poll began, `None` for their home
    /// region. The window drops a status for another, like one for another server.
    pub chosen_region: Option<String>,
    /// The region uploads go as: the one picked, else the host's home region. `None` while the
    /// home region isn't known (the server can't be reached) or the host has none.
    pub region: Option<String>,
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
    /// Whether the host is AFK, and the rounds noted for it lately.
    pub afk: AfkStatus,
    /// A dry run: logs are picked as usual but not sent (the debug panel lists them).
    pub dry_run: bool,
}

pub struct Uploader {
    record_path: PathBuf,
    record: Mutex<Record>,
    /// Why `uploads.json` couldn't be read, while it can't. It holds when uploads started, so
    /// it isn't written over and nothing is uploaded until it's fixed or deleted.
    record_error: Mutex<Option<String>>,
    /// Host AFK and its rounds (`afk.json`, next to `uploads.json`).
    afk: Mutex<AfkStore>,
    status: Mutex<UploadStatus>,
    /// The ranked files waiting to be uploaded, as of the last poll.
    queue: Mutex<QueueSnapshot>,
    /// Failed uploads the host asked to retry now, by file name.
    retries: Mutex<Vec<String>>,
    /// The latest uploads and dry runs, for the debug panel.
    recent: Mutex<Recent>,
    wake: Notify,
    /// Ask the server for the host and the listed matches' status at the next poll.
    refresh: AtomicBool,
    /// A token was saved, or a check found the token works: forget a rejection at the next poll.
    token_ok: AtomicBool,
    /// Goes up each time the server, the log folder, the region or the token changes (`changed`):
    /// a poll that began before stops sending.
    generation: AtomicU64,
    /// The tourney lobbies the host was assigned to, as the server last listed them (memory only).
    lobbies: Mutex<LobbyKeys>,
    /// A tourney match's end was uploaded: the tourney loop asks the server again (its lobby has
    /// a match now, and wants its verify screenshot).
    tourney_ended: AtomicBool,
}

/// What the tool knows of the host's tourney lobbies, per server, by lobby key: the key is only
/// in the server's answer while the lobby's code window is open, so it's kept once seen.
#[derive(Debug, Default)]
struct LobbyKeys {
    /// Server URL → lobby key → (lobby id, region).
    servers: HashMap<String, HashMap<String, (i64, String)>>,
    /// When a server was last asked for them, by an upload that needed a lobby's region.
    looked_up: HashMap<String, Instant>,
}

impl Uploader {
    /// Loads the record of past uploads from `record_path`. A missing one is a first run; one that
    /// can't be read is reported (`save`) until it can.
    pub fn new(record_path: PathBuf) -> Self {
        let (record, record_error) = match uploads::load(&record_path) {
            Ok(record) => (record, None),
            Err(e) => (Record::default(), Some(e)),
        };
        let afk = AfkStore::open(record_path.with_file_name(config::AFK_FILE));
        Self {
            record_path,
            record: Mutex::new(record),
            record_error: Mutex::new(record_error),
            afk: Mutex::new(afk),
            status: Mutex::new(UploadStatus::default()),
            queue: Mutex::new(QueueSnapshot::default()),
            retries: Mutex::new(Vec::new()),
            recent: Mutex::new(Recent::default()),
            wake: Notify::new(),
            refresh: AtomicBool::new(false),
            token_ok: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            lobbies: Mutex::new(LobbyKeys::default()),
            tourney_ended: AtomicBool::new(false),
        }
    }

    /// Notes the lobby keys and regions of the host's tourney lobbies on `server_url`, as the
    /// server listed them (`GET /api/host/tourneys`).
    pub fn learn_lobbies(&self, server_url: &str, lobbies: &[TourneyLobby]) {
        let mut known = self.lobbies.lock().unwrap();
        let keys = known.servers.entry(server_url.to_string()).or_default();
        for lobby in lobbies {
            if let Some(code) = &lobby.code {
                keys.insert(code.lobby_key.clone(), (lobby.id, lobby.region.clone()));
            }
        }
    }

    /// The region of the lobby with `lobby_key` on `server_url`, if the tool knows it.
    pub fn lobby_region(&self, server_url: &str, lobby_key: &str) -> Option<String> {
        let known = self.lobbies.lock().unwrap();
        Some(known.servers.get(server_url)?.get(lobby_key)?.1.clone())
    }

    /// The key noted for lobby `lobby` on `server_url`, if the tool saw its code values.
    pub fn lobby_key(&self, server_url: &str, lobby: i64) -> Option<String> {
        let known = self.lobbies.lock().unwrap();
        known
            .servers
            .get(server_url)?
            .iter()
            .find(|(_, (id, _))| *id == lobby)
            .map(|(key, _)| key.clone())
    }

    /// Whether this tool uploaded the end (`MATCH_END ROUNDS`) of lobby `lobby`'s match to
    /// `server_url`: by its key now (`lobby_key`, while the code window is open), else by the key
    /// noted earlier.
    pub fn tourney_ended(&self, server_url: &str, lobby: i64, lobby_key: Option<&str>) -> bool {
        let noted: Vec<String> = {
            let known = self.lobbies.lock().unwrap();
            known
                .servers
                .get(server_url)
                .into_iter()
                .flatten()
                .filter(|(_, (id, _))| *id == lobby)
                .map(|(key, _)| key.clone())
                .collect()
        };
        let record = self.record.lock().unwrap();
        lobby_key
            .into_iter()
            .chain(noted.iter().map(String::as_str))
            .any(|key| record.tourney_ended(server_url, key))
    }

    /// Whether an upload may ask `server_url` for the host's lobbies now (at most every
    /// `TOURNEY_LOOKUP_MIN_SECS`), noting that it does.
    fn may_look_up(&self, server_url: &str, now: Instant) -> bool {
        let mut known = self.lobbies.lock().unwrap();
        let every = Duration::from_secs(config::TOURNEY_LOOKUP_MIN_SECS);
        match known.looked_up.get(server_url) {
            Some(at) if now.saturating_duration_since(*at) < every => false,
            _ => {
                known.looked_up.insert(server_url.to_string(), now);
                true
            }
        }
    }

    /// Whether a tourney match's end was uploaded since this was last asked.
    pub fn take_tourney_ended(&self) -> bool {
        self.tourney_ended.swap(false, Ordering::Relaxed)
    }

    /// The server, the log folder, the region or the token changed: what a poll under way still has
    /// to send waits for the next one, which starts now with the new settings.
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

    /// The last poll's status, with AFK as it is now (before the first poll ends, say).
    pub fn status(&self) -> UploadStatus {
        UploadStatus {
            afk: self.afk(),
            ..self.status.lock().unwrap().clone()
        }
    }

    pub fn afk(&self) -> AfkStatus {
        self.afk.lock().unwrap().status()
    }

    /// The host turned AFK on or off, with `started` the rounds in the live log now (`afk.rs`).
    pub fn set_afk(&self, on: bool, started: &[RoundStart]) -> AfkStatus {
        let status = self.afk.lock().unwrap().set(on, started, SystemTime::now());
        // The window hears it from the next status too.
        self.wake();
        status
    }

    /// Notes the AFK rounds among `started`, the rounds read from the live log.
    fn note_afk(&self, started: &[RoundStart]) {
        self.afk.lock().unwrap().saw(
            started,
            SystemTime::now(),
            Duration::from_secs(config::AFK_KEEP_SECS),
        );
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

    /// The ranked files waiting to be uploaded to `server_url` from `folder`, as of the last poll.
    pub fn queue(&self, server_url: &str, folder: Option<&Path>) -> Vec<Queued> {
        self.queue
            .lock()
            .unwrap()
            .queue(server_url, folder)
            .to_vec()
    }

    /// The latest uploads and dry runs, newest first.
    pub fn recent(&self) -> Vec<Attempt> {
        self.recent.lock().unwrap().list()
    }

    /// See `UploadStatus::history_revision`. Both parts only go up.
    fn history_revision(&self) -> u64 {
        let queue = self.queue.lock().unwrap().revision();
        queue + self.record.lock().unwrap().revision()
    }

    /// Every match uploaded to `server_url`, newest first (`Record::known_matches`).
    pub fn known_matches(&self, server_url: &str) -> Vec<uploads::KnownMatch> {
        self.record.lock().unwrap().known_matches(server_url)
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

/// Where a poll's uploads go, and as which region (the host's home region for `None`).
#[derive(Clone, Copy)]
struct Target<'a> {
    server_url: &'a str,
    token: &'a str,
    region: Option<&'a str>,
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
                log_problem(current.problem.as_ref(), status.problem.as_ref());
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
    /// The server and token an upload without a region was refused for (`no_region`): uploads
    /// without one wait, even while the server can't say the host's home region.
    no_region: Option<(String, String)>,
    /// What a dry run went through, per server and file, as if it had been uploaded: it isn't
    /// picked again until it grows. Forgotten when the dry run ends, so those files go then.
    dry_sent: HashMap<String, HashMap<String, watcher::Sent>>,
}

/// A file as an upload sends it, besides its bytes: what's read from them.
struct Prepared {
    scan: Scan,
    host_afk: Option<String>,
    started_at: Option<String>,
}

impl Prepared {
    fn info<'a>(&'a self, file_name: &'a str, region: Option<&'a str>) -> UploadInfo<'a> {
        UploadInfo {
            file_name,
            started_at: self.started_at.as_deref(),
            region,
            host_afk: self.host_afk.as_deref(),
        }
    }
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

    /// Whether uploads to `server_url` with `token` need a region the host hasn't picked: the host
    /// has no home region, as far as the server last said.
    fn needs_region(&self, server_url: &str, token: &str, host: Option<&Host>) -> bool {
        match host {
            Some(host) => host.region.is_none(),
            None => {
                matches!(&self.no_region, Some((url, t)) if url == server_url && t == token)
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
        let chosen_region = settings.region.clone();
        let mut status = UploadStatus {
            server_url: server_url.clone(),
            log_folder: log_folder::current(settings.log_folder.as_deref()).map(|f| f.path),
            chosen_region: chosen_region.clone(),
            region: chosen_region.clone(),
            retrying: self.retrying.clone(),
            ..UploadStatus::default()
        };
        let region_of = |host: Option<&Host>| {
            chosen_region
                .clone()
                .or_else(|| host.and_then(|h| h.region.clone()))
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
            status.afk = uploader.afk();
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
        let dry_run = settings.dry_run_on();
        if !dry_run {
            self.dry_sent.clear();
        }
        // The retries the host asked for, before the poll picks what's due.
        for file in std::mem::take(&mut *uploader.retries.lock().unwrap()) {
            self.tracker.retry(&file);
        }
        let poll = {
            let record = uploader.record.lock().unwrap();
            // Before the first token for this server, count only what's written from now on.
            let since = record.started(&server_url).unwrap_or_else(SystemTime::now);
            let dry_sent = self.dry_sent.get(&server_url);
            self.tracker.poll(
                &folder.path,
                Instant::now(),
                SystemTime::now(),
                since,
                |name| {
                    let dry = dry_sent.and_then(|files| files.get(name)).copied();
                    dry.or_else(|| {
                        record.get(&server_url, name).map(|s| watcher::Sent {
                            size: s.size,
                            match_ends: s.match_ends,
                        })
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
        if !poll.due.is_empty() {
            let names: Vec<_> = poll.due.iter().map(|d| d.name.as_str()).collect();
            log::info!("Logs to upload: {}", names.join(", "));
        }
        // Before any upload: a file that grew with a round started while AFK sends it.
        uploader.note_afk(self.tracker.live_rounds());

        // Nothing goes to the server, so no token is needed. The region is the one picked, else
        // the home region if it's known (else an upload would leave `X-Region` out).
        if dry_run {
            let host = match &token {
                Ok(Some(token)) => self.known_host(&server_url, token),
                _ => None,
            };
            status.dry_run = true;
            status.region = region_of(host.as_ref());
            status.host = host;
            let done = self.dry_run(&uploader, &server_url, status.region.as_deref(), &poll.due);
            status.waiting = status.waiting.saturating_sub(done);
            return finish(self, polled, status);
        }

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
        // Uploads go as the home region: learn it first, so the window shows it before the first
        // upload, and a host without one isn't sent a match. While they have none, keep asking
        // (every `STATUS_REFRESH_SECS`, or on "Check again"): an admin may set it.
        let unsure = status.host.is_none() && !poll.due.is_empty();
        if chosen_region.is_none()
            && (unsure || self.needs_region(&server_url, &token, status.host.as_ref()))
        {
            let refreshed = self
                .refresh(
                    &uploader,
                    generation,
                    &server_url,
                    &token,
                    refresh_every,
                    timeout,
                )
                .await;
            status.host = self.known_host(&server_url, &token);
            if let Err(problem) = refreshed {
                status.problem = Some(problem);
                return finish(self, polled, status);
            }
        }
        let region = region_of(status.host.as_ref());
        status.region = region.clone();
        if region.is_none() && self.needs_region(&server_url, &token, status.host.as_ref()) {
            status.problem = Some(Problem::NoRegion);
            return finish(self, polled, status);
        }

        let target = Target {
            server_url: &server_url,
            token: &token,
            region: region.as_deref(),
        };
        let (done, problem) = self
            .send_due(&uploader, generation, target, &poll.due, timeout)
            .await;
        status.waiting = status.waiting.saturating_sub(done);
        status.problem = problem;
        status.retrying = self.retrying.clone();
        // Its lobby has a match now: the tourney list says so, and asks for the screenshot.
        if uploader.take_tourney_ended() {
            app.state::<Tourneys>().refresh();
        }
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
        status.region = region_of(status.host.as_ref());
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

        let check = server::check_token(server_url, token, timeout).await;
        log::debug!("Token check for {server_url}: {}", check.summary());
        match check {
            TokenCheck::Ok { host } => self.host = Some(host),
            check @ (TokenCheck::Unknown | TokenCheck::Revoked) => {
                let revoked = check == TokenCheck::Revoked;
                log::warn!("{server_url} turned the token down: {}", check.summary());
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
                Err(message) => log::warn!("Couldn't refresh the match status: {message}"),
            }
        }
        // A save that fails is tried again at the next poll, before any upload.
        uploader
            .save()
            .map_err(|message| Problem::Local { message })
    }

    /// `due` as an upload sends it: only its complete lines (the game may be halfway through
    /// writing the next one), and what's read from them. An error when it can't be read.
    fn prepare(&self, uploader: &Uploader, due: &Due) -> Result<(Vec<u8>, Prepared), String> {
        let mut bytes =
            fs::read(&due.path).map_err(|e| format!("Couldn't read {}: {e}", due.name))?;
        bytes.truncate(log_scan::complete_lines(&bytes).len());
        let scan = log_scan::scan(&String::from_utf8_lossy(&bytes));
        // The live log may have grown since the poll read it: a round that started meanwhile is
        // in what's sent, so it must be in `X-Host-Afk` too (the file may not grow again).
        if self.tracker.live() == Some(due.name.as_str()) {
            uploader.note_afk(&scan.round_starts);
        }
        let host_afk = uploader.afk.lock().unwrap().header(&scan.match_keys);
        let prepared = Prepared {
            scan,
            host_afk,
            started_at: started_at(due),
        };
        Ok((bytes, prepared))
    }

    /// A dry run of the files `due` to `server_url` as `region`: what each upload would send goes
    /// to the debug panel, and nothing is sent or put in `uploads.json`. How many are done with;
    /// one that can't be read is tried at the next poll.
    fn dry_run(
        &mut self,
        uploader: &Uploader,
        server_url: &str,
        region: Option<&str>,
        due: &[Due],
    ) -> usize {
        let mut done = 0;
        for due in due {
            let (bytes, prepared) = match self.prepare(uploader, due) {
                Ok(prepared) => prepared,
                Err(message) => {
                    log::warn!("Dry run: {message}");
                    continue;
                }
            };
            let size = bytes.len() as u64;
            log::info!(
                "Dry run: would upload {} ({size} bytes) to {server_url}",
                due.name
            );
            let outcome = if size > config::MAX_UPLOAD_BYTES {
                Outcome::TooLarge
            } else {
                Outcome::DryRun
            };
            // A tourney log as its lobby's region, if it's known (a dry run asks the server nothing).
            let region = tourney::upload_region(
                &prepared.scan,
                |k| uploader.lobby_region(server_url, k),
                region,
            );
            let info = prepared.info(&due.name, region.as_deref());
            let attempt = Attempt::new(server_url, &info, size, &prepared.scan.match_keys, outcome);
            uploader.recent.lock().unwrap().push(attempt);
            self.tracker.sent(&due.name);
            let sent = watcher::Sent {
                size,
                match_ends: prepared.scan.match_ends,
            };
            self.dry_sent
                .entry(server_url.to_string())
                .or_default()
                .insert(due.name.clone(), sent);
            done += 1;
        }
        done
    }

    /// Uploads the files `due`, oldest first, until one stops every upload, the server says to
    /// hold off (`429`), or the settings or token changed since `generation` (the rest waits for
    /// the next poll, with the new ones). How many are done with, and the problem if one came up.
    async fn send_due(
        &mut self,
        uploader: &Uploader,
        generation: u64,
        target: Target<'_>,
        due: &[Due],
        timeout: Duration,
    ) -> (usize, Option<Problem>) {
        let mut done = 0;
        for due in due {
            if !uploader.is_current(generation) || self.held(target.server_url, Instant::now()) {
                break;
            }
            match self.send(uploader, target, due, timeout).await {
                Ok(true) => done += 1,
                Ok(false) => {}
                Err(problem) => return (done, Some(problem)),
            }
        }
        (done, None)
    }

    /// The region the file `name` (read as `scan`) is uploaded as (`tourney::upload_region`): a log
    /// of tourney matches goes as their lobby's region, asking the server for the host's lobbies
    /// first if one isn't known yet; anything else as `target`'s.
    async fn file_region(
        &self,
        uploader: &Uploader,
        target: Target<'_>,
        name: &str,
        scan: &Scan,
        timeout: Duration,
    ) -> Option<String> {
        let server_url = target.server_url;
        let keys = scan.lobby_keys();
        if keys.is_empty() {
            return target.region.map(str::to_string);
        }
        let unknown = keys
            .iter()
            .any(|k| uploader.lobby_region(server_url, k).is_none());
        if unknown && uploader.may_look_up(server_url, Instant::now()) {
            match server::host_tourneys(server_url, target.token, timeout).await {
                Ok(list) => uploader.learn_lobbies(server_url, &list.lobbies),
                Err(e) => log::warn!("Couldn't ask {server_url} for your tourney lobbies: {e:?}"),
            }
        }
        let region = tourney::upload_region(
            scan,
            |k| uploader.lobby_region(server_url, k),
            target.region,
        );
        for key in &keys {
            match uploader.lobby_region(server_url, key) {
                Some(lobby) if Some(lobby.as_str()) == region.as_deref() => {
                    if region.as_deref() != target.region {
                        log::info!(
                            "{name} is a tourney match of lobby {key}: uploading it as its region, {lobby}"
                        );
                    }
                }
                Some(lobby) => log::warn!(
                    "{name} has a match of tourney lobby {key} ({lobby}), but goes as {}: the server will send it to review",
                    region.as_deref().unwrap_or("the home region")
                ),
                None => log::warn!(
                    "{name} has a match of tourney lobby {key}, which isn't one of your assigned lobbies on {server_url}"
                ),
            }
        }
        region
    }

    /// Uploads one file and records what came of it: `true` when it's done with, `false` when it
    /// waits to be retried. A problem that stops every upload is an error.
    async fn send(
        &mut self,
        uploader: &Uploader,
        target: Target<'_>,
        due: &Due,
        timeout: Duration,
    ) -> Result<bool, Problem> {
        let Target {
            server_url, token, ..
        } = target;
        let (bytes, prepared) = match self.prepare(uploader, due) {
            Ok(prepared) => prepared,
            Err(message) => {
                log::warn!("{message}. Trying again later");
                self.tracker
                    .failed(&due.name, Instant::now(), None, message.clone());
                self.retrying = Some(message);
                return Ok(false);
            }
        };
        let size = bytes.len() as u64;
        let scan = &prepared.scan;
        let region = self
            .file_region(uploader, target, &due.name, scan, timeout)
            .await;
        let region = region.as_deref();
        let info = prepared.info(&due.name, region);
        let note = |outcome| {
            let attempt = Attempt::new(server_url, &info, size, &scan.match_keys, outcome);
            uploader.recent.lock().unwrap().push(attempt);
        };
        let answer = if size > config::MAX_UPLOAD_BYTES {
            note(Outcome::TooLarge);
            Answer::Refused {
                error: "too_large".into(),
                message: format!(
                    "Over the server's {} KB limit",
                    config::MAX_UPLOAD_BYTES / 1024
                ),
            }
        } else {
            log::info!(
                "Uploading {} ({size} bytes) to {server_url} as {}",
                due.name,
                region.unwrap_or("the home region")
            );
            let uploaded = server::upload(server_url, token, info, bytes, timeout).await;
            note(Outcome::of(&uploaded));
            match uploaded.outcome {
                UploadOutcome::Stored(answer) => {
                    // The answer has no match id on the site: the status refresh brings it, so
                    // ask for one at the end of this poll rather than in `STATUS_REFRESH_SECS`.
                    if !answer.matches.is_empty() {
                        uploader.refresh.store(true, Ordering::Relaxed);
                    }
                    Answer::Answered(answer)
                }
                UploadOutcome::Refused { error, message } => Answer::Refused { error, message },
                UploadOutcome::NoRegion => {
                    log::warn!("{server_url} won't take {}: no region", due.name);
                    // Not recorded: the file goes once there's a region.
                    self.no_region = Some((server_url.to_string(), token.to_string()));
                    return Err(Problem::NoRegion);
                }
                UploadOutcome::TokenRejected { revoked } => {
                    log::warn!(
                        "{server_url} turned the token down uploading {} (revoked: {revoked})",
                        due.name
                    );
                    self.rejected = Some((server_url.to_string(), token.to_string(), revoked));
                    return Err(Problem::TokenRejected { revoked });
                }
                UploadOutcome::Retry { message, after } => {
                    log::warn!(
                        "Upload of {} failed, trying again {}: {message}",
                        due.name,
                        match after {
                            Some(wait) => format!("in {}s, as the server asked", wait.as_secs()),
                            None => "after a backoff".into(),
                        }
                    );
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
                    log::warn!(
                        "{server_url} asked to slow down: no uploads for {}s ({message})",
                        wait.as_secs()
                    );
                    self.held = Some((server_url.to_string(), timing.later(now, wait)));
                    self.tracker
                        .failed(&due.name, now, Some(wait), message.clone());
                    self.retrying = Some(message);
                    return Ok(false);
                }
            }
        };
        log::info!("{}: {}", due.name, describe(&answer));
        self.tracker.sent(&due.name);
        self.retrying = None;
        let tourneys = SentTourney::of(scan);
        if matches!(answer, Answer::Answered(_)) && tourneys.iter().any(|t| t.ended) {
            uploader.tourney_ended.store(true, Ordering::Relaxed);
        }
        uploader.record.lock().unwrap().put(
            server_url,
            &due.name,
            Sent {
                size,
                match_ends: scan.match_ends,
                at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                players: scan.players.clone(),
                answer,
                tourneys,
            },
        );
        uploader
            .save()
            .map(|()| true)
            .map_err(|message| Problem::Local { message })
    }
}

/// Logs uploads stopping, starting again or stopping for another reason.
fn log_problem(before: Option<&Problem>, now: Option<&Problem>) {
    if before == now {
        return;
    }
    match now {
        Some(problem) => log::warn!("Uploads wait: {problem:?}"),
        None => log::info!("Uploads go on"),
    }
}

/// The server's answer to an upload, for the log: its result and each match's status.
fn describe(answer: &Answer) -> String {
    match answer {
        Answer::Answered(answer) => {
            let matches: Vec<_> = answer
                .matches
                .iter()
                .map(|m| {
                    format!(
                        "{} {} {}",
                        m.match_key.as_deref().unwrap_or("?"),
                        m.action,
                        m.status
                    )
                })
                .collect();
            format!(
                "{} as {}, matches: [{}]",
                answer.result,
                answer.region.as_deref().unwrap_or("?"),
                matches.join(", ")
            )
        }
        Answer::Refused { error, message } => format!("refused, {error}: {message}"),
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

    /// Uploads to `server` with the token `t`, as the home region.
    fn target(server: &str) -> Target<'_> {
        Target {
            server_url: server,
            token: "t",
            region: None,
        }
    }

    fn host(region: Option<&str>) -> Host {
        Host {
            id: 3,
            name: "Kenzo".into(),
            trust: "trusted".into(),
            region: region.map(str::to_string),
        }
    }

    #[test]
    fn a_host_without_a_home_region_must_pick_one() {
        let run = Run::default();
        assert!(run.needs_region(SERVER, "t", Some(&host(None))));
        assert!(!run.needs_region(SERVER, "t", Some(&host(Some("eu")))));
        // Not known yet: the server stores the upload as the home region, or says there's none.
        assert!(!run.needs_region(SERVER, "t", None));
    }

    #[test]
    fn an_upload_without_a_region_waits_for_one_and_isnt_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let mut run = Run::default();
        let refused = r#"{"error":"no_region","message":"This host has no home region"}"#;
        let server = server::test_server(
            format!(
                "HTTP/1.1 422 Unprocessable Content\r\nContent-Length: {}\r\n\r\n{refused}",
                refused.len()
            ),
            EXAMPLE,
            || {},
        );
        let file = due(dir.path(), EXAMPLE);
        let sent = block_on(run.send(&uploader, target(&server), &file, TIMEOUT));
        assert_eq!(sent, Err(Problem::NoRegion));
        assert!(uploader
            .record
            .lock()
            .unwrap()
            .get(&server, &file.name)
            .is_none());
        // Even while the server can't say the home region; not for another token.
        assert!(run.needs_region(&server, "t", None));
        assert!(!run.needs_region(&server, "other", None));
    }

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
            target(&server),
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

    /// The request an upload of `text` (in `file`, rewritten first) sends, as `run` and `uploader`.
    fn sent_request(run: &mut Run, uploader: &Uploader, file: &Due, text: &str) -> String {
        fs::write(&file.path, text).unwrap();
        let (seen, request) = std::sync::mpsc::channel();
        let stored = r#"{"result":"stored","matches":[]}"#;
        let server = server::test_server_seeing(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{stored}",
                stored.len()
            ),
            text,
            seen,
        );
        let sent = block_on(run.send(uploader, target(&server), file, TIMEOUT));
        assert_eq!(sent, Ok(true));
        request.recv().unwrap().to_ascii_lowercase()
    }

    /// The example log up to the line that starts `round`, without it.
    fn before_round(round: usize) -> String {
        let (at, _) = EXAMPLE
            .match_indices("ROUND_START|")
            .nth(round - 1)
            .unwrap();
        EXAMPLE[..EXAMPLE[..at].rfind('\n').unwrap() + 1].to_string()
    }

    #[test]
    fn sends_the_rounds_that_started_while_the_host_was_afk() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        fs::create_dir(&logs).unwrap();
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let mut run = Run::default();
        let file = due(&logs, &before_round(2));
        let poll = |run: &mut Run| {
            run.tracker
                .poll(
                    &logs,
                    Instant::now(),
                    SystemTime::now(),
                    SystemTime::UNIX_EPOCH,
                    |_| None,
                )
                .unwrap();
            uploader.note_afk(run.tracker.live_rounds());
        };
        poll(&mut run);
        // Not AFK: no header.
        let request = sent_request(&mut run, &uploader, &file, &before_round(2));
        assert!(!request.contains("x-host-afk"), "{request}");

        // Round 2 had started when the host turned AFK on, but wasn't read yet.
        let started = log_scan::round_starts(&before_round(3));
        assert!(uploader.set_afk(true, &started).on);
        fs::write(&file.path, before_round(3)).unwrap();
        poll(&mut run);
        let request = sent_request(&mut run, &uploader, &file, &before_round(3));
        assert!(!request.contains("x-host-afk"), "{request}");

        // Round 3 starts after the poll read the file, just before the upload: it's sent too.
        let request = sent_request(&mut run, &uploader, &file, EXAMPLE);
        assert!(
            request.contains("\r\nx-host-afk: 482913507226:3\r\n"),
            "{request}"
        );
        // And kept across a restart, with AFK still on.
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let afk = uploader.afk();
        assert!(afk.on);
        assert_eq!(afk.latest.unwrap().rounds, [3]);
    }

    const TOURNEY_LOG: &str = include_str!("../tests/fixtures/ranked-log-tourney-example.txt");

    /// The host's lobby with `lobby_key`, in `region`, while its code window is open.
    fn tourney_lobby(lobby_key: &str, region: &str) -> TourneyLobby {
        TourneyLobby {
            id: 7,
            label: "Lobby 1/2".into(),
            region: region.into(),
            round_limit: 3,
            tourney: server::LobbyTourney {
                id: 3,
                name: "October Cup".into(),
                region: region.into(),
                starts_at: "2026-10-10T17:00:00Z".into(),
                status: "live".into(),
            },
            match_id: None,
            screenshot: None,
            screenshot_expired: false,
            verified: false,
            code_from: None,
            code: Some(server::TourneyCodeValues {
                lobby_key: lobby_key.into(),
                round_limit: 3,
                name: "October Cup".into(),
                label: "Lobby 1/2".into(),
            }),
        }
    }

    #[test]
    fn uploads_a_tourney_match_as_its_lobbys_region_and_notes_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let mut run = Run::default();
        let file = due(dir.path(), TOURNEY_LOG);
        let (seen, request) = std::sync::mpsc::channel();
        let stored = r#"{"result":"stored","matches":[]}"#;
        let server = server::test_server_seeing(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{stored}",
                stored.len()
            ),
            TOURNEY_LOG,
            seen,
        );
        uploader.learn_lobbies(&server, &[tourney_lobby("073518264903", "na")]);
        assert_eq!(
            uploader.lobby_key(&server, 7).as_deref(),
            Some("073518264903")
        );
        assert_eq!(uploader.lobby_key(&server, 8), None);
        // The host picked EU for the night; the lobby is NA's.
        let target = Target {
            region: Some("eu"),
            ..target(&server)
        };
        assert!(!uploader.tourney_ended(&server, 7, None));
        assert_eq!(
            block_on(run.send(&uploader, target, &file, TIMEOUT)),
            Ok(true)
        );
        let request = request.recv().unwrap().to_ascii_lowercase();
        assert!(request.contains("\r\nx-region: na\r\n"), "{request}");
        // Marked in the record, and the lobby wants its screenshot now, by its key or its id.
        let sent = uploader
            .record
            .lock()
            .unwrap()
            .get(&server, &file.name)
            .cloned();
        assert_eq!(
            sent.unwrap().tourneys,
            [SentTourney {
                lobby_key: "073518264903".into(),
                ended: true
            }]
        );
        assert!(uploader.take_tourney_ended());
        assert!(!uploader.take_tourney_ended());
        assert!(uploader.tourney_ended(&server, 7, None));
        assert!(uploader.tourney_ended(&server, 8, Some("073518264903")));
        assert!(!uploader.tourney_ended(&server, 8, None));
    }

    #[test]
    fn looks_up_the_lobbies_at_most_so_often() {
        let dir = tempfile::tempdir().unwrap();
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let now = Instant::now();
        assert!(uploader.may_look_up(SERVER, now));
        assert!(!uploader.may_look_up(SERVER, now + Duration::from_secs(1)));
        assert!(uploader.may_look_up("https://test.genjiball.us", now));
        let later = now + Duration::from_secs(config::TOURNEY_LOOKUP_MIN_SECS);
        assert!(uploader.may_look_up(SERVER, later));
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
        let sent = block_on(run.send(
            &uploader,
            target(&server),
            &due(dir.path(), EXAMPLE),
            TIMEOUT,
        ));
        assert_eq!(sent, Ok(false));
        // Two hours, past `RETRY_MAX_SECS`, for every file to that server; not another server.
        assert!(run.held(
            &server,
            start + Duration::from_secs(config::RETRY_MAX_SECS.default + 60)
        ));
        assert!(!run.held(&server, Instant::now() + Duration::from_secs(7200)));
        assert!(!run.held("https://test.genjiball.us", start));
    }

    #[test]
    fn a_dry_run_sends_nothing_and_lists_what_it_would_send() {
        let dir = tempfile::tempdir().unwrap();
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let mut run = Run::default();
        // Half a line at the end: left out, as an upload leaves it out.
        let file = due(dir.path(), &format!("{EXAMPLE}MATCH_END|"));
        let done = run.dry_run(&uploader, SERVER, Some("na"), std::slice::from_ref(&file));
        assert_eq!(done, 1);
        assert!(uploader
            .record
            .lock()
            .unwrap()
            .get(SERVER, &file.name)
            .is_none());

        let listed = uploader.recent();
        assert_eq!(listed.len(), 1);
        let attempt = &listed[0];
        assert_eq!(attempt.outcome, Outcome::DryRun);
        assert_eq!(attempt.bytes, EXAMPLE.len() as u64);
        assert_eq!(attempt.match_keys, ["482913507226"]);
        let header = |name| {
            attempt
                .headers
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(header("X-Log-File"), Some(file.name.as_str()));
        assert_eq!(header("X-Region"), Some("na"));
        assert!(header("X-Log-Started-At").is_some());

        // Gone through as if uploaded: picked again only once it grows.
        let sent = run.dry_sent[SERVER][&file.name];
        assert_eq!(sent.size, EXAMPLE.len() as u64);
        assert_eq!(sent.match_ends, 1);
    }

    #[test]
    fn an_upload_lists_the_servers_answer_without_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let mut run = Run::default();
        let stored = r#"{"result":"stored","matches":[]}"#;
        let server = server::test_server(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{stored}",
                stored.len()
            ),
            EXAMPLE,
            || {},
        );
        let file = due(dir.path(), EXAMPLE);
        let sent = block_on(run.send(&uploader, target(&server), &file, TIMEOUT));
        assert_eq!(sent, Ok(true));
        let attempt = &uploader.recent()[0];
        assert_eq!(
            attempt.outcome,
            Outcome::Answered {
                answer: server::RawAnswer {
                    status: 200,
                    body: stored.into()
                }
            }
        );
        assert!(attempt
            .headers
            .iter()
            .all(|(name, value)| !name.eq_ignore_ascii_case("authorization") && value != "t"));
    }

    #[test]
    fn a_failed_upload_lists_why() {
        let dir = tempfile::tempdir().unwrap();
        let uploader = Uploader::new(dir.path().join("uploads.json"));
        let mut run = Run::default();
        // Nothing listens there.
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        let file = due(dir.path(), EXAMPLE);
        let sent = block_on(run.send(&uploader, target(&closed), &file, TIMEOUT));
        assert_eq!(sent, Ok(false));
        assert!(
            matches!(&uploader.recent()[0].outcome, Outcome::Unanswered { message } if message.contains("Couldn't reach")),
            "{:?}",
            uploader.recent()
        );
    }
}
