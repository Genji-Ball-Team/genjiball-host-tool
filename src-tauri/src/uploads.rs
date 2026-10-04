//! The record of uploaded log files (`uploads.json`): per server URL and file name, the size that
//! was sent and what the server said. A file is uploaded again only when it's grown since, so a
//! restart doesn't send every file again, and a file that couldn't be sent (offline) still is.
//! Losing the record costs little: the server answers a file it already has with `duplicate`.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::server::UploadAnswer;
use crate::settings;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Record {
    /// Server URL → file name → the last upload of that file.
    servers: BTreeMap<String, BTreeMap<String, Sent>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sent {
    /// Bytes sent: the file is sent again once it's a different size.
    pub size: u64,
    /// `MATCH_END` lines in what was sent: a new one is sent straight away.
    pub match_ends: usize,
    /// When, RFC 3339 in UTC.
    pub at: String,
    pub answer: Answer,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Answer {
    /// The server took the upload (`200`).
    Answered(UploadAnswer),
    /// The server, or the tool before sending, refused the file (`413`, `422`).
    Refused { error: String, message: String },
}

/// One line of the window's upload list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentUpload {
    pub file: String,
    pub at: String,
    pub answer: Answer,
}

impl Record {
    pub fn get(&self, server_url: &str, file: &str) -> Option<&Sent> {
        self.servers.get(server_url)?.get(file)
    }

    pub fn put(&mut self, server_url: &str, file: &str, sent: Sent) {
        self.servers
            .entry(server_url.to_string())
            .or_default()
            .insert(file.to_string(), sent);
    }

    /// The `count` latest uploads to `server_url`, newest first.
    pub fn recent(&self, server_url: &str, count: usize) -> Vec<RecentUpload> {
        let mut all: Vec<_> = self.servers.get(server_url).into_iter().flatten().collect();
        // RFC 3339 in UTC sorts as text; the file name breaks ties.
        all.sort_by(|a, b| (&b.1.at, b.0).cmp(&(&a.1.at, a.0)));
        all.into_iter()
            .take(count)
            .map(|(file, sent)| RecentUpload {
                file: file.clone(),
                at: sent.at.clone(),
                answer: sent.answer.clone(),
            })
            .collect()
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

pub fn save(path: &Path, record: &Record) -> Result<(), String> {
    settings::save_json(path, record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sent(size: u64, at: &str) -> Sent {
        Sent {
            size,
            match_ends: 1,
            at: at.into(),
            answer: Answer::Refused {
                error: "not_ranked".into(),
                message: "x".into(),
            },
        }
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
        let files: Vec<_> = record
            .recent(server, 2)
            .into_iter()
            .map(|r| r.file)
            .collect();
        assert_eq!(files, ["Log-b.txt", "Log-c.txt"]);
        assert!(record.recent("https://other", 2).is_empty());
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
                    matches: vec![],
                }),
                ..sent(5, "2026-10-03T10:00:00Z")
            },
        );
        save(&path, &record).unwrap();
        assert_eq!(load(&path).unwrap(), record);
        fs::write(&path, "{ broken").unwrap();
        assert!(load(&path).is_err());
    }
}
