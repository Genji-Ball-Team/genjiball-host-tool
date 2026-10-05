//! Host AFK (GenjiBall-CE `docs/ranked-log.md`, "Host AFK"). While the host has AFK on, every
//! round that starts isn't rated for them: the server drops them from it as if they had left. The
//! game can't see the button, so the tool notes the rounds here, per `matchKey`, and sends them
//! with each upload of a file holding that match (`X-Host-Afk`, genjiball-ranked `docs/api.md`).
//!
//! A round counts when it starts after AFK was turned on. When the host turns it on, the live log
//! (the newest file) is read at once, and the last round each of its matches had started then is
//! kept (`seen`): those rounds, and the same rounds in a spectator copy of the match, never count,
//! even if the uploader only reads the file afterwards. Every round past that, in any match of the
//! live log, counts until AFK is turned off. It stays on across matches and restarts (`afk.json`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::log_scan::RoundStart;
use crate::settings;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Afk {
    on: bool,
    /// `matchKey` → the last round it had started when AFK was turned on. Only while it's on.
    seen: BTreeMap<String, u32>,
    /// `matchKey` → the rounds that started while AFK was on.
    matches: BTreeMap<String, Rounds>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rounds {
    rounds: BTreeSet<u32>,
    /// When the last of them was noted (Unix seconds): the match is forgotten `keep` after.
    at: u64,
}

fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Afk {
    /// Turns AFK on, with `started` the rounds already started in the live log. `false` when it
    /// was on already: what was seen when it was turned on stays.
    pub fn turn_on(&mut self, started: &[RoundStart]) -> bool {
        if self.on {
            return false;
        }
        self.on = true;
        self.seen.clear();
        for start in started {
            let last = self.seen.entry(start.match_key.clone()).or_default();
            *last = (*last).max(start.round);
        }
        true
    }

    /// Turns AFK off, after noting the rounds in `started` (the live log, read now) that started
    /// while it was on but weren't read yet. The rounds noted stay. `false` when it was off.
    pub fn turn_off(&mut self, started: &[RoundStart], now: SystemTime) -> bool {
        if !self.on {
            return false;
        }
        self.saw(started, now);
        self.on = false;
        self.seen.clear();
        true
    }

    /// Notes the rounds in `started` (read from the live log) that started while AFK is on.
    /// `true` when one is new.
    pub fn saw(&mut self, started: &[RoundStart], now: SystemTime) -> bool {
        if !self.on {
            return false;
        }
        let mut changed = false;
        for start in started {
            if self
                .seen
                .get(&start.match_key)
                .is_some_and(|&last| start.round <= last)
            {
                continue;
            }
            let rounds = self.matches.entry(start.match_key.clone()).or_default();
            if rounds.rounds.insert(start.round) {
                rounds.at = unix_secs(now);
                changed = true;
            }
        }
        changed
    }

    /// Forgets the matches whose last AFK round is older than `keep`: long uploaded by then.
    /// `true` when one was.
    pub fn prune(&mut self, now: SystemTime, keep: Duration) -> bool {
        let oldest = unix_secs(now).saturating_sub(keep.as_secs());
        let before = self.matches.len();
        self.matches.retain(|_, rounds| rounds.at >= oldest);
        self.matches.len() != before
    }

    /// `X-Host-Afk` for a file holding the matches `keys`: `matchKey:round,round,...` for each of
    /// them with AFK rounds, joined with `;`. `None` when none has any.
    pub fn header(&self, keys: &[String]) -> Option<String> {
        let parts: Vec<String> = keys
            .iter()
            .filter_map(|key| {
                let rounds = &self.matches.get(key)?.rounds;
                let list: Vec<String> = rounds.iter().map(u32::to_string).collect();
                (!list.is_empty()).then(|| format!("{key}:{}", list.join(",")))
            })
            .collect();
        (!parts.is_empty()).then(|| parts.join(";"))
    }

    /// The match an AFK round was last noted in, and its AFK rounds.
    fn latest(&self) -> Option<LatestMatch> {
        let (key, rounds) = self.matches.iter().max_by_key(|(_, r)| r.at)?;
        Some(LatestMatch {
            match_key: key.clone(),
            rounds: rounds.rounds.iter().copied().collect(),
        })
    }
}

/// What the window shows about AFK.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AfkStatus {
    pub on: bool,
    /// The match an AFK round was last noted in.
    pub latest: Option<LatestMatch>,
    /// Why `afk.json` couldn't be read or written.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LatestMatch {
    pub match_key: String,
    pub rounds: Vec<u32>,
}

/// `Afk` and its file, `afk.json`. A file that can't be read leaves AFK off, with the error in
/// the window, and isn't written over until the host turns AFK on or off.
pub struct AfkStore {
    path: PathBuf,
    afk: Afk,
    /// Changed since it was last written.
    dirty: bool,
    unreadable: bool,
    error: Option<String>,
}

impl AfkStore {
    pub fn open(path: PathBuf) -> Self {
        let (afk, error) = match load(&path) {
            Ok(afk) => (afk, None),
            Err(e) => (Afk::default(), Some(e)),
        };
        Self {
            path,
            afk,
            dirty: false,
            unreadable: error.is_some(),
            error,
        }
    }

    pub fn status(&self) -> AfkStatus {
        AfkStatus {
            on: self.afk.on,
            latest: self.afk.latest(),
            error: self.error.clone(),
        }
    }

    pub fn header(&self, keys: &[String]) -> Option<String> {
        self.afk.header(keys)
    }

    /// The host turned AFK on or off, with `started` the rounds in the live log now. Written at
    /// once, even over a file that couldn't be read.
    pub fn set(&mut self, on: bool, started: &[RoundStart], now: SystemTime) -> AfkStatus {
        let changed = if on {
            self.afk.turn_on(started)
        } else {
            self.afk.turn_off(started, now)
        };
        if changed || self.unreadable {
            self.dirty = true;
            self.unreadable = false;
            self.save();
        }
        self.status()
    }

    /// The uploader read the live log: notes its new AFK rounds and forgets old matches.
    pub fn saw(&mut self, started: &[RoundStart], now: SystemTime, keep: Duration) {
        let saw = self.afk.saw(started, now);
        let pruned = self.afk.prune(now, keep);
        self.dirty |= saw || pruned;
        self.save();
    }

    /// Writes what changed. One that fails is tried again at the next change or poll.
    fn save(&mut self) {
        if !self.dirty || self.unreadable {
            return;
        }
        match settings::save_json(&self.path, &self.afk) {
            Ok(()) => {
                self.dirty = false;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
}

/// What's in `path`: off with no rounds when there's no file yet.
fn load(path: &Path) -> Result<Afk, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            format!(
                "{} isn't a valid AFK record: {e}. AFK is off until you turn it on again",
                path.display()
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Afk::default()),
        Err(e) => Err(format!(
            "Couldn't read {}: {e}. AFK is off until you turn it on again",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "482913507226";
    const DAY: Duration = Duration::from_secs(24 * 3600);

    fn starts(key: &str, rounds: impl IntoIterator<Item = u32>) -> Vec<RoundStart> {
        rounds
            .into_iter()
            .map(|round| RoundStart {
                match_key: key.into(),
                round,
            })
            .collect()
    }

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000)
    }

    #[test]
    fn counts_rounds_that_start_after_afk_is_turned_on() {
        let mut afk = Afk::default();
        // Off: nothing counts.
        assert!(!afk.saw(&starts(KEY, [1, 2]), now()));
        assert_eq!(afk.header(&[KEY.into()]), None);

        // Round 3 had started when the host turned AFK on, even if it wasn't read yet.
        assert!(afk.turn_on(&starts(KEY, [1, 2, 3])));
        assert!(!afk.saw(&starts(KEY, [1, 2, 3]), now()));
        assert!(afk.saw(&starts(KEY, [1, 2, 3, 4]), now()));
        assert!(!afk.saw(&starts(KEY, [1, 2, 3, 4]), now()), "seen already");
        assert!(afk.saw(&starts(KEY, [1, 2, 3, 4, 5]), now()));
        assert_eq!(
            afk.header(&[KEY.into()]).as_deref(),
            Some("482913507226:4,5")
        );
    }

    #[test]
    fn stays_on_across_matches_until_turned_off() {
        let mut afk = Afk::default();
        afk.turn_on(&starts("1", [1, 2]));
        afk.saw(&starts("1", [1, 2, 3]), now());
        // The next match: every round counts.
        afk.saw(&starts("2", [1, 2]), now());
        // A second click on "on" doesn't move what was seen.
        assert!(!afk.turn_on(&starts("2", [1, 2, 3])));
        // Round 3 started before the host turned it off, not read until now: it counts.
        assert!(afk.turn_off(&starts("2", [1, 2, 3]), now()));
        assert!(!afk.saw(&starts("2", [1, 2, 3, 4]), now()));
        assert_eq!(
            afk.header(&["1".into(), "2".into(), "3".into()]).as_deref(),
            Some("1:3;2:1,2,3")
        );
        // Only the matches in the file.
        assert_eq!(afk.header(&["2".into()]).as_deref(), Some("2:1,2,3"));
        assert_eq!(afk.header(&["3".into()]), None);
        assert!(!afk.turn_off(&[], now()), "off already");

        // On again: the rounds seen then don't count, the next ones do.
        afk.turn_on(&starts("2", [1, 2, 3, 4, 5]));
        afk.saw(&starts("2", [1, 2, 3, 4, 5, 6]), now());
        assert_eq!(afk.header(&["2".into()]).as_deref(), Some("2:1,2,3,6"));
    }

    #[test]
    fn a_spectator_copy_of_the_match_adds_nothing_twice() {
        let mut afk = Afk::default();
        afk.turn_on(&starts(KEY, [1, 2]));
        afk.saw(&starts(KEY, [1, 2, 3]), now());
        // The host moved to spectator: a new file repeats the match so far.
        assert!(!afk.saw(&starts(KEY, [1, 2, 3]), now()));
        afk.saw(&starts(KEY, [1, 2, 3, 4]), now());
        assert_eq!(
            afk.header(&[KEY.into()]).as_deref(),
            Some("482913507226:3,4")
        );
    }

    #[test]
    fn forgets_old_matches() {
        let mut afk = Afk::default();
        afk.turn_on(&[]);
        afk.saw(&starts("old", [1]), now());
        let later = now() + 10 * DAY;
        afk.saw(&starts("new", [1]), later);
        assert!(!afk.prune(later, 30 * DAY));
        assert!(afk.prune(now() + 31 * DAY, 30 * DAY));
        assert_eq!(afk.header(&["old".into()]), None);
        assert_eq!(afk.header(&["new".into()]).as_deref(), Some("new:1"));
    }

    #[test]
    fn shows_the_latest_match() {
        let mut store = AfkStore::open(PathBuf::from("unused"));
        assert_eq!(store.status(), AfkStatus::default());
        store.afk.turn_on(&[]);
        store.afk.saw(&starts("1", [2]), now());
        store.afk.saw(&starts("2", [1, 3]), now() + DAY);
        assert_eq!(
            store.status().latest,
            Some(LatestMatch {
                match_key: "2".into(),
                rounds: vec![1, 3]
            })
        );
    }

    #[test]
    fn persists_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("afk.json");
        let keep = 30 * DAY;
        let mut store = AfkStore::open(path.clone());
        assert!(store.set(true, &starts(KEY, [1]), now()).on);
        store.saw(&starts(KEY, [1, 2]), now(), keep);

        let mut store = AfkStore::open(path.clone());
        assert!(store.status().on);
        assert_eq!(
            store.header(&[KEY.into()]).as_deref(),
            Some("482913507226:2")
        );
        // What was seen when it was turned on is kept too.
        store.saw(&starts(KEY, [1, 2]), now(), keep);
        assert_eq!(
            store.header(&[KEY.into()]).as_deref(),
            Some("482913507226:2")
        );

        store.set(false, &[], now());
        let store = AfkStore::open(path);
        assert!(!store.status().on);
        assert_eq!(
            store.header(&[KEY.into()]).as_deref(),
            Some("482913507226:2")
        );
    }

    #[test]
    fn a_file_that_cant_be_read_is_off_until_the_host_turns_it_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("afk.json");
        fs::write(&path, "{ broken").unwrap();
        let mut store = AfkStore::open(path.clone());
        let status = store.status();
        assert!(!status.on);
        assert!(status.error.unwrap().contains("isn't a valid AFK record"));
        // A poll doesn't write over it.
        store.saw(&starts(KEY, [1]), now(), DAY);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ broken");
        // The host turning it on (or off) does.
        let status = store.set(true, &[], now());
        assert!(status.on);
        assert_eq!(status.error, None);
        assert!(load(&path).unwrap().on);
    }
}
