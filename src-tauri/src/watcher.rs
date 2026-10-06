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
use crate::log_scan::{self, RoundStart, Scan};
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

#[derive(Default)]
pub struct Tracker {
    folder: PathBuf,
    files: HashMap<String, Tracked>,
    /// The newest log file in the folder at the last poll (`newest_log`): the one being written.
    live: Option<String>,
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
            if last.is_some_and(|s| s.size == size) {
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
                before => {
                    // A file still being written may be locked for a moment: try on the next poll.
                    let Ok(bytes) = fs::read(&path) else { continue };
                    let complete = log_scan::complete_lines(&bytes);
                    let (failures, retry_at, error, changed_at) = match before {
                        Some(t) => (t.failures, t.retry_at, t.error, now),
                        // First seen: it's been quiet since it was last written.
                        None => (0, None, None, now.checked_sub(age).unwrap_or(now)),
                    };
                    Tracked {
                        size,
                        modified,
                        changed_at,
                        complete: complete.len() as u64,
                        scan: log_scan::scan(&String::from_utf8_lossy(complete)),
                        failures,
                        retry_at,
                        error,
                        waiting: false,
                        due: false,
                    }
                }
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
        self.files = seen.into_iter().collect();
        // Oldest first, so matches reach the server in the order they were played.
        poll.due.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(poll)
    }

    /// The newest log file at the last poll: the one the game is writing, if any is.
    pub fn live(&self) -> Option<&str> {
        self.live.as_deref()
    }

    /// The rounds started in the live log, as last read. None when it isn't read: it isn't a
    /// ranked log, or it hasn't changed since it was uploaded as it is (nor been read since).
    pub fn live_rounds(&self) -> &[RoundStart] {
        self.live
            .as_ref()
            .and_then(|name| self.files.get(name))
            .map_or(&[], |t| &t.scan.round_starts)
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
        let sent = |_: &str| {
            Some(Sent {
                size: EXAMPLE.len() as u64,
                match_ends: 1,
            })
        };
        tracker
            .poll(dir.path(), Instant::now(), SystemTime::now(), EPOCH, sent)
            .unwrap();
        assert!(tracker.files.is_empty());
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
        let rounds: Vec<u32> = tracker.live_rounds().iter().map(|s| s.round).collect();
        assert_eq!(rounds, [1, 2, 3]);

        // Uploaded as it is: not read, so no rounds, but still the live log.
        let sent = |name: &str| {
            Some(Sent {
                size: fs::metadata(dir.path().join(name)).unwrap().len(),
                match_ends: 0,
            })
        };
        tracker
            .poll(dir.path(), now, SystemTime::now(), EPOCH, sent)
            .unwrap();
        assert_eq!(tracker.live(), Some(NAME));
        assert!(tracker.live_rounds().is_empty());
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
