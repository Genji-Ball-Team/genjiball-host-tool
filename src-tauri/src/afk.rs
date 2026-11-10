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
//!
//! The lobby host's sit-out in the game (`log_scan::SitOut`, "Host sit-out") turns it on and off
//! too (`follow`): on when the host starts sitting out, off when the sit-out ends (they play
//! again, leave, or the match ends) or no match is played in the live log any more (`lost`: no
//! log at all, or a newest one that isn't ranked). A live log the tool can't read now, or doesn't
//! read (too large, from before uploads started), changes nothing; one uploaded as it is (the
//! lobby went quiet) is still followed. The switch still works by hand, and what happened last
//! wins: a sit-out only acts when it changes. AFK switched on by hand is the host's own: only the
//! switch turns it off, whatever the game does. The switch (`AfkStore::set`) follows the live log
//! first, as a poll does, so AFK a sit-out turned on ends where the log shows, however long ago
//! the last poll was.
//!
//! Each look at the live log (a poll, an upload's read of it, the click) carries when it began
//! (`watcher::Seen`), and one that began before the last one applied changes nothing
//! (`AfkStore::saw`), whatever it found: a read, an unreadable log or no log. The click reads the
//! log while it holds the store and takes its number once it's read, so nothing that began before
//! it can undo it, whichever is applied first. Nothing compares files or lengths: a log the game
//! shortened, deleted or replaced by an older-named one is followed as soon as it's looked at.
//!
//! Switched on by hand while the live log can't show which rounds have started (a spectator copy
//! with nothing in it yet, a log that can't be read, no log), the first read after the click
//! settles it (`Afk::settle`): every round started in it counts as started before the click. That
//! may leave out a round started between the click and that read, never count one the host played.
//!
//! A round only counts for a sit-out when the log shows the host sat out for it: past the round
//! the sit-out started after, and up to where it ended. A sit-out that started and ended between
//! two reads of the log isn't seen, and when one ended and another started between two reads,
//! the rounds of the first not read yet don't count: either way the host wasn't in those rounds'
//! `ROUND_START`, so the server never rated them for the host anyway.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::log_scan::{RoundStart, SitOut};
use crate::settings;
use crate::watcher::{Live, Observed, Seen};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Afk {
    on: bool,
    /// `matchKey` → the last round it had started when AFK was turned on. Only while it's on.
    seen: BTreeMap<String, u32>,
    /// `matchKey` → the rounds that started while AFK was on.
    matches: BTreeMap<String, Rounds>,
    /// It's on because the host sits out in the game, not by the switch.
    sit_out: bool,
    /// The sit-out going on in the live log as last followed (`None`: the host plays).
    followed: Option<SitOut>,
    /// Switched on by hand while the live log couldn't show which rounds had started: the first
    /// read of it after that counts every round in it as started then (`settle`).
    unsettled: bool,
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
    /// Turns AFK on, by hand, with `started` the rounds already started in the live log. On
    /// already: what was seen when it was turned on stays, and AFK a sit-out turned on is the
    /// host's own from now on, so only the switch turns it off. `false` when nothing changed.
    fn turn_on(&mut self, started: &[RoundStart]) -> bool {
        if self.on {
            return std::mem::take(&mut self.sit_out);
        }
        self.on = true;
        self.seen.clear();
        for start in started {
            let last = self.seen.entry(start.match_key.clone()).or_default();
            *last = (*last).max(start.round);
        }
        true
    }

    /// Turns AFK off. The rounds noted stay; the ones started in the live log by now must be
    /// noted first (`saw`). `false` when it was off.
    fn turn_off(&mut self) -> bool {
        if !self.on {
            return false;
        }
        self.on = false;
        self.sit_out = false;
        self.unsettled = false;
        self.seen.clear();
        true
    }

    /// The first read of the live log since AFK was switched on without one (`unsettled`): the
    /// rounds in `started` had all started when it was switched on, as far as can be known.
    /// `true` when that settled it.
    fn settle(&mut self, started: &[RoundStart]) -> bool {
        if !std::mem::take(&mut self.unsettled) {
            return false;
        }
        if self.on {
            for start in started {
                let last = self.seen.entry(start.match_key.clone()).or_default();
                *last = (*last).max(start.round);
            }
        }
        true
    }

    /// Notes the rounds in `started` (read from the live log) that started while AFK is on.
    /// `true` when one is new.
    fn saw(&mut self, started: &[RoundStart], now: SystemTime) -> bool {
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

    /// Follows the host's sit-outs in the live log (`sit_outs`, `Scan::sit_outs`, with `started`
    /// its rounds): turns AFK on when a sit-out starts, unless it's on already, and off when the
    /// sit-out that turned it on ends. Only a change acts, so a host who switched it by hand keeps
    /// it so until the next one. `true` when anything changed.
    fn follow(&mut self, sit_outs: &[SitOut], started: &[RoundStart], now: SystemTime) -> bool {
        let sitting = sit_outs.iter().find(|s| s.until.is_none());
        if self.followed.as_ref() == sitting {
            return false;
        }
        let before = std::mem::replace(&mut self.followed, sitting.cloned());
        if self.on && self.sit_out {
            // The sit-out that turned AFK on is over: its rounds count up to where the log shows
            // it ended. Not if the log shows another one in its place (it ended and another
            // started between two reads), as the host may have played some of the rounds since.
            let end = before.and_then(|before| {
                let same = sit_outs.iter().find(|s| {
                    s.match_key == before.match_key && s.after_round == before.after_round
                })?;
                Some((before.match_key, same.until?))
            });
            if let Some((key, end)) = end {
                let sat: Vec<RoundStart> = started
                    .iter()
                    .filter(|r| r.match_key == key && r.round <= end)
                    .cloned()
                    .collect();
                self.saw(&sat, now);
            }
            self.turn_off();
        }
        if let Some(s) = sitting {
            if !self.on {
                // The rounds up to the sit-out were played: only the ones after it count.
                let played: Vec<RoundStart> = started
                    .iter()
                    .filter(|r| r.match_key != s.match_key || r.round <= s.after_round)
                    .cloned()
                    .collect();
                self.turn_on(&played);
                self.sit_out = true;
            }
        }
        true
    }

    /// No match is played in the live log any more (`Live::Gone`: no log, or a newest one that
    /// isn't ranked): AFK that a sit-out turned on ends, without noting more rounds (none can start
    /// in a log that isn't ranked). AFK switched on or off by hand stays as it is. `true` when it
    /// ended.
    fn lost(&mut self) -> bool {
        if !(self.on && self.sit_out) {
            return false;
        }
        self.turn_off();
        self.followed = None;
        true
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
    /// On because the host sits out in the game (`SIT_OUT`), not by the switch.
    pub sit_out: bool,
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
    /// When the last observation of the live log applied began (memory only). One that began
    /// before it changes nothing.
    applied: Option<Seen>,
    /// The rounds started in the last read of the live log applied: they had started before
    /// anything that comes after it.
    read_starts: Vec<RoundStart>,
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
            applied: None,
            read_starts: Vec::new(),
        }
    }

    pub fn status(&self) -> AfkStatus {
        AfkStatus {
            on: self.afk.on,
            sit_out: self.afk.on && self.afk.sit_out,
            latest: self.afk.latest(),
            error: self.error.clone(),
        }
    }

    pub fn header(&self, keys: &[String]) -> Option<String> {
        self.afk.header(keys)
    }

    /// The host switched AFK on or off, with `live` the live log read right at the click. The
    /// only way to switch it, so the order holds: the live log is followed first, exactly as a
    /// poll does (`saw`), so a sit-out that ended in it ends the AFK it turned on where the log
    /// shows, and a round the host played since is never noted, however long ago the last poll
    /// was. Only then the switch: on, AFK is the host's own (no sit-out or lost log turns it off)
    /// from the next round start; off, the rounds started while it was on are noted already.
    /// Written at once, even over a file that couldn't be read.
    ///
    /// `live` must be read while the caller holds this store, and numbered once it's read
    /// (`Uploader::set_afk`): then it began after every observation applied before, and one that
    /// began before it changes nothing, whichever order they come in.
    ///
    /// Switched on with no read of the live log (a spectator copy with nothing in it yet, a log
    /// that can't be read, no log), which rounds have started is settled by the next read
    /// (`Afk::settle`).
    pub fn set(
        &mut self,
        on: bool,
        live: Observed<'_>,
        now: SystemTime,
        keep: Duration,
    ) -> AfkStatus {
        let applied = self.apply(live, now, keep).is_some();
        let changed = if on {
            let was = self.afk.on;
            // Already started: the rounds in the log now, and in the last read before it.
            let mut started = self.read_starts.clone();
            let read = match live.live {
                Live::Read(scan) if applied => {
                    started.extend(scan.round_starts.iter().cloned());
                    true
                }
                _ => false,
            };
            let changed = self.afk.turn_on(&started);
            if !was && !read {
                self.afk.unsettled = true;
            }
            changed
        } else {
            self.afk.turn_off()
        };
        if changed || self.unreadable {
            self.dirty = true;
            self.unreadable = false;
            self.save();
        }
        self.status()
    }

    /// The uploader looked at the live log (`Observed`): follows its sit-out, notes its new AFK
    /// rounds and forgets old matches. A log that couldn't be read this time changes nothing; no
    /// match played in it any more ends AFK that a sit-out turned on. A look that began before
    /// the last one applied changes nothing at all, whatever it found. `true` when AFK went on or
    /// off.
    pub fn saw(&mut self, live: Observed<'_>, now: SystemTime, keep: Duration) -> bool {
        self.apply(live, now, keep).unwrap_or(false)
    }

    /// `saw`, `None` when `live` began before the last observation applied.
    fn apply(&mut self, live: Observed<'_>, now: SystemTime, keep: Duration) -> Option<bool> {
        if self.applied.is_some_and(|last| live.seen <= last) {
            return None;
        }
        self.applied = Some(live.seen);
        let was = self.afk.on;
        let started = match live.live {
            Live::Read(scan) => {
                self.read_starts.clone_from(&scan.round_starts);
                scan.round_starts.as_slice()
            }
            Live::Unread | Live::Gone => &[],
        };
        // Before anything in this read counts as an AFK round.
        let settled = matches!(live.live, Live::Read(_)) && self.afk.settle(started);
        // Not over a file that couldn't be read: AFK stays off until the host switches it.
        let followed = !self.unreadable
            && match live.live {
                Live::Read(scan) => self.afk.follow(&scan.sit_outs, started, now),
                Live::Unread => false,
                Live::Gone => self.afk.lost(),
            };
        let saw = self.afk.saw(started, now);
        let pruned = self.afk.prune(now, keep);
        self.dirty |= settled || followed || saw || pruned;
        self.save();
        Some(self.afk.on != was)
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
    use crate::log_scan::{self, Scan};

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
        afk.saw(&starts("2", [1, 2, 3]), now());
        assert!(afk.turn_off());
        assert!(!afk.saw(&starts("2", [1, 2, 3, 4]), now()));
        assert_eq!(
            afk.header(&["1".into(), "2".into(), "3".into()]).as_deref(),
            Some("1:3;2:1,2,3")
        );
        // Only the matches in the file.
        assert_eq!(afk.header(&["2".into()]).as_deref(), Some("2:1,2,3"));
        assert_eq!(afk.header(&["3".into()]), None);
        assert!(!afk.turn_off(), "off already");

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

    fn sat(key: &str, after_round: u32, until: Option<u32>) -> SitOut {
        SitOut {
            match_key: key.into(),
            after_round,
            until,
        }
    }

    /// A live log read with these rounds and sit-outs.
    fn log(round_starts: Vec<RoundStart>, sit_outs: Vec<SitOut>) -> Scan {
        Scan {
            round_starts,
            sit_outs,
            ..Scan::default()
        }
    }

    /// `live` looked at now: newer than anything before.
    fn fresh(live: Live<'_>) -> Observed<'_> {
        Observed {
            seen: Seen::next(),
            live,
        }
    }

    /// `scan` read from the live log now.
    fn read(scan: &Scan) -> Observed<'_> {
        fresh(Live::Read(scan))
    }

    #[test]
    fn follows_the_host_sit_out() {
        let mut afk = Afk::default();
        // Round 1 played, the sit-out logged after it, round 2 already read with it.
        let out = [sat(KEY, 1, None)];
        assert!(afk.follow(&out, &starts(KEY, [1, 2]), now()));
        assert!(afk.on && afk.sit_out);
        afk.saw(&starts(KEY, [1, 2]), now());
        // The same sit-out read again changes nothing.
        assert!(!afk.follow(&out, &starts(KEY, [1, 2, 3]), now()));
        afk.saw(&starts(KEY, [1, 2, 3]), now());
        // Playing from round 5: round 4 started before the `0`, round 5 is played.
        let ended = [sat(KEY, 1, Some(4))];
        assert!(afk.follow(&ended, &starts(KEY, 1..=5), now()));
        assert!(!afk.on && !afk.sit_out);
        afk.saw(&starts(KEY, 1..=5), now());
        assert_eq!(
            afk.header(&[KEY.into()]).as_deref(),
            Some("482913507226:2,3,4")
        );
        // An end read again doesn't turn it off a second time, nor does a host who plays.
        assert!(!afk.follow(&ended, &starts(KEY, 1..=6), now()));
        assert!(!afk.follow(&[], &starts(KEY, 1..=6), now()));
    }

    #[test]
    fn a_new_match_ends_the_sit_out() {
        let mut afk = Afk::default();
        afk.follow(&[sat("1", 1, None)], &starts("1", [1]), now());
        // The match ended and the next one is read at once, with its round 1.
        let mut started = starts("1", [1, 2, 3]);
        started.extend(starts("2", [1]));
        assert!(afk.follow(&[sat("1", 1, Some(3))], &started, now()));
        assert!(!afk.on);
        afk.saw(&started, now());
        assert_eq!(afk.header(&["1".into()]).as_deref(), Some("1:2,3"));
        assert_eq!(afk.header(&["2".into()]), None, "the host plays round 1");
    }

    #[test]
    fn a_sit_out_in_place_of_the_one_followed_counts_only_what_the_log_shows() {
        let mut afk = Afk::default();
        afk.follow(&[sat(KEY, 1, None)], &starts(KEY, [1, 2]), now());
        afk.saw(&starts(KEY, [1, 2]), now());
        // Between two reads: played again from round 4, and sits out again after round 5.
        let started = starts(KEY, 1..=6);
        assert!(afk.follow(&[sat(KEY, 5, None)], &started, now()));
        assert!(afk.on && afk.sit_out);
        afk.saw(&started, now());
        // Round 3 was sat out too, but where the first sit-out ended isn't known: 4 and 5 were
        // played, so none of them counts.
        assert_eq!(
            afk.header(&[KEY.into()]).as_deref(),
            Some("482913507226:2,6")
        );

        // The same with the second one over too.
        let mut afk = Afk::default();
        afk.follow(&[sat(KEY, 1, None)], &starts(KEY, [1, 2]), now());
        afk.saw(&starts(KEY, [1, 2]), now());
        assert!(afk.follow(&[sat(KEY, 5, Some(6))], &starts(KEY, 1..=7), now()));
        assert!(!afk.on);
        afk.saw(&starts(KEY, 1..=7), now());
        assert_eq!(afk.header(&[KEY.into()]).as_deref(), Some("482913507226:2"));
    }

    #[test]
    fn the_switch_still_works_by_hand() {
        // On by hand: a sit-out ending, or the log going quiet, leaves it on.
        let mut afk = Afk::default();
        afk.turn_on(&starts(KEY, [1]));
        assert!(afk.follow(&[sat(KEY, 1, None)], &starts(KEY, [1]), now()));
        assert!(afk.on && !afk.sit_out);
        afk.follow(&[sat(KEY, 1, Some(2))], &starts(KEY, [1, 2]), now());
        assert!(afk.on);
        assert!(!afk.lost());
        assert!(afk.on);

        // Off by hand while sitting out: it stays off until the next sit-out, even if the log
        // goes quiet and then on with the same sit-out.
        let mut afk = Afk::default();
        let out = [sat(KEY, 1, None)];
        afk.follow(&out, &starts(KEY, [1]), now());
        afk.turn_off();
        assert!(!afk.sit_out);
        assert!(!afk.follow(&out, &starts(KEY, [1, 2, 3]), now()));
        assert!(!afk.lost());
        assert!(!afk.follow(&out, &starts(KEY, [1, 2, 3]), now()));
        assert!(!afk.on);
        afk.follow(&[sat(KEY, 1, Some(3))], &starts(KEY, 1..=3), now());
        assert!(afk.follow(&[sat(KEY, 4, None)], &starts(KEY, 1..=4), now()));
        assert!(afk.on && afk.sit_out);
    }

    #[test]
    fn a_sit_out_ends_when_no_match_is_played_in_the_live_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("afk.json");
        let mut store = AfkStore::open(path.clone());
        let out = log(starts(KEY, [1, 2]), vec![sat(KEY, 1, None)]);
        assert!(store.saw(read(&out), now(), DAY));
        assert!(store.status().sit_out);
        // A poll that couldn't read the log changes nothing, nor does reading it again.
        assert!(!store.saw(fresh(Live::Unread), now(), DAY));
        assert!(store.status().sit_out);
        assert!(!store.saw(read(&out), now(), DAY));
        assert!(store.status().sit_out);

        // Kept across a restart while the log can't be read yet, or still says so.
        let mut store = AfkStore::open(path.clone());
        assert!(store.status().sit_out);
        assert!(!store.saw(fresh(Live::Unread), now(), DAY));
        assert!(!store.saw(read(&out), now(), DAY));
        assert!(store.status().sit_out);

        // The match stopped without `MATCH_END`, and the next game started a log that isn't ranked.
        assert!(store.saw(fresh(Live::Gone), now(), DAY));
        assert!(!store.status().on);
        assert_eq!(
            store.header(&[KEY.into()]).as_deref(),
            Some("482913507226:2")
        );
        assert!(!store.saw(fresh(Live::Gone), now(), DAY));
        // The match goes on in it after all, the host still sitting out: followed again.
        let more = log(starts(KEY, [1, 2, 3]), vec![sat(KEY, 1, None)]);
        assert!(store.saw(read(&more), now(), DAY));
        assert!(store.status().sit_out);
        assert_eq!(
            store.header(&[KEY.into()]).as_deref(),
            Some("482913507226:2,3")
        );

        // A restart with no match played in the live log (no log, or a newer one that isn't ranked).
        let mut store = AfkStore::open(path.clone());
        assert!(store.status().sit_out);
        assert!(store.saw(fresh(Live::Gone), now(), DAY));
        assert!(!AfkStore::open(path).status().on);

        // AFK on by hand stays on.
        let mut store = AfkStore::open(dir.path().join("hand.json"));
        store.set(true, fresh(Live::Gone), now(), DAY);
        assert!(!store.saw(fresh(Live::Gone), now(), DAY));
        assert!(store.status().on);
    }

    /// The lobby host (1) sits out after round 1 and moves to spectator after round 3. Ash (2),
    /// the game's host while they're away, sits out in round 4. The lobby host comes back (5)
    /// and plays rounds 5 and 6. In the next match they sit out rounds 2 and 3.
    const TRANSFER: &str = "GBR|1|2|1.3.3R|111
MATCH_START|1|workshop-island-night|Standard|0|
JOIN|1|1|Host|1
JOIN|1|2|Ash|
JOIN|1|3|Kenzo|
ROUND_START|2|1|1,2,3
ROUND_END|3|1|1|WIN
SIT_OUT|4|1|1
ROUND_START|5|2|2,3
ROUND_END|6|2|2|WIN
ROUND_START|7|3|2,3
ROUND_END|8|3|3|WIN
LEAVE|9|1
JOIN|10|4|Mochi|
SIT_OUT|11|2|1
ROUND_START|12|4|3,4
ROUND_END|13|4|3|WIN
JOIN|14|5|Host|1
ROUND_START|15|5|2,3,4,5
ROUND_END|16|5|5|WIN
ROUND_START|17|6|2,3,4,5
ROUND_END|18|6|5|WIN
MATCH_END|19|TIME
GBR|20|2|1.3.3R|222
JOIN|20|1|Host|1
JOIN|20|2|Ash|
JOIN|20|3|Kenzo|
ROUND_START|21|1|1,2,3
ROUND_END|22|1|1|WIN
SIT_OUT|23|1|1
ROUND_START|24|2|2,3
ROUND_END|25|2|2|WIN
ROUND_START|26|3|2,3
ROUND_END|27|3|3|WIN
SIT_OUT|28|1|0
ROUND_START|29|4|1,2,3
ROUND_END|30|4|1|WIN
MATCH_END|31|TIME
";

    /// The complete lines of `TRANSFER` up to its `n`th.
    fn transfer_until(n: usize) -> String {
        TRANSFER.split_inclusive('\n').take(n).collect()
    }

    /// The uploader reading the live log with `text` in it.
    fn read_log(store: &mut AfkStore, text: &str) {
        let scan = log_scan::scan(text);
        store.saw(fresh(Live::Read(&scan)), now(), DAY);
    }

    #[test]
    fn notes_only_rounds_the_lobby_host_sat_out() {
        let keys = ["111".to_string(), "222".to_string()];
        let lines = TRANSFER.lines().count();
        // Read after every line (a spectator copy repeats the same lines in a new file).
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        for n in 1..=lines {
            read_log(&mut store, &transfer_until(n));
            let line = TRANSFER.lines().nth(n - 1).unwrap();
            if line == "JOIN|14|5|Host|1" {
                // Off since the `LEAVE`, and Ash's `SIT_OUT` didn't turn it on.
                assert!(!store.status().on, "{line}");
            }
        }
        assert!(!store.status().on);
        assert_eq!(store.header(&keys).as_deref(), Some("111:2,3;222:2,3"));

        // Read only now and then: never a round the host played.
        for first in 1..=lines {
            for second in first..=lines {
                let dir = tempfile::tempdir().unwrap();
                let mut store = AfkStore::open(dir.path().join("afk.json"));
                for n in [first, second, lines] {
                    read_log(&mut store, &transfer_until(n));
                }
                assert!(!store.status().on);
                let header = store.header(&keys).unwrap_or_default();
                for part in header.split(';').filter(|p| !p.is_empty()) {
                    let (_, rounds) = part.split_once(':').unwrap();
                    for round in rounds.split(',') {
                        assert!(["2", "3"].contains(&round), "{first} {second}: {header}");
                    }
                }
            }
        }
    }

    /// The number of `TRANSFER`'s line `line` (from 1).
    fn transfer_line(line: &str) -> usize {
        TRANSFER.lines().position(|l| l == line).unwrap() + 1
    }

    /// The host switching AFK with `TRANSFER`'s first `n` lines in the live log, read at the click.
    fn switch(store: &mut AfkStore, on: bool, n: usize) -> AfkStatus {
        let text = transfer_until(n);
        let scan = log_scan::live_scan(&text);
        store.set(on, fresh(Live::Read(&scan)), now(), DAY)
    }

    /// Reads `TRANSFER` from line `from` to the end a line at a time, then finds the live log
    /// gone and back again: AFK switched on by hand stays on all along.
    fn read_rest_on_by_hand(store: &mut AfkStore, from: usize) {
        for n in from..=TRANSFER.lines().count() {
            read_log(store, &transfer_until(n));
            let status = store.status();
            assert!(status.on && !status.sit_out, "line {n}");
        }
        for live in [Live::Gone, Live::Unread, Live::Gone] {
            store.saw(fresh(live), now(), DAY);
            assert!(store.status().on && !store.status().sit_out);
        }
        read_log(store, TRANSFER);
        assert!(store.status().on && !store.status().sit_out);
    }

    #[test]
    fn switched_off_after_an_unread_sit_out_end_never_notes_a_played_round() {
        // Following the host's sit-out in match 222: AFK on by itself, round 2 read.
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        read_log(
            &mut store,
            &transfer_until(transfer_line("ROUND_START|24|2|2,3")),
        );
        assert!(store.status().sit_out);
        // Before the next poll, the host plays again from round 4 and round 4 starts. Then they
        // switch AFK off in the tool.
        let played = transfer_line("ROUND_START|29|4|1,2,3");
        let status = switch(&mut store, false, played);
        assert!(!status.on && !status.sit_out);
        let keys = ["222".to_string()];
        assert_eq!(store.header(&keys).as_deref(), Some("222:2,3"));
        read_log(&mut store, TRANSFER);
        assert_eq!(store.header(&keys).as_deref(), Some("222:2,3"));

        // The same as if a poll had read it just before the click.
        let mut polled = AfkStore::open(dir.path().join("polled.json"));
        read_log(
            &mut polled,
            &transfer_until(transfer_line("ROUND_START|24|2|2,3")),
        );
        read_log(&mut polled, &transfer_until(played));
        assert!(!polled.status().on);
        switch(&mut polled, false, played);
        assert_eq!(polled.header(&keys), store.header(&keys));
    }

    #[test]
    fn on_by_hand_stays_on_whatever_happens_in_the_game() {
        // Switched on in round 1 of match 111: the sit-out starting and ending in the game, the
        // host's `LEAVE`, the next match and the live log going away leave it on, and every
        // round that started since is an AFK round.
        let keys = ["111".to_string(), "222".to_string()];
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        let first = transfer_line("ROUND_START|2|1|1,2,3");
        read_log(&mut store, &transfer_until(first));
        assert!(switch(&mut store, true, first).on);
        read_rest_on_by_hand(&mut store, first);
        assert_eq!(
            store.header(&keys).as_deref(),
            Some("111:2,3,4,5,6;222:1,2,3,4")
        );
        // Across a restart too, with no live log.
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        assert!(!store.saw(fresh(Live::Gone), now(), DAY));
        assert!(store.status().on && !store.status().sit_out);
        // Only the switch turns it off.
        assert!(!switch(&mut store, false, first).on);
    }

    #[test]
    fn on_right_after_an_unread_sit_out_start_is_on_by_hand() {
        // The host sat out in the game, then switched AFK on before a poll read it.
        let keys = ["111".to_string(), "222".to_string()];
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        read_log(
            &mut store,
            &transfer_until(transfer_line("ROUND_END|3|1|1|WIN")),
        );
        let out = transfer_line("SIT_OUT|4|1|1");
        let status = switch(&mut store, true, out);
        assert!(status.on && !status.sit_out);
        read_rest_on_by_hand(&mut store, out);
        assert_eq!(
            store.header(&keys).as_deref(),
            Some("111:2,3,4,5,6;222:1,2,3,4")
        );
    }

    #[test]
    fn on_right_after_an_unread_sit_out_end_is_on_by_hand() {
        // AFK followed the sit-out in match 222; the host pressed to play again in the game, then
        // switched AFK on before a poll read it. On as AFK still on by the sit-out, too.
        let keys = ["222".to_string()];
        for read_until in ["ROUND_START|24|2|2,3", "SIT_OUT|23|1|1"] {
            let dir = tempfile::tempdir().unwrap();
            let mut store = AfkStore::open(dir.path().join("afk.json"));
            read_log(&mut store, &transfer_until(transfer_line(read_until)));
            assert!(store.status().sit_out);
            let back = transfer_line("SIT_OUT|28|1|0");
            let status = switch(&mut store, true, back);
            assert!(status.on && !status.sit_out, "{read_until}");
            read_rest_on_by_hand(&mut store, back);
            assert_eq!(store.header(&keys).as_deref(), Some("222:2,3,4"));
        }

        // Switched on while still sitting out: the host's own from then on.
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        let out = transfer_line("SIT_OUT|23|1|1");
        read_log(&mut store, &transfer_until(out));
        let status = switch(&mut store, true, out);
        assert!(status.on && !status.sit_out);
        read_rest_on_by_hand(&mut store, out);
        assert_eq!(store.header(&keys).as_deref(), Some("222:2,3,4"));
    }

    /// A poll's read of `TRANSFER`'s first `n` lines, begun at `seen`.
    fn poll_at(store: &mut AfkStore, seen: Seen, n: usize) -> bool {
        let text = transfer_until(n);
        let scan = log_scan::scan(&text);
        let live = Observed {
            seen,
            live: Live::Read(&scan),
        };
        store.saw(live, now(), DAY)
    }

    /// A poll's read of `TRANSFER`'s first `n` lines, begun now.
    fn poll(store: &mut AfkStore, n: usize) -> bool {
        poll_at(store, Seen::next(), n)
    }

    #[test]
    fn a_poll_that_began_before_the_click_cant_overtake_it() {
        // AFK follows the host's sit-out in match 222. The host moves to spectator: the game
        // starts a new file, repeating the match, and the host comes back and plays round 4. A
        // poll began reading the old file (sit-out still on) just before the host switched AFK
        // off, and applies its read after the click, or before it.
        let keys = ["222".to_string()];
        let sat = transfer_line("ROUND_START|24|2|2,3");
        let polled = transfer_line("ROUND_START|26|3|2,3");
        let played = transfer_line("ROUND_START|29|4|1,2,3");
        let outcomes: Vec<(bool, Option<String>)> = [true, false]
            .into_iter()
            .map(|click_first| {
                let dir = tempfile::tempdir().unwrap();
                let mut store = AfkStore::open(dir.path().join("afk.json"));
                poll(&mut store, sat);
                assert!(store.status().sit_out);
                let early = Seen::next();
                if click_first {
                    switch(&mut store, false, played);
                    // Began before the click: changes nothing. AFK doesn't come back on.
                    assert!(!poll_at(&mut store, early, polled));
                } else {
                    poll_at(&mut store, early, polled);
                    switch(&mut store, false, played);
                }
                assert!(!store.status().on, "click first: {click_first}");
                // The next poll reads the new file: nothing more.
                poll(&mut store, TRANSFER.lines().count());
                (store.status().on, store.header(&keys))
            })
            .collect();
        assert_eq!(outcomes[0], (false, Some("222:2,3".to_string())));
        assert_eq!(outcomes[0], outcomes[1]);

        // Switched on by hand, with a poll that began before applied after: still the host's,
        // and the rounds already started at the click aren't AFK rounds.
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        let first = transfer_line("ROUND_START|21|1|1,2,3");
        poll(&mut store, first);
        let early = Seen::next();
        let status = switch(&mut store, true, first);
        assert!(status.on && !status.sit_out);
        assert!(!poll_at(&mut store, early, first));
        assert!(store.status().on && !store.status().sit_out);
        poll(&mut store, played);
        assert!(store.status().on && !store.status().sit_out);
        assert_eq!(store.header(&keys).as_deref(), Some("222:2,3,4"));
    }

    #[test]
    fn an_earlier_read_applied_late_changes_nothing() {
        // Read to the sit-out's end (AFK off), then a read begun before it applied late.
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        poll(&mut store, transfer_line("ROUND_START|24|2|2,3"));
        assert!(store.status().sit_out);
        let early = Seen::next();
        poll(&mut store, transfer_line("ROUND_START|29|4|1,2,3"));
        assert!(!store.status().on);
        assert!(!poll_at(
            &mut store,
            early,
            transfer_line("ROUND_START|26|3|2,3")
        ));
        assert!(!store.status().on);
        // The same read again, begun later, is fine (nothing new in it).
        assert!(!poll(&mut store, transfer_line("ROUND_START|29|4|1,2,3")));
        assert_eq!(
            store.header(&["222".to_string()]).as_deref(),
            Some("222:2,3")
        );
    }

    #[test]
    fn a_gone_log_found_before_a_newer_look_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let sitting = transfer_line("ROUND_START|24|2|2,3");

        // A poll found no ranked log and is slow to apply it; the game writes the log with the
        // host sitting out, and a later poll reads it first.
        let mut store = AfkStore::open(dir.path().join("read.json"));
        let early = Seen::next();
        poll(&mut store, sitting);
        assert!(store.status().sit_out);
        let gone = Observed {
            seen: early,
            live: Live::Gone,
        };
        assert!(!store.saw(gone, now(), DAY));
        assert!(store.status().sit_out);

        // The same with the host's click in between: AFK switched on by hand while sitting out.
        let mut store = AfkStore::open(dir.path().join("click.json"));
        let early = Seen::next();
        assert!(switch(&mut store, true, sitting).on);
        assert!(!store.saw(gone_at(early), now(), DAY));
        assert!(store.status().on);
        // Followed by the click: the poll's Gone came too late to undo it, so the next read
        // of the sit-out going on is no change.
        assert!(!poll(&mut store, sitting));
        assert!(store.status().on);

        // AFK on by the sit-out, the click switching it off: a stale Gone or a stale read of the
        // sit-out doesn't bring it back.
        let mut store = AfkStore::open(dir.path().join("off.json"));
        poll(&mut store, sitting);
        let early = Seen::next();
        assert!(!switch(&mut store, false, sitting).on);
        assert!(!store.saw(gone_at(early), now(), DAY));
        assert!(!poll_at(&mut store, early, sitting));
        assert!(!store.status().on);
    }

    fn gone_at(seen: Seen) -> Observed<'static> {
        Observed {
            seen,
            live: Live::Gone,
        }
    }

    #[test]
    fn a_read_begun_before_a_newer_gone_changes_nothing() {
        // Sit-out AFK on. A poll reads the log (still sitting out); before it's applied, a newer
        // one finds no ranked log any more (the next game isn't ranked): AFK ends, and the late
        // read doesn't turn it back on.
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        let sitting = transfer_line("ROUND_START|24|2|2,3");
        poll(&mut store, sitting);
        assert!(store.status().sit_out);
        let early = Seen::next();
        assert!(store.saw(fresh(Live::Gone), now(), DAY));
        assert!(!store.status().on);
        assert!(!poll_at(
            &mut store,
            early,
            transfer_line("ROUND_START|26|3|2,3")
        ));
        assert!(!store.status().on);
        assert_eq!(store.header(&["222".to_string()]).as_deref(), Some("222:2"));
    }

    #[test]
    fn a_shorter_or_older_named_live_log_is_still_followed() {
        // Followed a long log with the host sitting out in match 222. The newest file is now a
        // shorter one (the followed file deleted, a shorter copy, or a future-dated file gone so
        // an older-named one is newest), in which the host plays a new match: followed at once.
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        poll(&mut store, transfer_line("ROUND_START|26|3|2,3"));
        assert!(store.status().sit_out);
        let short = "GBR|1|2|1.3.3R|333\nJOIN|1|1|Host|1\nJOIN|1|2|Ash|\nROUND_START|2|1|1,2\n";
        assert!(short.len() < transfer_until(transfer_line("ROUND_START|26|3|2,3")).len());
        let scan = log_scan::scan(short);
        assert!(store.saw(read(&scan), now(), DAY));
        assert!(!store.status().on);
        // And a sit-out in it is followed too.
        let out = format!("{short}ROUND_END|3|1|1|WIN\nSIT_OUT|4|1|1\nROUND_START|5|2|2\n");
        let scan = log_scan::scan(&out);
        assert!(store.saw(read(&scan), now(), DAY));
        assert!(store.status().sit_out);
        assert_eq!(store.header(&["333".to_string()]).as_deref(), Some("333:2"));
    }

    #[test]
    fn switched_on_with_the_new_file_still_empty() {
        // Round 1 of match 111 read; round 2 starts, the host moves to spectator and switches AFK
        // on while the new file has nothing in it yet. Once it's written, round 2 had started
        // before the click as far as can be known: only the rounds after count.
        let keys = ["111".to_string()];
        let round_2 = transfer_line("ROUND_START|5|2|2,3");
        let round_3 = transfer_line("ROUND_START|7|3|2,3");
        for live in [Live::Unread, Live::Gone] {
            let dir = tempfile::tempdir().unwrap();
            let mut store = AfkStore::open(dir.path().join("afk.json"));
            poll(&mut store, transfer_line("ROUND_START|2|1|1,2,3"));
            let status = store.set(true, fresh(live), now(), DAY);
            assert!(status.on && !status.sit_out, "{live:?}");
            // Still empty, or unreadable: nothing settled yet.
            store.saw(fresh(Live::Unread), now(), DAY);
            poll(&mut store, round_2);
            assert!(store.status().on && !store.status().sit_out);
            assert_eq!(store.header(&keys), None, "{live:?}");
            poll(&mut store, round_3);
            assert_eq!(store.header(&keys).as_deref(), Some("111:3"), "{live:?}");
        }

        // Across a restart before the copy is written: the first read after it settles it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("afk.json");
        let mut store = AfkStore::open(path.clone());
        poll(&mut store, transfer_line("ROUND_START|2|1|1,2,3"));
        store.set(true, fresh(Live::Unread), now(), DAY);
        let mut store = AfkStore::open(path);
        assert!(store.status().on);
        poll(&mut store, round_2);
        assert_eq!(store.header(&keys), None);
        poll(&mut store, round_3);
        assert_eq!(store.header(&keys).as_deref(), Some("111:3"));

        // Switched off again before the copy is written: nothing is left to settle, so the
        // host's sit-out in it (after round 1) turns AFK on as usual and its round 2 counts.
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        poll(&mut store, transfer_line("ROUND_START|2|1|1,2,3"));
        store.set(true, fresh(Live::Unread), now(), DAY);
        assert!(!store.set(false, fresh(Live::Unread), now(), DAY).on);
        assert!(!store.afk.unsettled);
        poll(&mut store, round_2);
        assert!(store.status().sit_out);
        assert_eq!(store.header(&keys).as_deref(), Some("111:2"));

        // A new match by the first read: its rounds in it count as started too, the next don't.
        let dir = tempfile::tempdir().unwrap();
        let mut store = AfkStore::open(dir.path().join("afk.json"));
        poll(&mut store, transfer_line("ROUND_START|17|6|2,3,4,5"));
        store.set(true, fresh(Live::Gone), now(), DAY);
        poll(&mut store, transfer_line("ROUND_START|21|1|1,2,3"));
        assert_eq!(store.header(&["111".into(), "222".into()]), None);
        poll(&mut store, transfer_line("ROUND_START|24|2|2,3"));
        assert_eq!(
            store.header(&["111".into(), "222".into()]).as_deref(),
            Some("222:2")
        );
        assert!(store.status().on && !store.status().sit_out);
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
        assert!(
            store
                .set(true, read(&log(starts(KEY, [1]), vec![])), now(), keep)
                .on
        );
        store.saw(read(&log(starts(KEY, [1, 2]), vec![])), now(), keep);

        let mut store = AfkStore::open(path.clone());
        assert!(store.status().on);
        assert_eq!(
            store.header(&[KEY.into()]).as_deref(),
            Some("482913507226:2")
        );
        // What was seen when it was turned on is kept too.
        store.saw(read(&log(starts(KEY, [1, 2]), vec![])), now(), keep);
        assert_eq!(
            store.header(&[KEY.into()]).as_deref(),
            Some("482913507226:2")
        );

        store.set(false, fresh(Live::Gone), now(), keep);
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
        store.saw(read(&log(starts(KEY, [1]), vec![])), now(), DAY);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ broken");
        // The host turning it on (or off) does.
        let status = store.set(true, fresh(Live::Gone), now(), DAY);
        assert!(status.on);
        assert_eq!(status.error, None);
        assert!(load(&path).unwrap().on);
    }
}
