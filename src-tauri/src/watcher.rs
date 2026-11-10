//! Which log files to upload now. Each poll reads the sizes in the log folder, reads a file again
//! only when it changed, and picks the ranked files that changed since they were last sent and
//! either have a new `MATCH_END` or stopped growing (`Timing::quiet`).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

use crate::config;
use crate::log_scan::{self, Scan};
use crate::settings::Settings;
use crate::uploads::SentTourney;

/// A file to upload.
#[derive(Debug, Clone, PartialEq)]
pub struct Due {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Default, PartialEq)]
pub struct Poll {
    pub due: Vec<Due>,
    /// Ranked files not sent as they are now: a match being played, or one waiting to be retried.
    pub waiting: usize,
}

/// The last upload of a file, as far as the watcher cares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sent {
    pub size: u64,
    pub match_ends: usize,
}

/// A ranked file not uploaded as it is now, for the upload history.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Queued {
    pub file: String,
    pub players: Vec<String>,
    pub state: QueueState,
    /// Its tourney matches, so far.
    pub tourneys: Vec<SentTourney>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QueueState {
    /// Still being written: uploaded once its match ends or it stops growing.
    Playing,
    /// Uploaded at the next chance (now, or once uploads aren't paused).
    Due,
    /// The last try failed (offline, server down). Tried again after the backoff, or on Retry.
    Failed { error: String },
}

struct Tracked {
    size: u64,
    modified: SystemTime,
    /// When the file last grew, as far as we know.
    changed_at: Instant,
    /// Bytes in its complete lines (`log_scan::complete_lines`): what an upload sends.
    complete: u64,
    scan: Scan,
    failures: u32,
    retry_at: Option<Instant>,
    /// Why the last try failed, until one works.
    error: Option<String>,
    /// Ranked and not uploaded as it is now, at the last poll.
    waiting: bool,
    /// Picked to upload at the last poll.
    due: bool,
}

/// How long the watcher waits, as the host set it (`config::QUIET_SECS`, `RETRY_FIRST_SECS`,
/// `RETRY_MAX_SECS`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    pub quiet: Duration,
    pub retry_first: Duration,
    pub retry_max: Duration,
}

impl Timing {
    pub fn new(settings: &Settings) -> Self {
        Self {
            quiet: settings.secs(&config::QUIET_SECS),
            retry_first: settings.secs(&config::RETRY_FIRST_SECS),
            retry_max: settings.secs(&config::RETRY_MAX_SECS),
        }
    }

    /// `retry_first`, doubled for each failure after the first, at most `retry_max`.
    pub fn backoff(&self, failures: u32) -> Duration {
        2u32.checked_pow(failures.saturating_sub(1))
            .and_then(|times| self.retry_first.checked_mul(times))
            .map_or(self.retry_max, |wait| wait.min(self.retry_max))
    }

    /// `wait` after `now`. A server's `Retry-After` too far off to count to waits `retry_max`.
    pub fn later(&self, now: Instant, wait: Duration) -> Instant {
        now.checked_add(wait).unwrap_or(now + self.retry_max)
    }
}

impl Default for Timing {
    fn default() -> Self {
        Self::new(&Settings::default())
    }
}

/// The live log (the newest file) as a poll or the host's click saw it (`Tracker::live_log`,
/// `read_live`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Live<'a> {
    /// A ranked log: what's in its complete lines, and how far into which file that is. Uploaded
    /// as it is or not: a lobby gone quiet is still the host's lobby, and what the file says
    /// (a sit-out going on, or over) is still the truth.
    Read(&'a Scan, LogAt<'a>),
    /// What's in it now isn't known: it couldn't be read and wasn't read before, nothing is
    /// written in it yet, or it's one the tool doesn't read (from before uploads started, or too
    /// large to upload).
    Unread,
    /// No ranked match is played in it: there's no log at all, or the newest one isn't a ranked
    /// one (another game, newer than any ranked log).
    Gone,
}

/// How far a read of the live log got: its folder, its file and the bytes of its complete lines.
/// A log only grows, and a newer one has a later name (`newest_log`), so a read of a later file,
/// or further into the same one, is the newer one (`LogAt::is_before`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogAt<'a> {
    pub folder: &'a Path,
    pub name: &'a str,
    pub len: u64,
}

impl LogAt<'_> {
    /// Whether this read is older than `other`: the same folder, and an older file or less of the
    /// same one. Reads of another folder (the setting changed) are never older.
    pub fn is_before(&self, other: &LogAt<'_>) -> bool {
        self.folder == other.folder && (self.name, self.len) < (other.name, other.len)
    }
}

/// The live log read right now, owned (`read_live`): what the host's AFK switch follows.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveNow {
    Read {
        scan: Scan,
        folder: PathBuf,
        name: String,
        len: u64,
    },
    Unread,
    Gone,
}

impl LiveNow {
    pub fn live(&self) -> Live<'_> {
        match self {
            LiveNow::Read {
                scan,
                folder,
                name,
                len,
            } => Live::Read(
                scan,
                LogAt {
                    folder,
                    name,
                    len: *len,
                },
            ),
            LiveNow::Unread => Live::Unread,
            LiveNow::Gone => Live::Gone,
        }
    }
}

/// The live log (the newest in `folder`) read now, for the host's AFK switch: whatever its size or
/// age (all of it, so the switch follows where the log is now), with a `ROUND_START` the game is
/// halfway through writing (`log_scan::live_scan`). As `Tracker::live_log` otherwise: no log or
/// a newest one that isn't ranked is `Gone`, one with nothing written in it yet `Unread`. An
/// error when the folder or the log can't be read (locked for a moment while the game writes it:
/// the host clicks again).
pub fn read_live(folder: &Path) -> Result<LiveNow, String> {
    let newest = newest_log(folder).map_err(|e| format!("Couldn't read the log folder: {e}"))?;
    let Some(path) = newest else {
        return Ok(LiveNow::Gone);
    };
    let bytes =
        fs::read(&path).map_err(|e| format!("Couldn't read the match log ({e}). Try again"))?;
    let len = log_scan::complete_lines(&bytes).len() as u64;
    let scan = log_scan::live_scan(&String::from_utf8_lossy(&bytes));
    Ok(if scan.ranked {
        LiveNow::Read {
            scan,
            folder: folder.to_path_buf(),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            len,
        }
    } else if len == 0 {
        LiveNow::Unread
    } else {
        LiveNow::Gone
    })
}

#[derive(Default)]
pub struct Tracker {
    folder: PathBuf,
    files: HashMap<String, Tracked>,
    /// The newest log file in the folder at the last poll (`newest_log`): the one being written.
    live: Option<String>,
    /// The newest log file the last poll couldn't look at or read, with nothing known of it from
    /// before (it may be locked for a moment while the game writes it).
    unread: Option<String>,
    /// Set from the settings before each poll.
    pub timing: Timing,
}

impl Tracker {
    /// Looks at `folder` at `now` (`clock` being the same moment as wall time). Files started
    /// before `since` (when uploads to this server started) are left alone, and that's what
    /// keeps a first run from sending a folder of old matches; a file after it waits however
    /// long it takes. `sent` gives the last upload of a file to the current server.
    pub fn poll(
        &mut self,
        folder: &Path,
        now: Instant,
        clock: SystemTime,
        since: SystemTime,
        sent: impl Fn(&str) -> Option<Sent>,
    ) -> io::Result<Poll> {
        if self.folder != folder {
            self.folder = folder.to_path_buf();
            self.files.clear();
        }
        let quiet = self.timing.quiet;
        let mut poll = Poll::default();
        let mut seen = Vec::new();
        self.live = None;
        // Files uploaded as they are: name, path, size, modified, age.
        let mut uploaded: Vec<(String, PathBuf, u64, SystemTime, Duration)> = Vec::new();
        let mut unread: Option<String> = None;
        let mut not_read = |name: &str| {
            if unread.as_deref().is_none_or(|u| name > u) {
                unread = Some(name.to_string());
            }
        };
        for entry in fs::read_dir(folder)? {
            let Ok(entry) = entry else { continue };
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if !log_scan::is_log_file(&name) {
                continue;
            }
            let path = entry.path();
            // From the file itself: a directory listing's sizes can lag behind a file being written.
            let Ok(meta) = fs::metadata(&path) else {
                not_read(&name);
                continue;
            };
            if meta.is_file() && self.live.as_ref().is_none_or(|live| name > *live) {
                self.live = Some(name.clone());
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let age = clock.duration_since(modified).unwrap_or_default();
            // When the file was started, not last written: a match that began before `since` and
            // is still being played stays out too.
            let started = log_scan::started_at_time(&name)
                .or_else(|| meta.created().ok())
                .unwrap_or(modified);
            if !meta.is_file() || started < since {
                continue;
            }
            let size = meta.len();
            let last = sent(&name);
            // Uploaded as it is: no need to read it (a folder of old matches is skipped this way).
            // Only the live log is, after this loop, as AFK follows what's in it.
            if last.is_some_and(|s| s.size == size) {
                uploaded.push((name, path, size, modified, age));
                continue;
            }
            // Refused once for its size: it can only get bigger, so don't read it again.
            if size > config::MAX_UPLOAD_BYTES
                && last.is_some_and(|s| s.size > config::MAX_UPLOAD_BYTES)
            {
                continue;
            }
            let mut tracked = match self.files.remove(&name) {
                Some(tracked) if tracked.size == size && tracked.modified == modified => tracked,
                mut before => match read_tracked(&path, size, modified, &mut before, now, age) {
                    Some(tracked) => tracked,
                    // A file still being written may be locked for a moment: try on the next poll,
                    // keeping what's known of it (a failed upload's backoff).
                    None => {
                        match before {
                            Some(mut t) => {
                                t.due = false;
                                poll.waiting += usize::from(t.waiting);
                                seen.push((name, t));
                            }
                            None => not_read(&name),
                        }
                        continue;
                    }
                },
            };
            tracked.waiting = tracked.scan.ranked && last.map(|s| s.size) != Some(tracked.complete);
            tracked.due = false;
            if !tracked.waiting {
                // Uploaded as it is: an earlier failure no longer matters.
                tracked.failures = 0;
                tracked.retry_at = None;
                tracked.error = None;
            } else {
                poll.waiting += 1;
                let ended = tracked.scan.match_ends > last.map_or(0, |s| s.match_ends);
                let stopped = now.saturating_duration_since(tracked.changed_at) >= quiet;
                let held = tracked.retry_at.is_some_and(|at| now < at);
                if (ended || stopped) && !held {
                    tracked.due = true;
                    poll.due.push(Due {
                        name: name.clone(),
                        path,
                    });
                }
            }
            seen.push((name, tracked));
        }
        let mut files: HashMap<String, Tracked> = seen.into_iter().collect();
        // The live log uploaded as it is (its match ended, or the lobby went quiet): what's in it
        // still says whether the host sits out (`live_log`). Read once, then kept while it's the
        // same; not one too large to upload (`live_log` doesn't follow it).
        let live = self.live.as_deref();
        if let Some((name, path, size, modified, age)) = uploaded
            .into_iter()
            .find(|u| Some(u.0.as_str()) == live && u.2 <= config::MAX_UPLOAD_BYTES)
        {
            let before = self.files.remove(&name);
            let tracked = match before {
                Some(t) if t.size == size && t.modified == modified => Some(t),
                _ => read_tracked(&path, size, modified, &mut None, now, age),
            };
            match tracked {
                Some(mut t) => {
                    t.waiting = false;
                    t.due = false;
                    t.failures = 0;
                    t.retry_at = None;
                    t.error = None;
                    files.insert(name, t);
                }
                None => not_read(&name),
            }
        }
        self.files = files;
        self.unread = unread;
        // Oldest first, so matches reach the server in the order they were played.
        poll.due.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(poll)
    }

    /// The newest log file at the last poll: the one the game is writing, if any is.
    pub fn live(&self) -> Option<&str> {
        self.live.as_deref()
    }

    /// The folder of the last poll.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// What the last poll knows of the live log (`Live`). Only what shows no ranked match is
    /// played in it any more is `Gone`: no log at all, or a newest one that isn't ranked. A log
    /// the tool doesn't read (too large, from before uploads started) or couldn't read is
    /// `Unread`, so AFK doesn't change for it.
    pub fn live_log(&self) -> Live<'_> {
        // The newest file, or one newer than it, may only be locked for a moment.
        if self
            .unread
            .as_ref()
            .is_some_and(|u| self.live.as_ref().is_none_or(|live| u >= live))
        {
            return Live::Unread;
        }
        let Some(name) = self.live.as_deref() else {
            return Live::Gone;
        };
        match self.files.get(name) {
            // Left alone: from before uploads started, or refused for its size.
            None => Live::Unread,
            // Only read up to the limit: not what's in it now.
            Some(t) if t.size > config::MAX_UPLOAD_BYTES => Live::Unread,
            // Read now, or at an earlier poll and unchanged since (or couldn't be read again).
            Some(t) if t.scan.ranked => Live::Read(
                &t.scan,
                LogAt {
                    folder: &self.folder,
                    name,
                    len: t.complete,
                },
            ),
            // A new file the game hasn't written a whole line in yet (a spectator copy starting).
            Some(t) if t.complete == 0 => Live::Unread,
            Some(_) => Live::Gone,
        }
    }

    /// The upload of `name` worked, or won't work however often it's tried. Either way it's in the
    /// upload record now, so it's no longer waiting.
    pub fn sent(&mut self, name: &str) {
        if let Some(t) = self.files.get_mut(name) {
            t.failures = 0;
            t.retry_at = None;
            t.error = None;
            t.waiting = false;
            t.due = false;
        }
    }

    /// The upload of `name` failed with `error`: hold it for `after`, or a backoff that grows with
    /// each failure.
    pub fn failed(&mut self, name: &str, now: Instant, after: Option<Duration>, error: String) {
        let timing = self.timing;
        if let Some(t) = self.files.get_mut(name) {
            t.failures += 1;
            let wait = after.unwrap_or_else(|| timing.backoff(t.failures));
            t.retry_at = Some(timing.later(now, wait));
            t.error = Some(error);
        }
    }

    /// The host asked to retry `name` now: drops the backoff, so the next poll picks it like any
    /// other file (only if it still isn't uploaded as it is). `false` when it isn't a failed upload.
    pub fn retry(&mut self, name: &str) -> bool {
        match self.files.get_mut(name) {
            Some(t) if t.waiting && t.error.is_some() => {
                t.failures = 0;
                t.retry_at = None;
                true
            }
            _ => false,
        }
    }

    /// The ranked files not uploaded as they are now, as of the last poll and the uploads since.
    pub fn queue(&self) -> Vec<Queued> {
        let mut queue: Vec<Queued> = self
            .files
            .iter()
            .filter(|(_, t)| t.waiting)
            .map(|(name, t)| Queued {
                file: name.clone(),
                players: t.scan.players.clone(),
                tourneys: SentTourney::of(&t.scan),
                state: match (&t.error, t.due) {
                    (Some(error), _) => QueueState::Failed {
                        error: error.clone(),
                    },
                    (None, true) => QueueState::Due,
                    (None, false) => QueueState::Playing,
                },
            })
            .collect();
        queue.sort_by(|a, b| a.file.cmp(&b.file));
        queue
    }
}

/// The file at `path` (`size` and `modified` from its metadata, `age` since then) read for the
/// watcher, keeping the backoff `before` knew of it (taken from it). `None` when it couldn't be
/// read, with `before` left as it was.
fn read_tracked(
    path: &Path,
    size: u64,
    modified: SystemTime,
    before: &mut Option<Tracked>,
    now: Instant,
    age: Duration,
) -> Option<Tracked> {
    let (bytes, _) = read_capped(path).ok()?;
    let complete = log_scan::complete_lines(&bytes);
    let (failures, retry_at, error, changed_at) = match before.take() {
        Some(t) => (t.failures, t.retry_at, t.error, now),
        // First seen: it's been quiet since it was last written.
        None => (0, None, None, now.checked_sub(age).unwrap_or(now)),
    };
    Some(Tracked {
        size,
        modified,
        changed_at,
        // Too large: what an upload records (`Prepared::size`) is its whole size.
        complete: if size > config::MAX_UPLOAD_BYTES {
            size
        } else {
            complete.len() as u64
        },
        scan: log_scan::scan(&String::from_utf8_lossy(complete)),
        failures,
        retry_at,
        error,
        waiting: false,
        due: false,
    })
}

/// The file at `path`, read up to `MAX_UPLOAD_BYTES` (all of it a server would take), and its
/// whole size: a file over the limit is never read into memory whole.
pub fn read_capped(path: &Path) -> io::Result<(Vec<u8>, u64)> {
    use std::io::Read;
    let file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut bytes = Vec::with_capacity(len.min(config::MAX_UPLOAD_BYTES) as usize);
    file.take(config::MAX_UPLOAD_BYTES)
        .read_to_end(&mut bytes)?;
    Ok((bytes, len))
}

/// The newest log file in `folder` (by name: Overwatch names them by when they start), the one
/// the game is writing if it's writing one. The same as `Tracker::live`.
pub fn newest_log(folder: &Path) -> io::Result<Option<PathBuf>> {
    let mut newest: Option<(String, PathBuf)> = None;
    for entry in fs::read_dir(folder)? {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !log_scan::is_log_file(&name) || !fs::metadata(entry.path()).is_ok_and(|m| m.is_file()) {
            continue;
        }
        if newest.as_ref().is_none_or(|(n, _)| name > *n) {
            newest = Some((name, entry.path()));
        }
    }
    Ok(newest.map(|(_, path)| path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    const EXAMPLE: &str = include_str!("../tests/fixtures/ranked-log-example.txt");
    const NAME: &str = "Log-2026-10-02-20-15-33.txt";
    const EPOCH: SystemTime = SystemTime::UNIX_EPOCH;

    /// The example log up to (not including) its `MATCH_END`.
    fn playing() -> String {
        let end = EXAMPLE.find("[00:01:54] MATCH_END").unwrap();
        EXAMPLE[..end].to_string()
    }

    fn write(dir: &Path, name: &str, text: &str, age: Duration) {
        let path = dir.join(name);
        fs::write(&path, text).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    fn names(poll: &Poll) -> Vec<&str> {
        poll.due.iter().map(|d| d.name.as_str()).collect()
    }

    fn never_sent(_: &str) -> Option<Sent> {
        None
    }

    fn backoff(failures: u32) -> Duration {
        Timing::default().backoff(failures)
    }

    fn later(now: Instant, wait: Duration) -> Instant {
        Timing::default().later(now, wait)
    }

    const QUIET_SECS: u64 = config::QUIET_SECS.default;
    const RETRY_FIRST_SECS: u64 = config::RETRY_FIRST_SECS.default;
    const RETRY_MAX_SECS: u64 = config::RETRY_MAX_SECS.default;

    #[test]
    fn uploads_a_match_as_soon_as_it_ends() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        write(dir.path(), NAME, &playing(), Duration::ZERO);
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(
            poll,
            Poll {
                due: vec![],
                waiting: 1
            }
        );

        write(dir.path(), NAME, EXAMPLE, Duration::ZERO);
        let poll = tracker
            .poll(
                dir.path(),
                now + Duration::from_secs(5),
                SystemTime::now(),
                EPOCH,
                never_sent,
            )
            .unwrap();
        assert_eq!(names(&poll), [NAME]);
        assert_eq!(poll.due[0].path, dir.path().join(NAME));
    }

    #[test]
    fn waits_for_the_end_of_a_half_written_match_end() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        // The game paused in the middle of the `MATCH_END` line: not an ended match yet.
        let half = format!("{}[00:01:54] MATCH_END|", playing());
        write(dir.path(), NAME, &half, Duration::ZERO);
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(
            poll,
            Poll {
                due: vec![],
                waiting: 1
            }
        );
        // The line is finished: due at once.
        write(dir.path(), NAME, EXAMPLE, Duration::ZERO);
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(names(&poll), [NAME]);
    }

    #[test]
    fn a_half_written_line_isnt_waiting_once_the_rest_was_sent() {
        let dir = tempfile::tempdir().unwrap();
        // Uploaded up to its last complete line, then left with half a line (stopped writing).
        let half = format!("{}[00:01:54] MATCH_END|", playing());
        write(dir.path(), NAME, &half, Duration::from_secs(QUIET_SECS + 5));
        let sent = |_: &str| {
            Some(Sent {
                size: playing().len() as u64,
                match_ends: 0,
            })
        };
        let poll = Tracker::default()
            .poll(dir.path(), Instant::now(), SystemTime::now(), EPOCH, sent)
            .unwrap();
        assert_eq!(poll, Poll::default());
    }

    #[test]
    fn uploads_a_file_that_stopped_growing() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        write(dir.path(), NAME, &playing(), Duration::ZERO);
        assert!(tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap()
            .due
            .is_empty());
        let later = now + Duration::from_secs(QUIET_SECS);
        let poll = tracker
            .poll(dir.path(), later, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(names(&poll), [NAME]);
    }

    #[test]
    fn a_quiet_file_found_at_startup_is_due_at_once() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            NAME,
            &playing(),
            Duration::from_secs(QUIET_SECS + 5),
        );
        let poll = Tracker::default()
            .poll(
                dir.path(),
                Instant::now(),
                SystemTime::now(),
                EPOCH,
                never_sent,
            )
            .unwrap();
        assert_eq!(names(&poll), [NAME]);
    }

    #[test]
    fn doesnt_send_a_file_twice() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), NAME, EXAMPLE, Duration::ZERO);
        let sent = |_: &str| {
            Some(Sent {
                size: EXAMPLE.len() as u64,
                match_ends: 1,
            })
        };
        let poll = Tracker::default()
            .poll(dir.path(), Instant::now(), SystemTime::now(), EPOCH, sent)
            .unwrap();
        assert_eq!(poll, Poll::default());
    }

    #[test]
    fn sends_a_file_again_once_it_grew() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        let first = playing();
        write(dir.path(), NAME, &first, Duration::ZERO);
        let sent = |_: &str| {
            Some(Sent {
                size: first.len() as u64,
                match_ends: 0,
            })
        };
        assert_eq!(
            tracker
                .poll(dir.path(), now, SystemTime::now(), EPOCH, sent)
                .unwrap(),
            Poll::default()
        );
        // More rounds, no `MATCH_END` yet: wait until it's quiet.
        let longer = format!("{first}[00:01:50] KILL|110.00||Ghost||4\n");
        write(dir.path(), NAME, &longer, Duration::ZERO);
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, sent)
            .unwrap();
        assert_eq!(
            poll,
            Poll {
                due: vec![],
                waiting: 1
            }
        );
        let later = now + Duration::from_secs(QUIET_SECS);
        assert_eq!(
            names(
                &tracker
                    .poll(dir.path(), later, SystemTime::now(), EPOCH, sent)
                    .unwrap()
            ),
            [NAME]
        );
    }

    #[test]
    fn doesnt_read_a_file_uploaded_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), NAME, EXAMPLE, Duration::ZERO);
        let mut tracker = Tracker::default();
        let sent = |name: &str| {
            (name == NAME).then_some(Sent {
                size: EXAMPLE.len() as u64,
                match_ends: 1,
            })
        };
        // The live log is read (AFK follows it), but isn't waiting or due.
        let poll = tracker
            .poll(dir.path(), Instant::now(), SystemTime::now(), EPOCH, sent)
            .unwrap();
        assert_eq!(poll, Poll::default());
        assert!(tracker.queue().is_empty());
        assert!(matches!(tracker.live_log(), Live::Read(..)));
        // Once a newer log is the live one, it isn't read any more.
        write(
            dir.path(),
            "Log-2026-10-02-21-00-00.txt",
            "say hi\n",
            Duration::ZERO,
        );
        tracker
            .poll(dir.path(), Instant::now(), SystemTime::now(), EPOCH, sent)
            .unwrap();
        assert!(!tracker.files.contains_key(NAME));
    }

    #[test]
    fn ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let old = Duration::from_secs(QUIET_SECS + 5);
        write(
            dir.path(),
            "Log-2026-10-02-20-00-00.txt",
            "[00:00:28] KILL|28.40|Sparrow|Ghost\n",
            old,
        );
        write(dir.path(), "notes.txt", EXAMPLE, old);
        fs::create_dir(dir.path().join("Log-folder.txt")).unwrap();
        let poll = Tracker::default()
            .poll(
                dir.path(),
                Instant::now(),
                SystemTime::now(),
                EPOCH,
                never_sent,
            )
            .unwrap();
        assert_eq!(poll, Poll::default());
    }

    #[test]
    fn an_old_match_still_waiting_to_upload_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        // Written after uploads started, but not uploaded for a month (offline, server down).
        let month = Duration::from_secs(30 * 24 * 60 * 60);
        let name = chrono::DateTime::<chrono::Local>::from(SystemTime::now() - month)
            .format("Log-%Y-%m-%d-%H-%M-%S.txt")
            .to_string();
        write(dir.path(), &name, EXAMPLE, month);
        let since = SystemTime::now() - month - month;
        let poll = Tracker::default()
            .poll(
                dir.path(),
                Instant::now(),
                SystemTime::now(),
                since,
                never_sent,
            )
            .unwrap();
        assert_eq!(names(&poll), [name.as_str()]);
    }

    #[test]
    fn sends_the_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let old = Duration::from_secs(QUIET_SECS + 5);
        write(dir.path(), "Log-2026-10-02-21-00-00.txt", EXAMPLE, old);
        write(dir.path(), "Log-2026-10-02-20-00-00.txt", EXAMPLE, old);
        let poll = Tracker::default()
            .poll(
                dir.path(),
                Instant::now(),
                SystemTime::now(),
                EPOCH,
                never_sent,
            )
            .unwrap();
        assert_eq!(
            names(&poll),
            ["Log-2026-10-02-20-00-00.txt", "Log-2026-10-02-21-00-00.txt"]
        );
    }

    #[test]
    fn holds_a_failed_upload_for_the_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        write(dir.path(), NAME, EXAMPLE, Duration::ZERO);
        assert_eq!(
            tracker
                .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
                .unwrap()
                .due
                .len(),
            1
        );

        tracker.failed(NAME, now, None, "offline".into());
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(
            poll,
            Poll {
                due: vec![],
                waiting: 1
            }
        );
        let retry = now + backoff(1);
        assert_eq!(
            names(
                &tracker
                    .poll(dir.path(), retry, SystemTime::now(), EPOCH, never_sent)
                    .unwrap()
            ),
            [NAME]
        );

        // A `Retry-After` wins over the backoff.
        tracker.failed(
            NAME,
            retry,
            Some(Duration::from_secs(3600)),
            "rate limited".into(),
        );
        let soon = retry + backoff(2);
        assert!(tracker
            .poll(dir.path(), soon, SystemTime::now(), EPOCH, never_sent)
            .unwrap()
            .due
            .is_empty());

        tracker.sent(NAME);
        assert_eq!(
            names(
                &tracker
                    .poll(dir.path(), soon, SystemTime::now(), EPOCH, never_sent)
                    .unwrap()
            ),
            [NAME]
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_file_it_cant_read_keeps_its_backoff() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        write(dir.path(), NAME, EXAMPLE, Duration::ZERO);
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        tracker.failed(NAME, now, None, "offline".into());
        // The game writes more while holding the file: it can't be read at this poll.
        write(dir.path(), NAME, &format!("{EXAMPLE}\n"), Duration::ZERO);
        let locked = File::options()
            .read(true)
            .share_mode(0)
            .open(dir.path().join(NAME))
            .unwrap();
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(
            poll,
            Poll {
                due: vec![],
                waiting: 1
            }
        );
        drop(locked);
        // Read again, still held for the backoff, and still failed.
        assert!(tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap()
            .due
            .is_empty());
        assert_eq!(
            states(&tracker),
            [QueueState::Failed {
                error: "offline".into()
            }]
        );
        assert_eq!(
            names(
                &tracker
                    .poll(
                        dir.path(),
                        now + backoff(1),
                        SystemTime::now(),
                        EPOCH,
                        never_sent
                    )
                    .unwrap()
            ),
            [NAME]
        );
    }

    fn states(tracker: &Tracker) -> Vec<QueueState> {
        tracker.queue().into_iter().map(|q| q.state).collect()
    }

    #[test]
    fn lists_the_files_waiting_to_upload() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        let other = "Log-2026-10-02-21-00-00.txt";
        write(dir.path(), NAME, &playing(), Duration::ZERO);
        write(dir.path(), other, EXAMPLE, Duration::ZERO);
        // A file already uploaded as it is isn't listed.
        write(
            dir.path(),
            "Log-2026-10-01-20-00-00.txt",
            EXAMPLE,
            Duration::ZERO,
        );
        let sent = |name: &str| {
            (name == "Log-2026-10-01-20-00-00.txt").then_some(Sent {
                size: EXAMPLE.len() as u64,
                match_ends: 1,
            })
        };
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, sent)
            .unwrap();
        let queue = tracker.queue();
        assert_eq!(
            queue.iter().map(|q| q.file.as_str()).collect::<Vec<_>>(),
            [NAME, other]
        );
        assert_eq!(queue[0].state, QueueState::Playing);
        assert_eq!(queue[1].state, QueueState::Due);
        assert_eq!(queue[1].players.len(), 5);

        tracker.failed(other, now, None, "offline".into());
        assert_eq!(
            states(&tracker)[1],
            QueueState::Failed {
                error: "offline".into()
            }
        );
        // Still failed at the next poll, while it's held.
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, sent)
            .unwrap();
        assert_eq!(
            states(&tracker)[1],
            QueueState::Failed {
                error: "offline".into()
            }
        );
        tracker.sent(other);
        assert_eq!(states(&tracker), [QueueState::Playing]);
    }

    #[test]
    fn retry_drops_the_backoff_of_a_failed_upload_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        write(dir.path(), NAME, EXAMPLE, Duration::ZERO);
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        // Not failed: nothing to retry.
        assert!(!tracker.retry(NAME));
        assert!(!tracker.retry("Log-unknown.txt"));

        tracker.failed(NAME, now, Some(Duration::from_secs(3600)), "offline".into());
        assert!(tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap()
            .due
            .is_empty());
        assert!(tracker.retry(NAME));
        assert_eq!(
            names(
                &tracker
                    .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
                    .unwrap()
            ),
            [NAME]
        );
        // The backoff starts over: the next failure waits the first step, not a doubled one.
        tracker.failed(NAME, now, None, "offline".into());
        assert_eq!(
            names(
                &tracker
                    .poll(
                        dir.path(),
                        now + backoff(1),
                        SystemTime::now(),
                        EPOCH,
                        never_sent
                    )
                    .unwrap()
            ),
            [NAME]
        );
    }

    #[test]
    fn retry_doesnt_send_an_uploaded_file_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        write(dir.path(), NAME, EXAMPLE, Duration::ZERO);
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        tracker.failed(NAME, now, None, "offline".into());
        // It was uploaded since (the next try worked, say): it isn't waiting any more.
        let uploaded = |_: &str| {
            Some(Sent {
                size: EXAMPLE.len() as u64,
                match_ends: 1,
            })
        };
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, uploaded)
            .unwrap();
        assert!(!tracker.retry(NAME));
        assert_eq!(
            tracker
                .poll(dir.path(), now, SystemTime::now(), EPOCH, uploaded)
                .unwrap(),
            Poll::default()
        );
        assert!(tracker.queue().is_empty());
    }

    #[test]
    fn skips_a_file_refused_for_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let big = format!("{EXAMPLE}{}", "x".repeat(config::MAX_UPLOAD_BYTES as usize));
        write(dir.path(), NAME, &big, Duration::ZERO);
        // Never sent: due once, so the host sees why it wasn't uploaded.
        let poll = Tracker::default()
            .poll(
                dir.path(),
                Instant::now(),
                SystemTime::now(),
                EPOCH,
                never_sent,
            )
            .unwrap();
        assert_eq!(names(&poll), [NAME]);
        // Refused at a size over the limit: left alone from then on, even with a new `MATCH_END`.
        let refused = |_: &str| {
            Some(Sent {
                size: config::MAX_UPLOAD_BYTES + 1,
                match_ends: 0,
            })
        };
        let poll = Tracker::default()
            .poll(
                dir.path(),
                Instant::now(),
                SystemTime::now(),
                EPOCH,
                refused,
            )
            .unwrap();
        assert_eq!(poll, Poll::default());
    }

    #[test]
    fn reads_a_file_only_up_to_the_upload_limit() {
        let dir = tempfile::tempdir().unwrap();
        let big = format!("{EXAMPLE}{}", "x".repeat(config::MAX_UPLOAD_BYTES as usize));
        write(dir.path(), NAME, &big, Duration::ZERO);
        let (bytes, len) = read_capped(&dir.path().join(NAME)).unwrap();
        assert_eq!(bytes.len() as u64, config::MAX_UPLOAD_BYTES);
        assert_eq!(len, big.len() as u64);
        assert!(bytes.starts_with(EXAMPLE.as_bytes()));
        write(dir.path(), NAME, EXAMPLE, Duration::ZERO);
        let (bytes, len) = read_capped(&dir.path().join(NAME)).unwrap();
        assert_eq!(
            (bytes.as_slice(), len),
            (EXAMPLE.as_bytes(), EXAMPLE.len() as u64)
        );
    }

    #[test]
    fn leaves_files_from_before_uploads_started() {
        let dir = tempfile::tempdir().unwrap();
        let since = SystemTime::now() - Duration::from_secs(600);
        let named = |at: SystemTime| {
            chrono::DateTime::<chrono::Local>::from(at)
                .format("Log-%Y-%m-%d-%H-%M-%S.txt")
                .to_string()
        };
        let before = named(since - Duration::from_secs(60));
        let after = named(since + Duration::from_secs(60));
        // Both just ended; the first started before uploads to this server did.
        write(dir.path(), &before, EXAMPLE, Duration::ZERO);
        write(dir.path(), &after, EXAMPLE, Duration::ZERO);
        let poll = Tracker::default()
            .poll(
                dir.path(),
                Instant::now(),
                SystemTime::now(),
                since,
                never_sent,
            )
            .unwrap();
        assert_eq!(names(&poll), [after.as_str()]);
    }

    #[test]
    fn backs_off_up_to_the_max() {
        assert_eq!(backoff(1), Duration::from_secs(RETRY_FIRST_SECS));
        assert_eq!(backoff(2), Duration::from_secs(RETRY_FIRST_SECS * 2));
        assert_eq!(backoff(100), Duration::from_secs(RETRY_MAX_SECS));
        let now = Instant::now();
        assert_eq!(
            later(now, Duration::from_secs(3600)),
            now + Duration::from_secs(3600)
        );
        assert!(later(now, Duration::MAX) > now);
        // However many failures: no overflow, and no cap but `RETRY_MAX_SECS`.
        assert_eq!(backoff(u32::MAX), Duration::from_secs(RETRY_MAX_SECS));
        assert_eq!(backoff(0), backoff(1));
        assert_eq!(
            backoff(20),
            Duration::from_secs((RETRY_FIRST_SECS << 19).min(RETRY_MAX_SECS))
        );
    }

    #[test]
    fn waits_as_the_host_set() {
        let dir = tempfile::tempdir().unwrap();
        let ago = Duration::from_secs(20);
        write(dir.path(), NAME, &playing(), ago);
        let mut tracker = Tracker::default();
        let now = Instant::now();
        // 20 s quiet: not long enough by default.
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert!(poll.due.is_empty());
        tracker.timing.quiet = Duration::from_secs(10);
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(names(&poll), [NAME]);

        let timing = Timing {
            quiet: Duration::from_secs(10),
            retry_first: Duration::from_secs(5),
            retry_max: Duration::from_secs(12),
        };
        assert_eq!(timing.backoff(1), Duration::from_secs(5));
        assert_eq!(timing.backoff(2), Duration::from_secs(10));
        assert_eq!(timing.backoff(3), Duration::from_secs(12));
        // A first wait longer than the longest: the longest.
        let odd = Timing {
            retry_first: Duration::from_secs(60),
            ..timing
        };
        assert_eq!(odd.backoff(1), Duration::from_secs(12));
    }

    #[test]
    fn knows_the_live_log_and_its_rounds() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        assert_eq!(newest_log(dir.path()).unwrap(), None);
        write(
            dir.path(),
            "Log-2026-10-02-19-00-00.txt",
            EXAMPLE,
            Duration::ZERO,
        );
        write(dir.path(), NAME, &playing(), Duration::ZERO);
        write(dir.path(), "notes.txt", EXAMPLE, Duration::ZERO);
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(tracker.live(), Some(NAME));
        assert_eq!(newest_log(dir.path()).unwrap(), Some(dir.path().join(NAME)));
        let Live::Read(scan, _) = tracker.live_log() else {
            panic!("{:?}", tracker.live_log());
        };
        let rounds: Vec<u32> = scan.round_starts.iter().map(|s| s.round).collect();
        assert_eq!(rounds, [1, 2, 3]);

        // Uploaded as it is (the lobby went quiet): still the live log, and still read, so a
        // sit-out in it goes on. A fresh tracker (a restart) reads it too.
        let sent = |name: &str| {
            Some(Sent {
                size: fs::metadata(dir.path().join(name)).unwrap().len(),
                match_ends: 0,
            })
        };
        for mut tracker in [tracker, Tracker::default()] {
            tracker
                .poll(dir.path(), now, SystemTime::now(), EPOCH, sent)
                .unwrap();
            assert_eq!(tracker.live(), Some(NAME));
            let Live::Read(scan, at) = tracker.live_log() else {
                panic!("{:?}", tracker.live_log());
            };
            assert_eq!(scan.round_starts.len(), 3);
            assert_eq!((at.name, at.len), (NAME, playing().len() as u64));
        }
        let mut tracker = Tracker::default();

        // Read, then uploaded before the next poll: still read.
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert!(matches!(tracker.live_log(), Live::Read(..)));
        tracker.sent(NAME);
        assert!(matches!(tracker.live_log(), Live::Read(..)));

        // A log that isn't a ranked one, and no log at all.
        write(
            dir.path(),
            "Log-2026-10-02-21-00-00.txt",
            "say hi\n",
            Duration::ZERO,
        );
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(tracker.live_log(), Live::Gone);
        let empty = tempfile::tempdir().unwrap();
        tracker
            .poll(empty.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(tracker.live_log(), Live::Gone);
    }

    #[test]
    fn a_live_log_it_doesnt_read_isnt_gone() {
        let now = Instant::now();
        // From before uploads started (the match was going on when the token was saved).
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), NAME, &playing(), Duration::ZERO);
        let mut tracker = Tracker::default();
        tracker
            .poll(
                dir.path(),
                now,
                SystemTime::now(),
                SystemTime::now(),
                never_sent,
            )
            .unwrap();
        assert_eq!(tracker.live(), Some(NAME));
        assert_eq!(tracker.live_log(), Live::Unread);

        // Too large to upload: only read up to the limit, so not what's in it now. Refused once
        // for its size, it isn't read at all.
        let big = format!(
            "{}{}",
            playing(),
            "x\n".repeat(config::MAX_UPLOAD_BYTES as usize)
        );
        write(dir.path(), NAME, &big, Duration::ZERO);
        let refused = |_: &str| {
            Some(Sent {
                size: config::MAX_UPLOAD_BYTES + 1,
                match_ends: 0,
            })
        };
        for sent in [never_sent as fn(&str) -> Option<Sent>, refused] {
            let mut tracker = Tracker::default();
            tracker
                .poll(dir.path(), now, SystemTime::now(), EPOCH, sent)
                .unwrap();
            assert_eq!(tracker.live_log(), Live::Unread);
        }

        // A new file with no whole line in it yet (a spectator copy starting).
        write(
            dir.path(),
            "Log-2026-10-02-21-00-00.txt",
            "[00:00",
            Duration::ZERO,
        );
        let mut tracker = Tracker::default();
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
            .unwrap();
        assert_eq!(tracker.live_log(), Live::Unread);
    }

    #[test]
    fn reads_the_live_log_at_the_click() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_live(dir.path()), Ok(LiveNow::Gone));
        // Read whole, with a half-written last line once its round is complete.
        let text = format!("{}[00:01", playing());
        write(dir.path(), NAME, &text, Duration::ZERO);
        let LiveNow::Read {
            scan,
            folder,
            name,
            len,
        } = read_live(dir.path()).unwrap()
        else {
            panic!();
        };
        assert_eq!(scan.round_starts.len(), 3);
        assert_eq!(
            (folder.as_path(), name.as_str(), len),
            (dir.path(), NAME, playing().len() as u64)
        );
        // Too large to upload: still read, whole.
        let big = format!(
            "{}{}",
            "x\n".repeat(config::MAX_UPLOAD_BYTES as usize),
            playing()
        );
        write(dir.path(), NAME, &big, Duration::ZERO);
        let LiveNow::Read { scan, .. } = read_live(dir.path()).unwrap() else {
            panic!();
        };
        assert_eq!(scan.round_starts.len(), 3);
        // Nothing written yet, then not a ranked log.
        write(
            dir.path(),
            "Log-2026-10-02-21-00-00.txt",
            "",
            Duration::ZERO,
        );
        assert_eq!(read_live(dir.path()), Ok(LiveNow::Unread));
        write(
            dir.path(),
            "Log-2026-10-02-21-00-00.txt",
            "say hi\n",
            Duration::ZERO,
        );
        assert_eq!(read_live(dir.path()), Ok(LiveNow::Gone));
        // No folder: an error, not gone.
        assert!(read_live(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn a_later_file_or_more_of_one_is_a_newer_read() {
        let folder = Path::new("logs");
        let at = |name, len| LogAt { folder, name, len };
        assert!(at(NAME, 10).is_before(&at(NAME, 20)));
        assert!(!at(NAME, 20).is_before(&at(NAME, 20)));
        assert!(!at(NAME, 20).is_before(&at(NAME, 10)));
        // A later file, even with less in it.
        assert!(at(NAME, 900).is_before(&at("Log-2026-10-02-21-00-00.txt", 0)));
        // Another folder is never older.
        let other = LogAt {
            folder: Path::new("other"),
            name: "Log-2026-10-02-21-00-00.txt",
            len: 900,
        };
        assert!(!at(NAME, 10).is_before(&other));
        assert!(!other.is_before(&at(NAME, 10)));
    }

    /// A live log another program holds without sharing it (as the game may, for a moment).
    #[cfg(windows)]
    #[test]
    fn a_live_log_that_cant_be_read_isnt_known() {
        use std::io::Write;
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        let poll = |tracker: &mut Tracker| {
            tracker
                .poll(dir.path(), now, SystemTime::now(), EPOCH, never_sent)
                .unwrap();
        };
        let path = dir.path().join(NAME);
        let hold = || {
            fs::File::options()
                .append(true)
                .share_mode(0)
                .open(&path)
                .unwrap()
        };
        // Never read: not known.
        write(dir.path(), NAME, &playing(), Duration::ZERO);
        let mut held = hold();
        poll(&mut tracker);
        assert_eq!(tracker.live_log(), Live::Unread);
        drop(held);
        poll(&mut tracker);
        assert!(matches!(tracker.live_log(), Live::Read(..)));
        // Read before, grown but held now: what was read stays.
        held = hold();
        held.write_all(b"JOIN|9|9|Late|\n").unwrap();
        poll(&mut tracker);
        let Live::Read(scan, _) = tracker.live_log() else {
            panic!("{:?}", tracker.live_log());
        };
        assert!(!scan.players.iter().any(|p| p == "Late"));
        // An older log held doesn't hide the live one.
        drop(held);
        let older = dir.path().join("Log-2026-10-02-18-00-00.txt");
        fs::write(&older, EXAMPLE).unwrap();
        let _older = fs::File::options()
            .read(true)
            .share_mode(0)
            .open(&older)
            .unwrap();
        poll(&mut tracker);
        assert!(matches!(tracker.live_log(), Live::Read(..)));
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Tracker::default()
            .poll(
                &dir.path().join("gone"),
                Instant::now(),
                SystemTime::now(),
                EPOCH,
                never_sent
            )
            .is_err());
    }
}
