//! What the watcher needs to know about a Workshop log file (GenjiBall-CE `docs/ranked-log.md` on
//! `v1.3.3R`): whether it holds a ranked match, how many matches in it have ended, the players'
//! names for the upload history, and which rounds each match has started and when the host sat
//! out (for host AFK, `afk.rs`).
//! The server does the real parsing (the match view, #16, uses its parser in the window).

use std::time::SystemTime;

use chrono::{Local, NaiveDateTime, SecondsFormat, TimeZone};
use serde::{Deserialize, Serialize};

use crate::config;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scan {
    /// A `GBR` line: a ranked match starts here. Files without one aren't uploaded.
    pub ranked: bool,
    /// `MATCH_END` lines. A new one means a match just ended, so the file is uploaded then.
    pub match_ends: usize,
    /// The names in `JOIN` lines, each once, in the order they first joined.
    pub players: Vec<String>,
    /// The `matchKey` of each `GBR` line, each once, in the order they start.
    pub match_keys: Vec<String>,
    /// Each `ROUND_START` after a `GBR`, with that match's key.
    pub round_starts: Vec<RoundStart>,
    /// The tourney matches (a `TOURNEY` line after their `GBR`, log format 2), in the order they
    /// start. A match without one is a ranked match.
    pub tourneys: Vec<TourneyMatch>,
    /// The lobby host's last sit-out in each match that has one, in the order they started. Only
    /// the file's last match can have one still going on (`SitOut::until` `None`).
    pub sit_outs: Vec<SitOut>,
}

/// The lobby host sitting out (GenjiBall-CE `docs/ranked-log.md`, "Host sit-out"): from a
/// `SIT_OUT|time|id|1` whose `id` is the lobby host's (a `JOIN` with `host` `1`) to their
/// `SIT_OUT ... 0` or `LEAVE`, `MATCH_END` or the next match's `GBR`, whichever comes first.
/// Another player's `SIT_OUT` (the game's host while the lobby host is in spectator) isn't one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SitOut {
    pub match_key: String,
    /// The last round the match had started at the `1` (`0`: none yet): the host sits out from
    /// the round after.
    pub after_round: u32,
    /// The last round the match had started where it ended: from the round after, the host plays
    /// (or is gone). `None` while it goes on.
    pub until: Option<u32>,
}

/// A tourney match: `TOURNEY|time|lobbyKey|roundLimit` after the `GBR` of `match_key`
/// (GenjiBall-CE `docs/ranked-log.md`, "Tourney matches").
#[derive(Debug, Clone, PartialEq)]
pub struct TourneyMatch {
    pub match_key: String,
    /// The server's id for the tourney lobby: digits, text (it may start with `0`).
    pub lobby_key: String,
    /// It ended with `MATCH_END|time|ROUNDS`: round `roundLimit` is over, and the game shows the
    /// final standings the host screenshots.
    pub ended: bool,
}

impl Scan {
    /// Whether every match in the file is a tourney match (and there's at least one).
    pub fn only_tourneys(&self) -> bool {
        !self.match_keys.is_empty()
            && self
                .match_keys
                .iter()
                .all(|key| self.tourneys.iter().any(|t| t.match_key == *key))
    }

    /// The lobby keys of the tourney matches in the file, each once.
    pub fn lobby_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for t in &self.tourneys {
            if !keys.contains(&t.lobby_key) {
                keys.push(t.lobby_key.clone());
            }
        }
        keys
    }

    /// The sit-out still going on at the end of the file, if there is one.
    pub fn sitting_out(&self) -> Option<&SitOut> {
        self.sit_outs.iter().find(|s| s.until.is_none())
    }
}

/// The `lobbyKey` of a `TOURNEY|time|lobbyKey|roundLimit` event: digits only, else `None`.
fn lobby_key(event: &str) -> Option<&str> {
    let key = event.strip_prefix("TOURNEY|")?.split('|').nth(1)?;
    (!key.is_empty() && key.bytes().all(|b| b.is_ascii_digit())).then_some(key)
}

/// Whether a `MATCH_END|time|result` event ended a tourney match (`ROUNDS`).
fn ended_by_rounds(event: &str) -> bool {
    event
        .strip_prefix("MATCH_END|")
        .and_then(|rest| rest.split('|').nth(1))
        == Some("ROUNDS")
}

/// A round that started: `ROUND_START|time|round|ids` in the match whose `GBR` came before it.
#[derive(Debug, Clone, PartialEq)]
pub struct RoundStart {
    pub match_key: String,
    pub round: u32,
}

/// The `matchKey` of a `GBR|time|format|gameVersion|matchKey` event (text, not a number).
fn match_key(event: &str) -> Option<&str> {
    let key = event.strip_prefix("GBR|")?.split('|').nth(3)?;
    (!key.is_empty()).then_some(key)
}

/// The round of a `ROUND_START|time|round|ids` event, once the round is written in full: the `|`
/// after it is there. So the half-written last line of a file counts once its round is known.
fn round_start(event: &str) -> Option<u32> {
    let mut fields = event.strip_prefix("ROUND_START|")?.splitn(3, '|');
    let (_time, round) = (fields.next()?, fields.next()?);
    fields.next()?;
    round.parse().ok()
}

/// A `SIT_OUT|time|id|on` event: the player id, and whether they start sitting out (`1`) or
/// stop (`0`).
fn sit_out(event: &str) -> Option<(u32, bool)> {
    let mut fields = event.strip_prefix("SIT_OUT|")?.split('|').skip(1);
    let id = fields.next()?.parse().ok()?;
    match fields.next()? {
        "1" => Some((id, true)),
        "0" => Some((id, false)),
        _ => None,
    }
}

/// The player id of a `LEAVE|time|id` event.
fn leave(event: &str) -> Option<u32> {
    event
        .strip_prefix("LEAVE|")?
        .split('|')
        .nth(1)?
        .parse()
        .ok()
}

/// The event part of a line: the Workshop's `[hh:mm:ss] ` prefix stripped, if it's there.
pub fn event(line: &str) -> &str {
    let line = line.trim_end_matches('\r');
    match line
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
    {
        Some((_, event)) => event,
        None => line,
    }
}

/// The complete lines at the start of a log: up to and with its last `\n`. The game may be in the
/// middle of writing the line after it (a `MATCH_END|` without the rest yet), so only this part
/// is read and uploaded. The server counts lines the same way (a `\n` ends one).
pub fn complete_lines(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    &bytes[..end]
}

/// `complete_lines` of a log already read as text.
pub fn complete_text(text: &str) -> &str {
    &text[..text.rfind('\n').map_or(0, |i| i + 1)]
}

/// What's in the complete lines of `text` (see `complete_lines`).
pub fn scan(text: &str) -> Scan {
    read(complete_text(text)).0
}

/// The last round the match `key` has started in what's read so far (`0`: none).
fn last_round(found: &Scan, key: Option<&str>) -> u32 {
    found
        .round_starts
        .iter()
        .rev()
        .find(|r| Some(r.match_key.as_str()) == key)
        .map_or(0, |r| r.round)
}

/// Ends the sit-out going on, if there is one, after the last round its match has started.
fn end_sit_out(found: &mut Scan) {
    let Some(key) = found.sitting_out().map(|s| s.match_key.clone()) else {
        return;
    };
    let last = last_round(found, Some(&key));
    if let Some(s) = found.sit_outs.iter_mut().find(|s| s.until.is_none()) {
        s.until = Some(last);
    }
}

/// What's in these lines, and the key of the match the last of them is in.
fn read(text: &str) -> (Scan, Option<&str>) {
    let mut found = Scan::default();
    let mut current = None;
    // The ids the current match gave the lobby host (`JOIN` with `host` `1`: a new one each time
    // they come back), and the one sitting out.
    let mut hosts: Vec<u32> = Vec::new();
    let mut sitting: Option<u32> = None;
    for line in text.lines().map(event) {
        if line.starts_with("GBR|") {
            found.ranked = true;
            // A match that stopped without `MATCH_END`: the host doesn't sit out in the next one.
            end_sit_out(&mut found);
            hosts.clear();
            sitting = None;
            current = match_key(line);
            if let Some(key) = current {
                if !found.match_keys.iter().any(|k| k == key) {
                    found.match_keys.push(key.to_string());
                }
            }
        } else if let Some(round) = round_start(line) {
            if let Some(key) = current {
                found.round_starts.push(RoundStart {
                    match_key: key.to_string(),
                    round,
                });
            }
        } else if line.starts_with("MATCH_END|") {
            found.match_ends += 1;
            end_sit_out(&mut found);
            sitting = None;
            if ended_by_rounds(line) {
                if let Some(t) = found
                    .tourneys
                    .iter_mut()
                    .rev()
                    .find(|t| Some(t.match_key.as_str()) == current)
                {
                    t.ended = true;
                }
            }
        } else if let Some((id, on)) = sit_out(line) {
            // Only the lobby host's: while they're in spectator, the game's host is someone else.
            if let Some(key) = current.filter(|_| hosts.contains(&id)) {
                if !on {
                    end_sit_out(&mut found);
                    sitting = None;
                } else {
                    // A second `1` keeps the round the sit-out started after.
                    if found.sitting_out().is_none() {
                        let after_round = last_round(&found, current);
                        found.sit_outs.retain(|s| s.match_key != key);
                        found.sit_outs.push(SitOut {
                            match_key: key.to_string(),
                            after_round,
                            until: None,
                        });
                    }
                    sitting = Some(id);
                }
            }
        } else if let Some(id) = leave(line) {
            // The game logs no `0` for a host who leaves or moves to spectator while sitting out.
            if sitting == Some(id) {
                end_sit_out(&mut found);
                sitting = None;
            }
        } else if let Some(lobby) = lobby_key(line) {
            // The first `TOURNEY` of the match counts: the game logs one, right after `GBR`.
            if let Some(key) = current {
                if !found.tourneys.iter().any(|t| t.match_key == key) {
                    found.tourneys.push(TourneyMatch {
                        match_key: key.to_string(),
                        lobby_key: lobby.to_string(),
                        ended: false,
                    });
                }
            }
        } else if let Some(fields) = line.strip_prefix("JOIN|") {
            // `JOIN|time|id|name|host`, and maybe fields a newer game appends.
            let mut fields = fields.split('|').skip(1);
            let id = fields.next().and_then(|id| id.parse::<u32>().ok());
            let name = fields.next().unwrap_or_default();
            if !name.is_empty() && !found.players.iter().any(|p| p == name) {
                found.players.push(name.to_string());
            }
            if let (Some(id), Some("1")) = (id, fields.next()) {
                hosts.push(id);
            }
        }
    }
    (found, current)
}

/// Every round started in `text`, with the line the game may be halfway through writing once its
/// round is written in full (`round_start`): what has already started when the host turns AFK on.
pub fn round_starts(text: &str) -> Vec<RoundStart> {
    let complete = complete_text(text);
    let (found, current) = read(complete);
    let mut starts = found.round_starts;
    if let (Some(round), Some(key)) = (round_start(event(&text[complete.len()..])), current) {
        starts.push(RoundStart {
            match_key: key.to_string(),
            round,
        });
    }
    starts
}

/// What's in the live log `text` read right now (the host's AFK switch): `scan`, with its round
/// starts from `round_starts`, so a round whose `ROUND_START` is being written counts.
pub fn live_scan(text: &str) -> Scan {
    Scan {
        round_starts: round_starts(text),
        ..scan(text)
    }
}

/// Whether Overwatch would have named a Workshop log this.
pub fn is_log_file(name: &str) -> bool {
    name.starts_with(config::LOG_FILE_PREFIX) && name.ends_with(config::LOG_FILE_SUFFIX)
}

/// When Overwatch started the file, from its name (`Log-2026-09-22-21-27-16.txt`, this PC's time).
pub fn started_at(name: &str) -> Option<NaiveDateTime> {
    let stamp = name
        .strip_prefix(config::LOG_FILE_PREFIX)?
        .strip_suffix(config::LOG_FILE_SUFFIX)?;
    NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d-%H-%M-%S").ok()
}

/// `started_at` as a moment in time.
pub fn started_at_time(name: &str) -> Option<SystemTime> {
    let local = Local.from_local_datetime(&started_at(name)?).earliest()?;
    Some(local.into())
}

/// `started_at` as the server's `X-Log-Started-At`: ISO 8601 with this PC's time zone.
pub fn started_at_header(name: &str) -> Option<String> {
    let local = Local.from_local_datetime(&started_at(name)?).earliest()?;
    Some(local.to_rfc3339_opts(SecondsFormat::Secs, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../tests/fixtures/ranked-log-example.txt");

    #[test]
    fn reads_the_example_log() {
        assert_eq!(
            scan(EXAMPLE),
            Scan {
                ranked: true,
                match_ends: 1,
                // Two players named Ghost, and Nova joining after round 1.
                players: ["Sparrow", "Tidal", "Mochi", "Ghost", "Nova"]
                    .map(String::from)
                    .to_vec(),
                match_keys: vec!["482913507226".into()],
                round_starts: [1, 2, 3].map(|round| start("482913507226", round)).to_vec(),
                tourneys: vec![],
                sit_outs: vec![],
            }
        );
        assert!(!scan(EXAMPLE).only_tourneys());
    }

    const TOURNEY: &str = include_str!("../tests/fixtures/ranked-log-tourney-example.txt");

    #[test]
    fn reads_a_tourney_match() {
        let found = scan(TOURNEY);
        assert!(found.ranked);
        assert_eq!(found.match_keys, ["219604738815"]);
        assert_eq!(
            found.tourneys,
            [TourneyMatch {
                match_key: "219604738815".into(),
                // Text: the leading 0 stays.
                lobby_key: "073518264903".into(),
                ended: true,
            }]
        );
        assert!(found.only_tourneys());
        assert_eq!(found.lobby_keys(), ["073518264903"]);
        // Not ended until `MATCH_END|…|ROUNDS` is written in full.
        let playing = &TOURNEY[..TOURNEY.find("[00:01:20] MATCH_END").unwrap()];
        assert!(!scan(playing).tourneys[0].ended);
    }

    #[test]
    fn tells_tourney_and_ranked_matches_apart_in_one_file() {
        let text = format!("{EXAMPLE}{TOURNEY}");
        let found = scan(&text);
        assert_eq!(found.match_keys, ["482913507226", "219604738815"]);
        assert_eq!(found.tourneys.len(), 1);
        assert!(!found.only_tourneys());
        // A `TIME` end isn't a tourney's, and a `TOURNEY` line before any `GBR` belongs to none.
        let odd = "TOURNEY|1|111|30\nGBR|1|2|1.3.3R|5\nTOURNEY|1|0042|30\nMATCH_END|9|TIME\n";
        let found = scan(odd);
        assert_eq!(found.tourneys.len(), 1);
        assert_eq!(found.tourneys[0].lobby_key, "0042");
        assert!(!found.tourneys[0].ended);
        // A lobby key that isn't digits isn't one.
        assert!(scan("GBR|1|2|1.3.3R|5\nTOURNEY|1|abc|30\n")
            .tourneys
            .is_empty());
    }

    fn start(match_key: &str, round: u32) -> RoundStart {
        RoundStart {
            match_key: match_key.into(),
            round,
        }
    }

    #[test]
    fn reads_the_rounds_of_each_match() {
        let text =
            "GBR|1|1|1.3.3R|111\nROUND_START|2|1|1,2\nROUND_START|3|2|1,2\nMATCH_END|4|TIME\n\
                    [00:00:05] GBR|5|1|1.3.3R|222\n[00:00:06] ROUND_START|6|1|1,2\n";
        let found = scan(text);
        assert_eq!(found.match_keys, ["111", "222"]);
        assert_eq!(
            found.round_starts,
            [start("111", 1), start("111", 2), start("222", 1)]
        );
        // A round before any `GBR` has no match, and a `GBR` without a key starts none.
        let found = scan("ROUND_START|1|1|1\nGBR|2|1|1.3.3R|\nROUND_START|3|2|1\n");
        assert!(found.match_keys.is_empty());
        assert!(found.round_starts.is_empty());
    }

    fn sat(match_key: &str, after_round: u32, until: Option<u32>) -> SitOut {
        SitOut {
            match_key: match_key.into(),
            after_round,
            until,
        }
    }

    /// The start of a match: the host (id 1) and two others join, and round 1 is played.
    const HOSTED: &str = "GBR|1|2|1.3.3R|111
MATCH_START|1|workshop-island-night|Standard|0|
JOIN|1|1|Host|1
JOIN|1|2|Ash|
JOIN|1|3|Kenzo|
ROUND_START|2|1|1,2,3
ROUND_END|3|1|1|WIN
";

    #[test]
    fn reads_the_host_sit_out() {
        assert_eq!(scan(HOSTED).sit_outs, [], "plays until a `SIT_OUT`");
        let out = format!("{HOSTED}SIT_OUT|4|1|1\nROUND_START|5|2|2,3\n");
        let found = scan(&out);
        assert_eq!(found.sit_outs, [sat("111", 1, None)]);
        assert_eq!(found.sitting_out(), Some(&sat("111", 1, None)));
        // Another `1` keeps the round it started after.
        assert_eq!(
            scan(&format!("{out}SIT_OUT|6|1|1\n")).sit_outs,
            [sat("111", 1, None)]
        );
        // Played again from round 3, or the match ended after round 2.
        for end in ["SIT_OUT|6|1|0", "MATCH_END|6|TIME"] {
            let found = scan(&format!("{out}{end}\nROUND_START|7|3|1,2,3\n"));
            assert_eq!(found.sit_outs, [sat("111", 1, Some(2))], "{end}");
            assert_eq!(found.sitting_out(), None);
        }
        // Only complete lines, and only a valid `on` and id.
        assert_eq!(scan(&format!("{HOSTED}SIT_OUT|4|1|1")).sit_outs, []);
        for odd in ["SIT_OUT|4|1|yes", "SIT_OUT|4||1", "SIT_OUT|4|1"] {
            assert_eq!(scan(&format!("{HOSTED}{odd}\n")).sit_outs, [], "{odd}");
        }
    }

    #[test]
    fn follows_only_the_lobby_host_sit_out() {
        // Ash (2) is the game's host while the lobby host is in spectator: not the tool's user.
        let other = format!("{HOSTED}SIT_OUT|4|2|1\nROUND_START|5|2|1,3\n");
        assert_eq!(scan(&other).sit_outs, []);
        // Nor before anyone's `JOIN`, nor a `JOIN` whose `host` isn't `1`.
        let early = "GBR|1|2|1.3.3R|111\nSIT_OUT|2|1|1\nJOIN|3|1|Host|1\n";
        assert_eq!(scan(early).sit_outs, []);
        let not_host =
            "GBR|1|2|1.3.3R|111\nJOIN|1|1|Host|0\nJOIN|1|2|Ash|\nSIT_OUT|2|1|1\nSIT_OUT|2|2|1\n";
        assert_eq!(scan(not_host).sit_outs, []);
        // Ids are per match: the host's id in the last match isn't the host's in this one.
        let next =
            format!("{HOSTED}MATCH_END|4|TIME\nGBR|5|2|1.3.3R|222\nJOIN|5|1|Ash|\nSIT_OUT|6|1|1\n");
        assert_eq!(scan(&next).sit_outs, []);
    }

    #[test]
    fn a_sit_out_ends_when_the_host_leaves() {
        // The host sits out from round 2, then moves to spectator after round 3: no `0` is logged.
        let out = format!(
            "{HOSTED}SIT_OUT|4|1|1\nROUND_START|5|2|2,3\nROUND_END|6|2|2|WIN\n\
             ROUND_START|7|3|2,3\nROUND_END|8|3|2|WIN\nLEAVE|9|1\n"
        );
        assert_eq!(scan(&out).sit_outs, [sat("111", 1, Some(3))]);
        // Someone else's `LEAVE` doesn't end it.
        let other = format!("{HOSTED}SIT_OUT|4|1|1\nROUND_START|5|2|2,3\nLEAVE|6|2\n");
        assert_eq!(scan(&other).sit_outs, [sat("111", 1, None)]);
        // Ash, the game's host now, sits out: not followed. The lobby host comes back with a new
        // id, plays round 5, and sits out again from round 6.
        let back = format!(
            "{out}SIT_OUT|10|2|1\nROUND_START|11|4|3\nJOIN|12|4|Host|1\n\
             ROUND_START|13|5|3,4\nSIT_OUT|14|4|1\nROUND_START|15|6|3\n"
        );
        assert_eq!(scan(&back).sit_outs, [sat("111", 5, None)]);
        // The id they had before leaving is gone: its `0` would end nothing new.
        let back_playing = format!("{out}JOIN|12|4|Host|1\nROUND_START|13|4|2,3,4\n");
        assert_eq!(scan(&back_playing).sit_outs, [sat("111", 1, Some(3))]);
        assert_eq!(scan(&back_playing).sitting_out(), None);
    }

    #[test]
    fn keeps_the_last_sit_out_of_each_match() {
        let first = format!("{HOSTED}SIT_OUT|4|1|1\nROUND_START|5|2|2,3\n");
        // A match that stops without `MATCH_END` ends its sit-out at the next `GBR`.
        let text = format!(
            "{first}GBR|6|2|1.3.3R|222\nJOIN|6|1|Host|1\nJOIN|6|2|Ash|\nROUND_START|7|1|1,2\n\
             SIT_OUT|8|1|1\nROUND_START|9|2|2\nSIT_OUT|10|1|0\nSIT_OUT|11|1|1\n"
        );
        let found = scan(&text);
        assert_eq!(
            found.sit_outs,
            [sat("111", 1, Some(2)), sat("222", 2, None)]
        );
        assert_eq!(found.sitting_out(), Some(&sat("222", 2, None)));
    }

    #[test]
    fn a_half_written_round_start_counts_once_its_round_is_there() {
        let done = "GBR|1|1|1.3.3R|111\nROUND_START|2|1|1,2\n";
        // `scan` only reads complete lines; `round_starts` takes the last one once its round is.
        for (rest, rounds) in [
            ("ROUND_START|3|", vec![1]),
            ("ROUND_START|3|1", vec![1]),
            ("ROUND_START|3|2|", vec![1, 2]),
            ("[00:00:03] ROUND_START|3|2|1,", vec![1, 2]),
        ] {
            let text = format!("{done}{rest}");
            assert_eq!(scan(&text).round_starts, [start("111", 1)], "{rest}");
            assert_eq!(
                round_starts(&text),
                rounds.iter().map(|&r| start("111", r)).collect::<Vec<_>>(),
                "{rest}"
            );
        }
        assert!(round_starts("ROUND_START|1|1|1").is_empty());
    }

    #[test]
    fn reads_names_only_from_join_lines() {
        let text = "GBR|1|1|1.3.3R|1\nJOIN|1|1|Kenzo|extra\nJOIN|2|2|\nJOIN|3\n[00:00:04] JOIN|4|3|Kenzo\nKILL|5|Ash|Kenzo|4|1\n";
        assert_eq!(scan(text).players, ["Kenzo"]);
    }

    #[test]
    fn a_match_still_being_played_has_no_end() {
        let start: String = EXAMPLE
            .lines()
            .take_while(|line| !line.contains("MATCH_END|"))
            .map(|line| format!("{line}\r\n"))
            .collect();
        let found = scan(&start);
        assert!(found.ranked);
        assert_eq!(found.match_ends, 0);
    }

    #[test]
    fn counts_every_match_in_a_file() {
        let two = format!("{EXAMPLE}\n{EXAMPLE}");
        assert_eq!(scan(&two).match_ends, 2);
    }

    #[test]
    fn other_logs_arent_ranked() {
        let legacy = "[00:00:28] KILL|28.40|Sparrow|Ghost\n[00:00:30] KILL|30.10|Ghost|Tidal\n";
        assert_eq!(scan(legacy), Scan::default());
        assert_eq!(scan(""), Scan::default());
        // Only a line that starts with the event counts.
        assert!(!scan("[00:00:01] say GBR|1|1|1.3.3R|1\n").ranked);
    }

    #[test]
    fn reads_lines_without_the_prefix() {
        let found = scan("GBR|2.38|1|1.3.3R|482913507226\nMATCH_END|9|TIME\n");
        assert!(found.ranked);
        assert_eq!(found.match_ends, 1);
    }

    #[test]
    fn a_line_still_being_written_doesnt_count() {
        // The game paused in the middle of writing the `MATCH_END` line.
        let end = EXAMPLE.find("MATCH_END|").unwrap() + "MATCH_END|".len();
        let half = &EXAMPLE[..end];
        assert_eq!(scan(half).match_ends, 0);
        assert!(scan(half).ranked);
        assert_eq!(
            complete_lines(half.as_bytes()),
            &EXAMPLE.as_bytes()[..EXAMPLE.find("[00:01:54] MATCH_END").unwrap()]
        );
        // Once the line has its `\n`, it's there.
        assert_eq!(scan(EXAMPLE).match_ends, 1);
        assert_eq!(complete_lines(EXAMPLE.as_bytes()), EXAMPLE.as_bytes());
        assert_eq!(complete_lines(b"GBR|1|1|1.3.3R|1"), b"");
        assert_eq!(scan("GBR|1|1|1.3.3R|1"), Scan::default());
    }

    #[test]
    fn knows_log_file_names() {
        assert!(is_log_file("Log-2026-09-22-21-27-16.txt"));
        assert!(!is_log_file("Log-2026-09-22-21-27-16.txt.bak"));
        assert!(!is_log_file("notes.txt"));
    }

    #[test]
    fn reads_the_start_from_the_name() {
        assert_eq!(
            started_at("Log-2026-09-22-21-27-16.txt"),
            NaiveDateTime::parse_from_str("2026-09-22 21:27:16", "%Y-%m-%d %H:%M:%S").ok()
        );
        assert_eq!(started_at("Log-yesterday.txt"), None);
        assert_eq!(started_at("2026-09-22-21-27-16.txt"), None);

        let header = started_at_header("Log-2026-09-22-21-27-16.txt").unwrap();
        assert!(header.starts_with("2026-09-22T21:27:16"), "{header}");
        // Only parses with a time zone.
        assert!(
            chrono::DateTime::parse_from_rfc3339(&header).is_ok(),
            "{header}"
        );
    }
}
