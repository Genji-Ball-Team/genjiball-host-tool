//! The record of uploaded log files (`uploads.json`): per server URL and file name, the size that
//! was sent and what the server said. A file is uploaded again only when it's grown since, so a
//! restart doesn't send every file again, and a file that couldn't be sent (offline) still is.
//! It also holds when each server's uploads started (`started`), so a record that can't be read
//! is never written over: uploads wait until it's fixed or deleted. Deleting it starts afresh: the
//! server answers a file it already has with `duplicate`, and uploads to a server with a token
//! start again from then.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::server::{self, MatchState, UploadAnswer};
use crate::settings;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Record {
    /// Server URL → file name → the last upload of that file.
    servers: BTreeMap<String, BTreeMap<String, Sent>>,
    /// Server URL → when the tool first had a token for it (Unix seconds). Logs last written
    /// before that aren't uploaded to it, so switching from the test server to the real one
    /// doesn't send the test matches along.
    started: BTreeMap<String, u64>,
    /// Goes up each time an upload or its status changes, on any page of the history: the window
    /// asks for the page it shows again. Memory only.
    #[serde(skip)]
    revision: u64,
    /// Changed since it was last saved: `save` writes it, and tries again until that works.
    #[serde(skip)]
    dirty: bool,
}

/// The same uploads, whatever the revision.
impl PartialEq for Record {
    fn eq(&self, other: &Self) -> bool {
        self.servers == other.servers && self.started == other.started
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    /// Bytes sent, the file's complete lines (`log_scan::complete_lines`): the file is sent again
    /// once those are a different size.
    pub size: u64,
    /// `MATCH_END` lines in what was sent: a new one is sent straight away.
    pub match_ends: usize,
    /// When, RFC 3339 in UTC.
    pub at: String,
    /// The players' names in what was sent (`JOIN` lines), for the upload history.
    #[serde(default)]
    pub players: Vec<String>,
    pub answer: Answer,
    /// The tourney matches in what was sent (`TOURNEY` lines): the history marks them, and a
    /// lobby whose match ended is asked for its verify screenshot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tourneys: Vec<SentTourney>,
}

/// A tourney match in an upload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentTourney {
    /// Its lobby's key (`TOURNEY|time|lobbyKey|roundLimit`).
    pub lobby_key: String,
    /// It ended with `MATCH_END ROUNDS`: the final standings were shown.
    pub ended: bool,
}

impl SentTourney {
    /// The tourney matches of a scanned file.
    pub fn of(scan: &crate::log_scan::Scan) -> Vec<Self> {
        scan.tourneys
            .iter()
            .map(|t| SentTourney {
                lobby_key: t.lobby_key.clone(),
                ended: t.ended,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Answer {
    /// The server took the upload (`200`).
    Answered(UploadAnswer),
    /// The server, or the tool before sending, refused the file (`413`, `422`).
    Refused { error: String, message: String },
}

/// A match as the record knows it (`Record::known_matches`).
#[derive(Debug, Clone, PartialEq)]
pub struct KnownMatch {
    pub match_key: String,
    pub match_id: Option<i64>,
    /// As the server last said.
    pub status: String,
    /// When its first copy was uploaded, RFC 3339 in UTC.
    pub first_at: String,
}

impl Record {
    pub fn started(&self, server_url: &str) -> Option<SystemTime> {
        let secs = *self.started.get(server_url)?;
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    /// Notes that uploads to `server_url` start at `now`, unless they already started.
    /// `true` when that's new.
    pub fn start(&mut self, server_url: &str, now: SystemTime) -> bool {
        if self.started.contains_key(server_url) {
            return false;
        }
        let secs = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.started.insert(server_url.to_string(), secs);
        self.dirty = true;
        true
    }

    pub fn get(&self, server_url: &str, file: &str) -> Option<&Sent> {
        self.servers.get(server_url)?.get(file)
    }

    pub fn put(&mut self, server_url: &str, file: &str, sent: Sent) {
        self.servers
            .entry(server_url.to_string())
            .or_default()
            .insert(file.to_string(), sent);
        self.revision += 1;
        self.dirty = true;
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The keys of the `max` newest matches uploaded to `server_url`: the ones whose status is
    /// asked of the server again. Newest file first, and in each file its last match first.
    pub fn recent_match_keys(&self, server_url: &str, max: usize) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for (_, sent) in self.uploads(server_url) {
            let Answer::Answered(answer) = &sent.answer else {
                continue;
            };
            for key in answer
                .matches
                .iter()
                .rev()
                .filter_map(|m| m.match_key.as_ref())
            {
                if keys.len() == max {
                    return keys;
                }
                if !keys.contains(key) {
                    keys.push(key.clone());
                }
            }
        }
        keys
    }

    /// Puts the server's status now on every upload of those matches. `true` when one changed.
    pub fn update_states(&mut self, server_url: &str, states: &[MatchState]) -> bool {
        let mut changed = false;
        let uploads = self
            .servers
            .get_mut(server_url)
            .into_iter()
            .flat_map(|f| f.values_mut());
        for sent in uploads {
            let Answer::Answered(answer) = &mut sent.answer else {
                continue;
            };
            for m in &mut answer.matches {
                let Some(state) = states
                    .iter()
                    .find(|s| m.match_key.as_ref() == Some(&s.match_key))
                else {
                    continue;
                };
                if m.status != state.status
                    || m.rejection != state.rejection
                    || m.review_reasons != state.review_reasons
                {
                    m.status = state.status.clone();
                    m.rejection = state.rejection.clone();
                    m.review_reasons = state.review_reasons.clone();
                    changed = true;
                }
                // A server that doesn't give the id yet leaves the one we have.
                if state.match_id.is_some() && m.match_id != state.match_id {
                    m.match_id = state.match_id;
                    changed = true;
                }
            }
        }
        if changed {
            self.revision += 1;
            self.dirty = true;
        }
        changed
    }

    /// Every match uploaded to `server_url`, once each, newest upload first: for the overlay's
    /// match summary and session (#53). Each with its id on the site once the status refresh
    /// brought one, and when it was first uploaded.
    pub fn known_matches(&self, server_url: &str) -> Vec<KnownMatch> {
        let mut known: Vec<KnownMatch> = Vec::new();
        for (_, sent) in self.uploads(server_url) {
            let Answer::Answered(answer) = &sent.answer else {
                continue;
            };
            for m in answer.matches.iter().rev() {
                let Some(key) = &m.match_key else { continue };
                match known.iter_mut().find(|k| &k.match_key == key) {
                    // An older copy: uploaded first then.
                    Some(k) => {
                        k.first_at = sent.at.clone();
                        k.match_id = k.match_id.or(m.match_id);
                    }
                    None => known.push(KnownMatch {
                        match_key: key.clone(),
                        match_id: m.match_id,
                        status: m.status.clone(),
                        first_at: sent.at.clone(),
                    }),
                }
            }
        }
        known
    }

    /// Whether a match uploaded to `server_url` has this id on the site and is public there.
    pub fn has_public_match(&self, server_url: &str, match_id: i64) -> bool {
        self.uploads(server_url)
            .into_iter()
            .any(|(_, sent)| match &sent.answer {
                Answer::Answered(answer) => answer
                    .matches
                    .iter()
                    .any(|m| m.match_id == Some(match_id) && server::is_public(&m.status)),
                Answer::Refused { .. } => false,
            })
    }

    /// Whether a tourney match of the lobby with `lobby_key` was uploaded to `server_url` with its
    /// end (`MATCH_END ROUNDS`), and the server took the upload.
    pub fn tourney_ended(&self, server_url: &str, lobby_key: &str) -> bool {
        self.servers
            .get(server_url)
            .into_iter()
            .flat_map(|files| files.values())
            .filter(|sent| matches!(sent.answer, Answer::Answered(_)))
            .any(|sent| {
                sent.tourneys
                    .iter()
                    .any(|t| t.ended && t.lobby_key == lobby_key)
            })
    }

    /// Every file uploaded to `server_url` and its last upload, newest first.
    pub fn uploads(&self, server_url: &str) -> Vec<(&str, &Sent)> {
        let mut all: Vec<_> = self
            .servers
            .get(server_url)
            .into_iter()
            .flatten()
            .map(|(file, sent)| (file.as_str(), sent))
            .collect();
        // RFC 3339 in UTC sorts as text; the file name breaks ties.
        all.sort_by(|a, b| (&b.1.at, b.0).cmp(&(&a.1.at, a.0)));
        all
    }
}

/// The record in `path`, empty when the file doesn't exist yet.
pub fn load(path: &Path) -> Result<Record, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{} isn't a valid upload record: {e}", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Record::default()),
        Err(e) => Err(format!("Couldn't read {}: {e}", path.display())),
    }
}

/// Writes `record` to `path` if it changed since it was last written. A write that fails leaves
/// it changed, so the next call tries again.
pub fn save(path: &Path, record: &mut Record) -> Result<(), String> {
    if record.dirty {
        settings::save_json(path, record)?;
        record.dirty = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use crate::server::UploadedMatch;

    fn sent(size: u64, at: &str) -> Sent {
        Sent {
            size,
            match_ends: 1,
            at: at.into(),
            players: vec![],
            answer: Answer::Refused {
                error: "not_ranked".into(),
                message: "x".into(),
            },
            tourneys: vec![],
        }
    }

    #[test]
    fn knows_which_tourney_lobbies_had_their_match_uploaded() {
        let mut record = Record::default();
        let server = "https://genjiball.us";
        let tourney = |key: &str, ended| SentTourney {
            lobby_key: key.into(),
            ended,
        };
        record.put(
            server,
            "Log-a.txt",
            Sent {
                tourneys: vec![tourney("111", false)],
                ..answered("2026-10-03T10:00:00Z", &["1"], "accepted")
            },
        );
        // Being played: not ended yet.
        assert!(!record.tourney_ended(server, "111"));
        record.put(
            server,
            "Log-b.txt",
            Sent {
                tourneys: vec![tourney("111", true)],
                ..answered("2026-10-03T11:00:00Z", &["1"], "accepted")
            },
        );
        assert!(record.tourney_ended(server, "111"));
        assert!(!record.tourney_ended(server, "222"));
        assert!(!record.tourney_ended("https://test.genjiball.us", "111"));
        // Refused by the server: not uploaded.
        record.put(
            server,
            "Log-c.txt",
            Sent {
                tourneys: vec![tourney("333", true)],
                ..sent(1, "2026-10-03T12:00:00Z")
            },
        );
        assert!(!record.tourney_ended(server, "333"));
        // Kept in uploads.json, and left out of it for a ranked file.
        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains(r#""tourneys":[{"lobbyKey":"111","ended":false}]"#));
        let back: Record = serde_json::from_str(&json).unwrap();
        assert!(back.tourney_ended(server, "111"));
        let ranked = serde_json::to_string(&sent(1, "x")).unwrap();
        assert!(!ranked.contains("tourneys"), "{ranked}");
    }

    #[test]
    fn keeps_each_server_apart() {
        let mut record = Record::default();
        record.put(
            "https://genjiball.us",
            "Log-a.txt",
            sent(10, "2026-10-03T10:00:00Z"),
        );
        assert_eq!(
            record
                .get("https://genjiball.us", "Log-a.txt")
                .unwrap()
                .size,
            10
        );
        assert_eq!(record.get("https://test.genjiball.us", "Log-a.txt"), None);
        record.put(
            "https://genjiball.us",
            "Log-a.txt",
            sent(20, "2026-10-03T11:00:00Z"),
        );
        assert_eq!(
            record
                .get("https://genjiball.us", "Log-a.txt")
                .unwrap()
                .size,
            20
        );
    }

    #[test]
    fn lists_the_newest_first() {
        let mut record = Record::default();
        let server = "https://genjiball.us";
        record.put(server, "Log-a.txt", sent(1, "2026-10-03T10:00:00Z"));
        record.put(server, "Log-b.txt", sent(1, "2026-10-03T12:00:00Z"));
        record.put(server, "Log-c.txt", sent(1, "2026-10-03T11:00:00Z"));
        // The same time: the file name breaks the tie.
        record.put(server, "Log-d.txt", sent(1, "2026-10-03T11:00:00Z"));
        let files: Vec<_> = record
            .uploads(server)
            .into_iter()
            .map(|(file, _)| file)
            .collect();
        assert_eq!(files, ["Log-b.txt", "Log-d.txt", "Log-c.txt", "Log-a.txt"]);
        assert!(record.uploads("https://other").is_empty());
    }

    fn answered(at: &str, keys: &[&str], status: &str) -> Sent {
        Sent {
            answer: Answer::Answered(UploadAnswer {
                result: "stored".into(),
                region: Some("eu".into()),
                matches: keys
                    .iter()
                    .map(|key| UploadedMatch {
                        match_key: Some(key.to_string()),
                        match_id: None,
                        line_count: 10,
                        region: Some("eu".into()),
                        action: "insert".into(),
                        status: status.into(),
                        rejection: None,
                        review_reasons: vec!["untrusted_host".into()],
                    })
                    .collect(),
            }),
            ..sent(1, at)
        }
    }

    #[test]
    fn knows_each_match_once_with_its_id() {
        let mut record = Record::default();
        let server = "https://genjiball.us";
        record.put(
            server,
            "Log-a.txt",
            answered("2026-10-03T10:00:00Z", &["1"], "accepted"),
        );
        record.put(
            server,
            "Log-b.txt",
            answered("2026-10-03T11:00:00Z", &["1", "2"], "accepted"),
        );
        record.update_states(
            server,
            &[MatchState {
                match_key: "1".into(),
                match_id: Some(40),
                status: "accepted".into(),
                rejection: None,
                review_reasons: vec![],
            }],
        );
        let known = record.known_matches(server);
        let keys: Vec<_> = known.iter().map(|k| k.match_key.as_str()).collect();
        assert_eq!(keys, ["2", "1"]);
        assert_eq!(known[1].match_id, Some(40));
        assert_eq!(known[1].first_at, "2026-10-03T10:00:00Z");
        assert!(record.known_matches("https://other").is_empty());
    }

    #[test]
    fn lists_the_recent_match_keys_once() {
        let mut record = Record::default();
        let server = "https://genjiball.us";
        record.put(
            server,
            "Log-a.txt",
            answered("2026-10-03T10:00:00Z", &["1"], "review"),
        );
        // A spectator copy of the same match, and another match.
        record.put(
            server,
            "Log-b.txt",
            answered("2026-10-03T11:00:00Z", &["1", "2"], "review"),
        );
        record.put(server, "Log-c.txt", sent(1, "2026-10-03T12:00:00Z"));
        // Newest first: Log-b's matches before Log-a's, and Log-b's last match first.
        assert_eq!(record.recent_match_keys(server, 50), ["2", "1"]);
        assert_eq!(record.recent_match_keys(server, 1), ["2"]);
        assert_eq!(
            record.recent_match_keys("https://other", 50),
            Vec::<String>::new()
        );
    }

    #[test]
    fn refreshes_the_newest_matches_of_a_long_file() {
        let mut record = record_with_many_matches();
        let server = "https://genjiball.us";
        let keys = record.recent_match_keys(server, config::MAX_STATUS_KEYS);
        assert_eq!(keys.len(), config::MAX_STATUS_KEYS);
        // The newest file's last match first, down to its 11th: its first 10 are left out.
        assert_eq!(keys[0], "m59");
        assert_eq!(keys[config::MAX_STATUS_KEYS - 1], "m10");
        // Spectator copies of the newest matches don't take a place twice.
        record.put(
            server,
            "Log-c.txt",
            answered("2026-10-03T12:00:00Z", &["m58", "m59"], "review"),
        );
        assert_eq!(
            record.recent_match_keys(server, config::MAX_STATUS_KEYS),
            keys
        );
    }

    /// An older file with match `m0`, then a file with 60 matches, `m0` (a copy) to `m59`.
    fn record_with_many_matches() -> Record {
        let mut record = Record::default();
        let server = "https://genjiball.us";
        record.put(
            server,
            "Log-a.txt",
            answered("2026-10-03T10:00:00Z", &["m0"], "review"),
        );
        let keys: Vec<String> = (0..60).map(|n| format!("m{n}")).collect();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        record.put(
            server,
            "Log-b.txt",
            answered("2026-10-03T11:00:00Z", &keys, "review"),
        );
        record
    }

    #[test]
    fn takes_the_servers_status_now() {
        let mut record = Record::default();
        let server = "https://genjiball.us";
        record.put(
            server,
            "Log-a.txt",
            answered("2026-10-03T10:00:00Z", &["1"], "review"),
        );
        record.put(
            server,
            "Log-b.txt",
            answered("2026-10-03T11:00:00Z", &["1", "2"], "review"),
        );
        let accepted = MatchState {
            match_key: "1".into(),
            match_id: None,
            status: "accepted".into(),
            rejection: None,
            review_reasons: vec!["untrusted_host".into()],
        };
        assert!(record.update_states(server, std::slice::from_ref(&accepted)));
        assert!(!record.update_states(server, std::slice::from_ref(&accepted)));
        let matches = |record: &Record| -> Vec<UploadedMatch> {
            record
                .uploads(server)
                .into_iter()
                .flat_map(|(_, sent)| match &sent.answer {
                    Answer::Answered(a) => a.matches.clone(),
                    Answer::Refused { .. } => vec![],
                })
                .collect()
        };
        let statuses: Vec<_> = matches(&record)
            .into_iter()
            .map(|m| (m.match_key.unwrap(), m.status))
            .collect();
        assert_eq!(
            statuses,
            [
                ("1".to_string(), "accepted".to_string()),
                ("2".to_string(), "review".to_string()),
                ("1".to_string(), "accepted".to_string()),
            ]
        );

        // The site's id, once the server gives it, and kept when a server doesn't.
        let with_id = MatchState {
            match_id: Some(40),
            ..accepted.clone()
        };
        assert!(record.update_states(server, std::slice::from_ref(&with_id)));
        assert!(!record.update_states(server, &[accepted]));
        let ids: Vec<_> = matches(&record).into_iter().map(|m| m.match_id).collect();
        assert_eq!(ids, [Some(40), None, Some(40)]);
    }

    #[test]
    fn starts_each_server_once() {
        let mut record = Record::default();
        let first = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000);
        assert_eq!(record.started("https://genjiball.us"), None);
        assert!(record.start("https://genjiball.us", first));
        assert!(!record.start("https://genjiball.us", first + Duration::from_secs(60)));
        assert_eq!(record.started("https://genjiball.us"), Some(first));
        assert_eq!(record.started("https://test.genjiball.us"), None);
    }

    #[test]
    fn saves_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uploads.json");
        assert_eq!(load(&path).unwrap(), Record::default());
        let mut record = Record::default();
        record.put(
            "https://genjiball.us",
            "Log-a.txt",
            Sent {
                answer: Answer::Answered(UploadAnswer {
                    result: "duplicate".into(),
                    region: None,
                    matches: vec![],
                }),
                ..sent(5, "2026-10-03T10:00:00Z")
            },
        );
        record.start("https://genjiball.us", SystemTime::now());
        save(&path, &mut record).unwrap();
        assert_eq!(load(&path).unwrap(), record);
        fs::write(&path, "{ broken").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn a_failed_save_is_tried_again_until_it_works() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the folder should be: every write fails.
        let folder = dir.path().join("config");
        fs::write(&folder, "").unwrap();
        let path = folder.join("uploads.json");
        let mut record = Record::default();
        assert!(save(&path, &mut record).is_ok(), "nothing to save");

        record.start("https://genjiball.us", SystemTime::now());
        assert!(save(&path, &mut record).is_err());
        // Still not saved: the next try writes it, and only that one.
        assert!(save(&path, &mut record).is_err());
        fs::remove_file(&folder).unwrap();
        save(&path, &mut record).unwrap();
        assert_eq!(load(&path).unwrap(), record);
        fs::remove_file(&path).unwrap();
        save(&path, &mut record).unwrap();
        assert!(!path.exists(), "saved already");

        // Every change is saved: an upload, and a status the server gave since.
        let server = "https://genjiball.us";
        record.put(
            server,
            "Log-a.txt",
            answered("2026-10-03T10:00:00Z", &["1"], "review"),
        );
        save(&path, &mut record).unwrap();
        assert_eq!(load(&path).unwrap(), record);
        let accepted = MatchState {
            match_key: "1".into(),
            match_id: Some(3),
            status: "accepted".into(),
            rejection: None,
            review_reasons: vec![],
        };
        record.update_states(server, &[accepted]);
        save(&path, &mut record).unwrap();
        assert_eq!(load(&path).unwrap(), record);
    }

    #[test]
    fn knows_which_matches_are_on_the_site() {
        let mut record = Record::default();
        let server = "https://genjiball.us";
        record.put(
            server,
            "Log-a.txt",
            answered("2026-10-03T10:00:00Z", &["1"], "review"),
        );
        record.put(
            server,
            "Log-b.txt",
            answered("2026-10-03T11:00:00Z", &["2"], "accepted"),
        );
        let ids = |key: &str, id: i64, status: &str| MatchState {
            match_key: key.into(),
            match_id: Some(id),
            status: status.into(),
            rejection: None,
            review_reasons: vec![],
        };
        record.update_states(server, &[ids("1", 7, "review"), ids("2", 8, "accepted")]);
        // In review: not public yet.
        assert!(!record.has_public_match(server, 7));
        assert!(record.has_public_match(server, 8));
        assert!(!record.has_public_match(server, 9));
        assert!(!record.has_public_match("https://test.genjiball.us", 8));
    }

    #[test]
    fn loads_a_record_from_before_the_history() {
        // No `players` and no `matchId`: written by an older version of the tool.
        let old = r#"{"servers":{"https://genjiball.us":{"Log-a.txt":{"size":5,"matchEnds":1,"at":"2026-10-03T10:00:00Z","answer":{"kind":"answered","result":"stored","matches":[{"matchKey":"1","lineCount":9,"action":"insert","status":"accepted","rejection":null,"reviewReasons":[]}]}}}},"started":{}}"#;
        let record: Record = serde_json::from_str(old).unwrap();
        let sent = record.get("https://genjiball.us", "Log-a.txt").unwrap();
        assert!(sent.players.is_empty());
        let Answer::Answered(answer) = &sent.answer else {
            panic!()
        };
        assert_eq!(answer.matches[0].match_id, None);
    }
}
