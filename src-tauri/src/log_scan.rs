//! What the watcher needs to know about a Workshop log file (GenjiBall-CE `docs/ranked-log.md` on
//! `v1.3.3R`): whether it holds a ranked match, and how many matches in it have ended. The server
//! does the real parsing.

use chrono::{Local, NaiveDateTime, SecondsFormat, TimeZone};

use crate::config;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Scan {
    /// A `GBR` line: a ranked match starts here. Files without one aren't uploaded.
    pub ranked: bool,
    /// `MATCH_END` lines. A new one means a match just ended, so the file is uploaded then.
    pub match_ends: usize,
}

/// The event part of a line: the Workshop's `[hh:mm:ss] ` prefix stripped, if it's there.
fn event(line: &str) -> &str {
    let line = line.trim_end_matches('\r');
    match line
        .strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
    {
        Some((_, event)) => event,
        None => line,
    }
}

pub fn scan(text: &str) -> Scan {
    let mut found = Scan::default();
    for line in text.lines().map(event) {
        if line.starts_with("GBR|") {
            found.ranked = true;
        } else if line.starts_with("MATCH_END|") {
            found.match_ends += 1;
        }
    }
    found
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
                match_ends: 1
            }
        );
    }

    #[test]
    fn a_match_still_being_played_has_no_end() {
        let start: String = EXAMPLE
            .lines()
            .take_while(|line| !line.contains("MATCH_END|"))
            .map(|line| format!("{line}\r\n"))
            .collect();
        assert_eq!(
            scan(&start),
            Scan {
                ranked: true,
                match_ends: 0
            }
        );
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
        assert!(scan("GBR|2.38|1|1.3.3R|482913507226\nMATCH_END|9|TIME").ranked);
        assert_eq!(scan("GBR|2.38|1|1.3.3R|1\nMATCH_END|9|TIME").match_ends, 1);
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
