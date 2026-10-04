//! Which log files to upload now. Each poll reads the sizes in the log folder, reads a file again
//! only when it changed, and picks the ranked files that changed since they were last sent and
//! either have a new `MATCH_END` or stopped growing (`config::QUIET_SECS`).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::config;
use crate::log_scan::{self, Scan};

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

struct Tracked {
    size: u64,
    modified: SystemTime,
    /// When the file last grew, as far as we know.
    changed_at: Instant,
    scan: Scan,
    failures: u32,
    retry_at: Option<Instant>,
}

#[derive(Default)]
pub struct Tracker {
    folder: PathBuf,
    files: HashMap<String, Tracked>,
}

impl Tracker {
    /// Looks at `folder` at `now` (`clock` being the same moment as wall time). `sent` gives the
    /// last upload of a file to the current server.
    pub fn poll(
        &mut self,
        folder: &Path,
        now: Instant,
        clock: SystemTime,
        sent: impl Fn(&str) -> Option<Sent>,
    ) -> io::Result<Poll> {
        if self.folder != folder {
            self.folder = folder.to_path_buf();
            self.files.clear();
        }
        let max_age = Duration::from_secs(config::MAX_LOG_AGE_DAYS * 24 * 60 * 60);
        let quiet = Duration::from_secs(config::QUIET_SECS);
        let mut poll = Poll::default();
        let mut seen = Vec::new();
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
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let age = clock.duration_since(modified).unwrap_or_default();
            if !meta.is_file() || age > max_age {
                continue;
            }
            let size = meta.len();
            let tracked = match self.files.remove(&name) {
                Some(tracked) if tracked.size == size && tracked.modified == modified => tracked,
                before => {
                    // A file still being written may be locked for a moment: try on the next poll.
                    let Ok(bytes) = fs::read(&path) else { continue };
                    let (failures, retry_at, changed_at) = match before {
                        Some(t) => (t.failures, t.retry_at, now),
                        // First seen: it's been quiet since it was last written.
                        None => (0, None, now.checked_sub(age).unwrap_or(now)),
                    };
                    Tracked {
                        size,
                        modified,
                        changed_at,
                        scan: log_scan::scan(&String::from_utf8_lossy(&bytes)),
                        failures,
                        retry_at,
                    }
                }
            };
            let last = sent(&name);
            if tracked.scan.ranked && last.map(|s| s.size) != Some(size) {
                poll.waiting += 1;
                let ended = tracked.scan.match_ends > last.map_or(0, |s| s.match_ends);
                let stopped = now.saturating_duration_since(tracked.changed_at) >= quiet;
                let held = tracked.retry_at.is_some_and(|at| now < at);
                if (ended || stopped) && !held {
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

    /// The upload of `name` worked, or won't work however often it's tried.
    pub fn sent(&mut self, name: &str) {
        if let Some(t) = self.files.get_mut(name) {
            t.failures = 0;
            t.retry_at = None;
        }
    }

    /// The upload of `name` failed: hold it for `after`, or a backoff that grows with each failure.
    pub fn failed(&mut self, name: &str, now: Instant, after: Option<Duration>) {
        if let Some(t) = self.files.get_mut(name) {
            t.failures += 1;
            let wait = after.unwrap_or_else(|| backoff(t.failures));
            // A `Retry-After` too far off to add is treated as the longest backoff.
            t.retry_at = Some(
                now.checked_add(wait)
                    .unwrap_or_else(|| now + Duration::from_secs(config::RETRY_MAX_SECS)),
            );
        }
    }
}

/// `RETRY_FIRST_SECS`, doubled for each failure after the first, at most `RETRY_MAX_SECS`.
pub fn backoff(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    Duration::from_secs(
        config::RETRY_FIRST_SECS
            .saturating_mul(1 << doublings)
            .min(config::RETRY_MAX_SECS),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    const EXAMPLE: &str = include_str!("../tests/fixtures/ranked-log-example.txt");
    const NAME: &str = "Log-2026-10-02-20-15-33.txt";

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

    #[test]
    fn uploads_a_match_as_soon_as_it_ends() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        write(dir.path(), NAME, &playing(), Duration::ZERO);
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), never_sent)
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
                never_sent,
            )
            .unwrap();
        assert_eq!(names(&poll), [NAME]);
        assert_eq!(poll.due[0].path, dir.path().join(NAME));
    }

    #[test]
    fn uploads_a_file_that_stopped_growing() {
        let dir = tempfile::tempdir().unwrap();
        let mut tracker = Tracker::default();
        let now = Instant::now();
        write(dir.path(), NAME, &playing(), Duration::ZERO);
        assert!(tracker
            .poll(dir.path(), now, SystemTime::now(), never_sent)
            .unwrap()
            .due
            .is_empty());
        let later = now + Duration::from_secs(config::QUIET_SECS);
        let poll = tracker
            .poll(dir.path(), later, SystemTime::now(), never_sent)
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
            Duration::from_secs(config::QUIET_SECS + 5),
        );
        let poll = Tracker::default()
            .poll(dir.path(), Instant::now(), SystemTime::now(), never_sent)
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
            .poll(dir.path(), Instant::now(), SystemTime::now(), sent)
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
                .poll(dir.path(), now, SystemTime::now(), sent)
                .unwrap(),
            Poll::default()
        );
        // More rounds, no `MATCH_END` yet: wait until it's quiet.
        let longer = format!("{first}[00:01:50] KILL|110.00||Ghost||4\n");
        write(dir.path(), NAME, &longer, Duration::ZERO);
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), sent)
            .unwrap();
        assert_eq!(
            poll,
            Poll {
                due: vec![],
                waiting: 1
            }
        );
        let later = now + Duration::from_secs(config::QUIET_SECS);
        assert_eq!(
            names(
                &tracker
                    .poll(dir.path(), later, SystemTime::now(), sent)
                    .unwrap()
            ),
            [NAME]
        );
    }

    #[test]
    fn ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let old = Duration::from_secs(config::QUIET_SECS + 5);
        write(
            dir.path(),
            "Log-2026-10-02-20-00-00.txt",
            "[00:00:28] KILL|28.40|Sparrow|Ghost\n",
            old,
        );
        write(dir.path(), "notes.txt", EXAMPLE, old);
        write(
            dir.path(),
            NAME,
            EXAMPLE,
            Duration::from_secs(config::MAX_LOG_AGE_DAYS * 24 * 60 * 60 + 60),
        );
        fs::create_dir(dir.path().join("Log-folder.txt")).unwrap();
        let poll = Tracker::default()
            .poll(dir.path(), Instant::now(), SystemTime::now(), never_sent)
            .unwrap();
        assert_eq!(poll, Poll::default());
    }

    #[test]
    fn sends_the_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let old = Duration::from_secs(config::QUIET_SECS + 5);
        write(dir.path(), "Log-2026-10-02-21-00-00.txt", EXAMPLE, old);
        write(dir.path(), "Log-2026-10-02-20-00-00.txt", EXAMPLE, old);
        let poll = Tracker::default()
            .poll(dir.path(), Instant::now(), SystemTime::now(), never_sent)
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
                .poll(dir.path(), now, SystemTime::now(), never_sent)
                .unwrap()
                .due
                .len(),
            1
        );

        tracker.failed(NAME, now, None);
        let poll = tracker
            .poll(dir.path(), now, SystemTime::now(), never_sent)
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
                    .poll(dir.path(), retry, SystemTime::now(), never_sent)
                    .unwrap()
            ),
            [NAME]
        );

        // A `Retry-After` wins over the backoff.
        tracker.failed(NAME, retry, Some(Duration::from_secs(3600)));
        let soon = retry + backoff(2);
        assert!(tracker
            .poll(dir.path(), soon, SystemTime::now(), never_sent)
            .unwrap()
            .due
            .is_empty());

        tracker.sent(NAME);
        assert_eq!(
            names(
                &tracker
                    .poll(dir.path(), soon, SystemTime::now(), never_sent)
                    .unwrap()
            ),
            [NAME]
        );
    }

    #[test]
    fn backs_off_up_to_the_max() {
        assert_eq!(backoff(1), Duration::from_secs(config::RETRY_FIRST_SECS));
        assert_eq!(
            backoff(2),
            Duration::from_secs(config::RETRY_FIRST_SECS * 2)
        );
        assert_eq!(backoff(100), Duration::from_secs(config::RETRY_MAX_SECS));
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Tracker::default()
            .poll(
                &dir.path().join("gone"),
                Instant::now(),
                SystemTime::now(),
                never_sent
            )
            .is_err());
    }
}
