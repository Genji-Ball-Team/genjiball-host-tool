//! Live lobby (#6), the logic: whether a ranked match is being played in the live log and with how
//! many players, and when to send a heartbeat, close the lobby or wait (genjiball-ranked
//! `docs/api.md`, "Live lobby"). The loop and the requests are in `live_lobby.rs` and `server.rs`.
//!
//! A match is being played from its `GBR` and `MATCH_START` to its `MATCH_END`, in the newest log
//! (the one the game writes), while that log keeps growing (`QUIET_SECS`): once it stops, the host
//! closed the lobby, moved to spectator without a new file yet, or the game closed or crashed. A
//! match with an `UNRANKED` line won't count (GenjiBall-CE `docs/ranked-log.md`), so it isn't
//! listed. The players are the ids that have a `JOIN` and no `LEAVE` since.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::{config, log_scan};

/// What the live log says about the match in it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Live {
    /// No ranked match is being played: none started, the last one ended, or the log stopped
    /// growing.
    #[default]
    Idle,
    /// A match is being played, but the game marked it `UNRANKED`: it won't count.
    Unranked,
    /// A ranked match is being played, with this many players in it now.
    Playing { players: u32 },
}

/// The match the complete lines of `text` end in (`log_scan::complete_lines`), regardless of
/// whether the log is still growing.
pub fn read(text: &str) -> Live {
    #[derive(Default)]
    struct Match<'a> {
        started: bool,
        unranked: bool,
        players: BTreeSet<&'a str>,
    }
    let complete = &text[..text.rfind('\n').map_or(0, |i| i + 1)];
    let mut current: Option<Match> = None;
    for line in complete.lines().map(log_scan::event) {
        let mut fields = line.split('|');
        let kind = fields.next().unwrap_or_default();
        // `TYPE|time|...`: the fields after the time.
        let id = fields.nth(1).filter(|id| !id.is_empty());
        match (kind, current.as_mut()) {
            ("GBR", _) => current = Some(Match::default()),
            ("MATCH_END", _) => current = None,
            ("MATCH_START", Some(m)) => m.started = true,
            ("UNRANKED", Some(m)) => m.unranked = true,
            ("JOIN", Some(m)) => m.players.extend(id),
            ("LEAVE", Some(m)) => {
                if let Some(id) = id {
                    m.players.remove(id);
                }
            }
            _ => {}
        }
    }
    match current {
        Some(m) if m.started && m.unranked => Live::Unranked,
        Some(m) if m.started => Live::Playing {
            players: u32::try_from(m.players.len())
                .unwrap_or(u32::MAX)
                .min(config::LOBBY_PLAYERS_MAX),
        },
        _ => Live::Idle,
    }
}

/// The newest log file as a poll found it.
#[derive(Debug, Clone, Copy)]
pub struct LogFile<'a> {
    pub name: &'a str,
    pub size: u64,
    /// How long ago it was last written, by its modified time.
    pub age: Duration,
}

/// The live log across polls: reads it only when it grew, and knows when it last did.
#[derive(Debug, Default)]
pub struct Watch {
    /// The newest log's name and size at the last poll.
    file: Option<(String, u64)>,
    changed_at: Option<Instant>,
    found: Live,
}

impl Watch {
    /// What's being played at `now`, with `newest` the newest log in the folder and `read` its
    /// text (`None` when it couldn't be read: locked for a moment while the game writes it, tried
    /// again at the next poll). A log quiet for `quiet` is `Idle`, whatever it holds.
    pub fn look(
        &mut self,
        newest: Option<LogFile>,
        now: Instant,
        quiet: Duration,
        read: impl FnOnce() -> Option<String>,
    ) -> Live {
        let Some(file) = newest else {
            *self = Watch::default();
            return Live::Idle;
        };
        let known = self
            .file
            .as_ref()
            .map(|(name, size)| (name.as_str(), *size));
        if known != Some((file.name, file.size)) {
            let first_seen = known.is_none_or(|(name, _)| name != file.name);
            // A file first seen has been quiet since it was last written: no need to read it.
            let quiet_since_written = first_seen && file.age >= quiet;
            let found = if quiet_since_written {
                Some(Live::Idle)
            } else {
                read().map(|text| self::read(&text))
            };
            let Some(found) = found else {
                return self.now(now, quiet);
            };
            self.file = Some((file.name.to_string(), file.size));
            self.changed_at = Some(if first_seen {
                now.checked_sub(file.age).unwrap_or(now)
            } else {
                now
            });
            self.found = found;
        }
        self.now(now, quiet)
    }

    fn now(&self, now: Instant, quiet: Duration) -> Live {
        match self.changed_at {
            Some(at) if now.saturating_duration_since(at) < quiet => self.found.clone(),
            _ => Live::Idle,
        }
    }
}

/// What a heartbeat says: the lobby's players, its name and the region it's sent as (`None`: the
/// host's home region).
#[derive(Debug, Clone, PartialEq)]
pub struct Heartbeat {
    pub players: u32,
    pub name: Option<String>,
    pub region: Option<String>,
}

/// What to do now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Heartbeat,
    Close,
    Wait,
}

/// When heartbeats and closes go, from what the server last took.
#[derive(Debug, Default)]
pub struct Schedule {
    /// The last heartbeat the server took, while the lobby is listed as far as the tool knows.
    listed: Option<Listed>,
    /// When the server last took a heartbeat or a close: the next heartbeat waits
    /// `LOBBY_HEARTBEAT_MIN_SECS` from it.
    last_write: Option<Instant>,
    /// Nothing is sent before this: the server said to wait (`429`), or a request failed.
    hold_until: Option<Instant>,
}

#[derive(Debug)]
struct Listed {
    sent: Heartbeat,
    at: Instant,
    every: Duration,
    ttl: Duration,
}

impl Schedule {
    /// What to do at `now`, with `want` the heartbeat to send while the lobby should be listed
    /// (`None`: it shouldn't be).
    pub fn step(&mut self, want: Option<&Heartbeat>, now: Instant) -> Step {
        let min_gap = Duration::from_secs(config::LOBBY_HEARTBEAT_MIN_SECS);
        // Past its TTL the server dropped it too: nothing to close or refresh.
        if self
            .listed
            .as_ref()
            .is_some_and(|l| now.saturating_duration_since(l.at) >= l.ttl)
        {
            self.listed = None;
        }
        if self.hold_until.is_some_and(|until| now < until) {
            return Step::Wait;
        }
        let gap_over = self
            .last_write
            .is_none_or(|at| now.saturating_duration_since(at) >= min_gap);
        match (want, &self.listed) {
            (None, None) => Step::Wait,
            (None, Some(_)) => Step::Close,
            (Some(_), None) if gap_over => Step::Heartbeat,
            // Refreshed when it's due, and as soon as the server takes one when it changed.
            (Some(want), Some(listed))
                if gap_over
                    && (now.saturating_duration_since(listed.at) >= listed.every
                        || *want != listed.sent) =>
            {
                Step::Heartbeat
            }
            _ => Step::Wait,
        }
    }

    /// The server took `sent` at `now`, and said when to send the next one (`every`) and how long
    /// it lists the lobby without one (`ttl`). An interval shorter than the server's rate limit
    /// waits for it.
    pub fn listed(&mut self, sent: Heartbeat, now: Instant, every: Duration, ttl: Duration) {
        let min_gap = Duration::from_secs(config::LOBBY_HEARTBEAT_MIN_SECS);
        self.listed = Some(Listed {
            sent,
            at: now,
            every: every.max(min_gap),
            ttl,
        });
        self.last_write = Some(now);
        self.hold_until = None;
    }

    /// The server took the lobby off the list at `now`.
    pub fn closed(&mut self, now: Instant) {
        self.listed = None;
        self.last_write = Some(now);
        self.hold_until = None;
    }

    /// Sends nothing before `until`: a `429`'s `Retry-After`, or a wait after a failed request.
    pub fn hold(&mut self, until: Instant) {
        self.hold_until = Some(until);
    }

    /// The host changed a setting: what was held for the old ones goes now.
    pub fn release(&mut self) {
        self.hold_until = None;
    }

    /// The lobby isn't listed with this server and token any more (another server or token is
    /// used now, or the server turned the token down): start afresh.
    pub fn forget(&mut self) {
        *self = Schedule::default();
    }

    /// Whether the lobby is listed, as far as the tool knows.
    pub fn is_listed(&self) -> bool {
        self.listed.is_some()
    }
}

/// The heartbeat interval and TTL from a heartbeat's answer, or the defaults where it has none.
pub fn timing(heartbeat_secs: Option<u64>, ttl_secs: Option<u64>) -> (Duration, Duration) {
    (
        Duration::from_secs(heartbeat_secs.unwrap_or(config::LOBBY_HEARTBEAT_SECS)),
        Duration::from_secs(ttl_secs.unwrap_or(config::LOBBY_TTL_SECS)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../tests/fixtures/ranked-log-example.txt");
    const QUIET: Duration = Duration::from_secs(config::QUIET_SECS.default);
    const MIN_GAP: Duration = Duration::from_secs(config::LOBBY_HEARTBEAT_MIN_SECS);
    const EVERY: Duration = Duration::from_secs(config::LOBBY_HEARTBEAT_SECS);
    const TTL: Duration = Duration::from_secs(config::LOBBY_TTL_SECS);

    /// The example log up to (not including) the line holding `marker`.
    fn until(marker: &str) -> String {
        let at = EXAMPLE.find(marker).unwrap();
        EXAMPLE[..EXAMPLE[..at].rfind('\n').unwrap() + 1].to_string()
    }

    fn playing(players: u32) -> Live {
        Live::Playing { players }
    }

    #[test]
    fn a_match_is_played_from_its_start_to_its_end() {
        assert_eq!(read(""), Live::Idle);
        // `GBR` without `MATCH_START` yet.
        assert_eq!(read(&until("MATCH_START|")), Live::Idle);
        // Started, before the players' `JOIN`s.
        assert_eq!(read(&until("JOIN|2.38|1|")), playing(0));
        // Five joined; Tidal (2) left in round 1; Nova (6) joined after it.
        assert_eq!(read(&until("ROUND_START|")), playing(5));
        assert_eq!(read(&until("LEAVE|36.50|2")), playing(5));
        assert_eq!(read(&until("JOIN|45.02|6|Nova")), playing(4));
        assert_eq!(read(&until("MATCH_END|")), playing(5));
        assert_eq!(read(EXAMPLE), Live::Idle);
    }

    #[test]
    fn counts_ids_not_names() {
        // Two players named Ghost are two players; a player who leaves and comes back has a new id.
        let text = "GBR|1|1|1.3.3R|1\nMATCH_START|1|workshop-island-night|Default|0|\n\
                    JOIN|1|4|Ghost\nJOIN|1|5|Ghost\nLEAVE|9|4\n[00:00:10] JOIN|10|7|Ghost|1\n\
                    LEAVE|11|99\nLEAVE|12|\n";
        assert_eq!(read(text), playing(2));
    }

    #[test]
    fn only_complete_lines_count() {
        let started =
            "GBR|1|1|1.3.3R|1\nMATCH_START|1|workshop-island-night|Default|0|\nJOIN|1|1|A\n";
        assert_eq!(read(&format!("{started}JOIN|2|2|B")), playing(1));
        assert_eq!(read(&format!("{started}MATCH_END|")), playing(1));
        assert_eq!(read(&format!("{started}MATCH_END|9|TIME\r\n")), Live::Idle);
    }

    #[test]
    fn the_next_match_starts_afresh() {
        let match_start = "MATCH_START|1|workshop-island-night|Default|0|";
        let text = format!(
            "{EXAMPLE}[00:02:00] GBR|120|1|1.3.3R|999\n[00:02:00] {match_start}\n[00:02:00] JOIN|120|1|Sparrow\n"
        );
        assert_eq!(read(&text), playing(1));
        // A `GBR` that wasn't ended starts a new match too (the old one was cut off).
        let cut = format!(
            "{}GBR|120|1|1.3.3R|999\n{match_start}\n",
            until("MATCH_END|")
        );
        assert_eq!(read(&cut), playing(0));
    }

    #[test]
    fn an_unranked_match_isnt_listed() {
        let text = format!("{}[00:00:30] UNRANKED|30|BOT\n", until("MATCH_END|"));
        assert_eq!(read(&text), Live::Unranked);
        // The next match is ranked again.
        let next = format!(
            "{text}MATCH_END|9|TIME\nGBR|1|1|1.3.3R|2\nMATCH_START|1|workshop-island-night|Default|0|\n"
        );
        assert_eq!(read(&next), playing(0));
        // Lines of other modes, or before any match, don't count.
        assert_eq!(read("UNRANKED|1|MAP\nJOIN|1|1|A\n"), Live::Idle);
    }

    #[test]
    fn caps_the_players_at_the_servers_limit() {
        let mut text =
            String::from("GBR|1|1|1.3.3R|1\nMATCH_START|1|workshop-island-night|Default|0|\n");
        for id in 1..=20 {
            text.push_str(&format!("JOIN|1|{id}|P{id}\n"));
        }
        assert_eq!(read(&text), playing(config::LOBBY_PLAYERS_MAX));
    }

    fn file(name: &str, size: u64, age: u64) -> Option<LogFile<'_>> {
        Some(LogFile {
            name,
            size,
            age: Duration::from_secs(age),
        })
    }

    #[test]
    fn a_log_that_stops_growing_isnt_being_played() {
        let mut watch = Watch::default();
        let now = Instant::now();
        let text = until("MATCH_END|");
        let size = text.len() as u64;
        let read = || Some(text.clone());
        assert_eq!(
            watch.look(file("Log-a.txt", size, 0), now, QUIET, read),
            playing(5)
        );
        // Not read again while it doesn't grow, and played until it's been quiet for `QUIET`.
        let later = now + QUIET - Duration::from_secs(1);
        assert_eq!(
            watch.look(file("Log-a.txt", size, 59), later, QUIET, || unreachable!()),
            playing(5)
        );
        assert_eq!(
            watch.look(
                file("Log-a.txt", size, 60),
                now + QUIET,
                QUIET,
                || unreachable!()
            ),
            Live::Idle
        );
        // It grows again: played again.
        let grown = format!("{text}KILL|50|A|B|1|2\n");
        let at = now + QUIET * 2;
        assert_eq!(
            watch.look(
                file("Log-a.txt", grown.len() as u64, 0),
                at,
                QUIET,
                || Some(grown.clone())
            ),
            playing(5)
        );
    }

    #[test]
    fn an_old_log_isnt_read_at_all() {
        let mut watch = Watch::default();
        let now = Instant::now();
        let old = file("Log-a.txt", 100, config::QUIET_SECS.default * 10);
        assert_eq!(watch.look(old, now, QUIET, || unreachable!()), Live::Idle);
        assert_eq!(
            watch.look(old, now + QUIET, QUIET, || unreachable!()),
            Live::Idle
        );
        // A new file after it is read: the host moved to spectator, which repeats the match so far.
        let text = until("MATCH_END|");
        let new = file("Log-b.txt", text.len() as u64, 0);
        assert_eq!(
            watch.look(new, now + QUIET, QUIET, || Some(text.clone())),
            playing(5)
        );
        // No log at all.
        assert_eq!(
            watch.look(None, now + QUIET, QUIET, || unreachable!()),
            Live::Idle
        );
    }

    #[test]
    fn a_locked_log_is_read_at_the_next_poll() {
        let mut watch = Watch::default();
        let now = Instant::now();
        let text = until("MATCH_END|");
        let size = text.len() as u64;
        assert_eq!(
            watch.look(file("Log-a.txt", size, 0), now, QUIET, || None),
            Live::Idle
        );
        assert_eq!(
            watch.look(file("Log-a.txt", size, 0), now, QUIET, || Some(
                text.clone()
            )),
            playing(5)
        );
        // Grown but locked: what it held before stands.
        assert_eq!(
            watch.look(file("Log-a.txt", size + 9, 0), now, QUIET, || None),
            playing(5)
        );
    }

    fn beat(players: u32) -> Heartbeat {
        Heartbeat {
            players,
            name: Some("Kenzo's ranked".into()),
            region: Some("eu".into()),
        }
    }

    #[test]
    fn heartbeats_while_a_match_is_played_then_closes() {
        let mut schedule = Schedule::default();
        let now = Instant::now();
        assert_eq!(schedule.step(None, now), Step::Wait);
        assert_eq!(schedule.step(Some(&beat(6)), now), Step::Heartbeat);
        schedule.listed(beat(6), now, EVERY, TTL);
        assert!(schedule.is_listed());
        assert_eq!(
            schedule.step(Some(&beat(6)), now + EVERY - Duration::from_secs(1)),
            Step::Wait
        );
        assert_eq!(schedule.step(Some(&beat(6)), now + EVERY), Step::Heartbeat);
        // The match ended: closed at once, not rate limited.
        assert_eq!(
            schedule.step(None, now + Duration::from_secs(1)),
            Step::Close
        );
        schedule.closed(now + Duration::from_secs(1));
        assert!(!schedule.is_listed());
        assert_eq!(schedule.step(None, now + EVERY), Step::Wait);
    }

    #[test]
    fn a_change_goes_as_soon_as_the_server_takes_it() {
        let mut schedule = Schedule::default();
        let now = Instant::now();
        schedule.listed(beat(6), now, EVERY, TTL);
        // A player joined: not before the server's rate limit, then without waiting for the interval.
        assert_eq!(
            schedule.step(Some(&beat(7)), now + MIN_GAP - Duration::from_secs(1)),
            Step::Wait
        );
        assert_eq!(
            schedule.step(Some(&beat(7)), now + MIN_GAP),
            Step::Heartbeat
        );
        // So does a new name or region.
        let renamed = Heartbeat {
            name: None,
            ..beat(6)
        };
        assert_eq!(
            schedule.step(Some(&renamed), now + MIN_GAP),
            Step::Heartbeat
        );
        let moved = Heartbeat {
            region: Some("na".into()),
            ..beat(6)
        };
        assert_eq!(schedule.step(Some(&moved), now + MIN_GAP), Step::Heartbeat);
    }

    #[test]
    fn a_heartbeat_after_a_close_waits_for_the_rate_limit() {
        let mut schedule = Schedule::default();
        let now = Instant::now();
        schedule.listed(beat(6), now, EVERY, TTL);
        schedule.closed(now + Duration::from_secs(5));
        // The next match starts right away: listed once the server takes it.
        let soon = now + Duration::from_secs(5) + MIN_GAP - Duration::from_secs(1);
        assert_eq!(schedule.step(Some(&beat(6)), soon), Step::Wait);
        assert_eq!(
            schedule.step(Some(&beat(6)), soon + Duration::from_secs(1)),
            Step::Heartbeat
        );
    }

    #[test]
    fn waits_as_long_as_the_server_says() {
        let mut schedule = Schedule::default();
        let now = Instant::now();
        let after = Duration::from_secs(45);
        schedule.hold(now + after);
        assert_eq!(
            schedule.step(Some(&beat(6)), now + after - Duration::from_secs(1)),
            Step::Wait
        );
        assert_eq!(schedule.step(Some(&beat(6)), now + after), Step::Heartbeat);
        // A setting changed meanwhile: no need to wait.
        schedule.hold(now + after);
        schedule.release();
        assert_eq!(schedule.step(Some(&beat(6)), now), Step::Heartbeat);
    }

    #[test]
    fn takes_the_servers_interval_but_not_below_its_rate_limit() {
        let mut schedule = Schedule::default();
        let now = Instant::now();
        let (every, ttl) = timing(Some(90), Some(300));
        assert_eq!(
            (every, ttl),
            (Duration::from_secs(90), Duration::from_secs(300))
        );
        schedule.listed(beat(6), now, every, ttl);
        assert_eq!(schedule.step(Some(&beat(6)), now + EVERY), Step::Wait);
        assert_eq!(schedule.step(Some(&beat(6)), now + every), Step::Heartbeat);
        assert_eq!(timing(None, None), (EVERY, TTL));
        schedule.listed(beat(6), now, Duration::ZERO, ttl);
        assert_eq!(
            schedule.step(Some(&beat(6)), now + Duration::from_secs(1)),
            Step::Wait
        );
        assert_eq!(
            schedule.step(Some(&beat(6)), now + MIN_GAP),
            Step::Heartbeat
        );
    }

    #[test]
    fn a_lobby_past_its_ttl_is_off_the_list_already() {
        let mut schedule = Schedule::default();
        let now = Instant::now();
        schedule.listed(beat(6), now, EVERY, TTL);
        // Offline for a while: nothing left to close.
        assert_eq!(schedule.step(None, now + TTL), Step::Wait);
        assert!(!schedule.is_listed());
        // A close that failed is tried again after its wait, until then.
        schedule.listed(beat(6), now, EVERY, TTL);
        schedule.hold(now + MIN_GAP);
        assert_eq!(
            schedule.step(None, now + Duration::from_secs(1)),
            Step::Wait
        );
        assert_eq!(schedule.step(None, now + MIN_GAP), Step::Close);
        schedule.forget();
        assert_eq!(schedule.step(None, now + MIN_GAP), Step::Wait);
    }
}
