//! What the watcher needs to know about a Workshop log file (GenjiBall-CE `docs/ranked-log.md` on
//! `v1.3.3R`): whether it holds a ranked match, how many matches in it have ended, the players'
//! names for the upload history, and which rounds each match has started (for host AFK, `afk.rs`).
//! The server does the real parsing (and the match view, #16, will share its parser).

use std::time::SystemTime;

use chrono::{Local, NaiveDateTime, SecondsFormat, TimeZone};

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

/// What's in the complete lines of `text` (see `complete_lines`).
pub fn scan(text: &str) -> Scan {
    read(&text[..text.rfind('\n').map_or(0, |i| i + 1)]).0
}

/// What's in these lines, and the key of the match the last of them is in.
fn read(text: &str) -> (Scan, Option<&str>) {
    let mut found = Scan::default();
    let mut current = None;
    for line in text.lines().map(event) {
        if line.starts_with("GBR|") {
            found.ranked = true;
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
            // `JOIN|time|id|name`, and maybe fields a newer game appends.
            let name = fields.split('|').nth(2).unwrap_or_default();
            if !name.is_empty() && !found.players.iter().any(|p| p == name) {
                found.players.push(name.to_string());
            }
        }
    }
    (found, current)
}

/// Every round started in `text`, with the line the game may be halfway through writing once its
/// round is written in full (`round_start`): what has already started when the host turns AFK on.
pub fn round_starts(text: &str) -> Vec<RoundStart> {
    let complete = text.rfind('\n').map_or(0, |i| i + 1);
    let (found, current) = read(&text[..complete]);
    let mut starts = found.round_starts;
    if let (Some(round), Some(key)) = (round_start(event(&text[complete..])), current) {
        starts.push(RoundStart {
            match_key: key.to_string(),
            round,
        });
    }
    starts
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
        assert!(scan("GBR|2.38|1|1.3.3R|482913507226\nMATCH_END|9|TIME\n").ranked);
        assert_eq!(
            scan("GBR|2.38|1|1.3.3R|1\nMATCH_END|9|TIME\n").match_ends,
            1
        );
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
