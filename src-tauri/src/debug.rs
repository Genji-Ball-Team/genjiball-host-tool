//! The debug panel (Advanced → Debug): the latest uploads and dry runs, each with what was sent (or
//! would have been) and what the server answered, and the live log's newest events. Memory only,
//! and never the token: `UploadInfo::headers` leaves it out.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::Path;

use chrono::{SecondsFormat, Utc};
use serde::Serialize;

use crate::server::{RawAnswer, UploadInfo, UploadOutcome, Uploaded};
use crate::{config, log_scan, watcher};

/// An upload, or in a dry run what it would have been.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    /// When it was made, RFC 3339 in UTC.
    pub at: String,
    pub server_url: String,
    pub file: String,
    /// Bytes of complete lines: what was sent.
    pub bytes: u64,
    /// The headers sent besides the token, as `[name, value]`.
    pub headers: Vec<(&'static str, String)>,
    /// The `matchKey`s in what was sent.
    pub match_keys: Vec<String>,
    pub outcome: Outcome,
}

impl Attempt {
    /// An upload of `size` bytes to `server_url`, with `info`'s headers, made now.
    pub fn new(
        server_url: &str,
        info: &UploadInfo,
        size: u64,
        match_keys: &[String],
        outcome: Outcome,
    ) -> Self {
        Self {
            at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            server_url: server_url.to_string(),
            file: info.file_name.to_string(),
            bytes: size,
            headers: info.headers(),
            match_keys: match_keys.to_vec(),
            outcome,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    /// A dry run: nothing was sent.
    DryRun,
    /// Not sent: over the server's size limit (`config::MAX_UPLOAD_BYTES`).
    TooLarge,
    /// The server answered.
    Answered { answer: RawAnswer },
    /// No answer (offline, timed out), and why.
    Unanswered { message: String },
}

impl Outcome {
    pub fn of(uploaded: &Uploaded) -> Self {
        match (&uploaded.answer, &uploaded.outcome) {
            (Some(answer), _) => Outcome::Answered {
                answer: answer.clone(),
            },
            (None, UploadOutcome::Retry { message, .. }) => Outcome::Unanswered {
                message: message.clone(),
            },
            // Every other outcome comes from an answer.
            (None, outcome) => Outcome::Unanswered {
                message: format!("{outcome:?}"),
            },
        }
    }
}

/// The latest `config::DEBUG_UPLOADS_KEPT` attempts, newest first.
#[derive(Debug, Default)]
pub struct Recent(VecDeque<Attempt>);

impl Recent {
    pub fn push(&mut self, attempt: Attempt) {
        self.0.push_front(attempt);
        self.0.truncate(config::DEBUG_UPLOADS_KEPT);
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Attempt> {
        self.0.iter().cloned().collect()
    }
}

/// The live log's newest events: its last complete lines, oldest first.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveEvents {
    pub file: String,
    pub lines: Vec<String>,
}

/// The last `count` complete lines of the live log (the newest in `folder`), `None` while there's
/// no log.
pub fn live_events(folder: &Path, count: usize) -> io::Result<Option<LiveEvents>> {
    let Some(path) = watcher::newest_log(folder)? else {
        return Ok(None);
    };
    let bytes = fs::read(&path)?;
    let text = String::from_utf8_lossy(log_scan::complete_lines(&bytes));
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    Ok(Some(LiveEvents {
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        lines: lines[lines.len().saturating_sub(count)..]
            .iter()
            .map(|l| l.to_string())
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(file: &str) -> Attempt {
        Attempt {
            at: "2026-10-06T12:00:00Z".into(),
            server_url: "https://genjiball.us".into(),
            file: file.into(),
            bytes: 1,
            headers: Vec::new(),
            match_keys: Vec::new(),
            outcome: Outcome::DryRun,
        }
    }

    #[test]
    fn keeps_the_newest_attempts_first() {
        let mut recent = Recent::default();
        for i in 0..config::DEBUG_UPLOADS_KEPT + 3 {
            recent.push(attempt(&format!("Log-{i}.txt")));
        }
        let list = recent.list();
        assert_eq!(list.len(), config::DEBUG_UPLOADS_KEPT);
        assert_eq!(
            list[0].file,
            format!("Log-{}.txt", config::DEBUG_UPLOADS_KEPT + 2)
        );
        assert_eq!(list.last().unwrap().file, "Log-3.txt");
    }

    #[test]
    fn reads_the_last_complete_lines_of_the_live_log() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(live_events(dir.path(), 2).unwrap(), None);
        fs::write(dir.path().join("Log-2026-10-02-19-00-00.txt"), "old\n").unwrap();
        let live = "Log-2026-10-02-20-15-33.txt";
        fs::write(dir.path().join(live), "a\nb\n\nc\nhalf").unwrap();
        assert_eq!(
            live_events(dir.path(), 2).unwrap(),
            Some(LiveEvents {
                file: live.into(),
                lines: vec!["b".into(), "c".into()],
            })
        );
        assert_eq!(live_events(dir.path(), 10).unwrap().unwrap().lines.len(), 3);
    }
}
