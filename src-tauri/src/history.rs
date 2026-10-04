//! The upload history the window lists, a page at a time: the ranked files waiting to be uploaded
//! (a match being played, one due, one that failed and waits to be retried), then every upload in
//! `uploads.json` to the current server, newest first. One line per file.

use serde::Serialize;

use crate::uploads::{Answer, Record};
use crate::watcher::{QueueState, Queued};

/// One line of the history.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub file: String,
    /// When it was last uploaded, RFC 3339 in UTC. `None` while it never was.
    pub at: Option<String>,
    /// The players' names in the file (`JOIN` lines).
    pub players: Vec<String>,
    /// What the server said to the last upload, with the match status since.
    pub answer: Option<Answer>,
    /// Not uploaded as it is now: being played, due, or failed. A file uploaded before that grew
    /// since has both.
    pub queued: Option<QueueState>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub entries: Vec<Entry>,
    /// Which page this is, from 0 (the newest).
    pub page: usize,
    pub page_size: usize,
    /// Lines on every page together.
    pub total: usize,
}

/// Page `page` (from 0) of the history of uploads to `server_url`, `page_size` lines a page. A
/// page past the end gives the last one.
pub fn page(
    record: &Record,
    server_url: &str,
    queue: &[Queued],
    page: usize,
    page_size: usize,
) -> Page {
    let page_size = page_size.max(1);
    let uploads = record.uploads(server_url);
    let queued_files = |file: &str| queue.iter().any(|q| q.file == file);
    let total = queue.len() + uploads.iter().filter(|(f, _)| !queued_files(f)).count();
    let page = page.min(total.saturating_sub(1) / page_size);

    // The queue first, newest file (the name holds when it was started) first.
    let waiting = queue.iter().rev().map(|q| {
        let sent = record.get(server_url, &q.file);
        Entry {
            file: q.file.clone(),
            at: sent.map(|s| s.at.clone()),
            players: match sent {
                Some(s) if q.players.is_empty() => s.players.clone(),
                _ => q.players.clone(),
            },
            answer: sent.map(|s| s.answer.clone()),
            queued: Some(q.state.clone()),
        }
    });
    let done = uploads
        .into_iter()
        .filter(|(f, _)| !queued_files(f))
        .map(|(file, sent)| Entry {
            file: file.to_string(),
            at: Some(sent.at.clone()),
            players: sent.players.clone(),
            answer: Some(sent.answer.clone()),
            queued: None,
        });
    Page {
        entries: waiting
            .chain(done)
            .skip(page * page_size)
            .take(page_size)
            .collect(),
        page,
        page_size,
        total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{UploadAnswer, UploadedMatch};
    use crate::uploads::Sent;

    const SERVER: &str = "https://genjiball.us";

    fn sent(at: &str, status: &str) -> Sent {
        Sent {
            size: 10,
            match_ends: 1,
            at: at.into(),
            players: vec!["Sparrow".into(), "Tidal".into()],
            answer: Answer::Answered(UploadAnswer {
                result: "stored".into(),
                matches: vec![UploadedMatch {
                    match_key: Some("1".into()),
                    match_id: Some(4),
                    line_count: 9,
                    action: "insert".into(),
                    status: status.into(),
                    rejection: None,
                    review_reasons: vec![],
                }],
            }),
        }
    }

    fn queued(file: &str, state: QueueState) -> Queued {
        Queued {
            file: file.into(),
            players: vec!["Mochi".into()],
            state,
        }
    }

    fn files(page: &Page) -> Vec<&str> {
        page.entries.iter().map(|e| e.file.as_str()).collect()
    }

    /// Five uploads, `Log-1` the oldest.
    fn record() -> Record {
        let mut record = Record::default();
        for n in 1..=5 {
            record.put(
                SERVER,
                &format!("Log-{n}.txt"),
                sent(&format!("2026-10-03T1{n}:00:00Z"), "accepted"),
            );
        }
        record
    }

    #[test]
    fn lists_the_newest_first_a_page_at_a_time() {
        let record = record();
        let first = page(&record, SERVER, &[], 0, 2);
        assert_eq!(files(&first), ["Log-5.txt", "Log-4.txt"]);
        assert_eq!((first.page, first.page_size, first.total), (0, 2, 5));
        assert_eq!(
            files(&page(&record, SERVER, &[], 1, 2)),
            ["Log-3.txt", "Log-2.txt"]
        );
        assert_eq!(files(&page(&record, SERVER, &[], 2, 2)), ["Log-1.txt"]);
        // Past the end: the last page.
        let past = page(&record, SERVER, &[], 9, 2);
        assert_eq!((files(&past), past.page), (vec!["Log-1.txt"], 2));
        // Another server's uploads aren't listed.
        assert_eq!(
            page(&record, "https://other", &[], 0, 2),
            Page {
                page_size: 2,
                ..Page::default()
            }
        );
    }

    #[test]
    fn keeps_what_the_server_said() {
        let entry = &page(&record(), SERVER, &[], 0, 1).entries[0];
        assert_eq!(entry.at.as_deref(), Some("2026-10-03T15:00:00Z"));
        assert_eq!(entry.players, ["Sparrow", "Tidal"]);
        assert_eq!(entry.queued, None);
        let Some(Answer::Answered(answer)) = &entry.answer else {
            panic!()
        };
        assert_eq!(answer.matches[0].match_id, Some(4));
    }

    #[test]
    fn lists_the_queue_first_once_per_file() {
        let record = record();
        let failed = QueueState::Failed {
            error: "offline".into(),
        };
        // The watcher lists the queue oldest first. Log-3 was uploaded and has grown since.
        let queue = [
            queued("Log-3.txt", QueueState::Playing),
            queued("Log-6.txt", failed.clone()),
        ];
        let first = page(&record, SERVER, &queue, 0, 3);
        assert_eq!(files(&first), ["Log-6.txt", "Log-3.txt", "Log-5.txt"]);
        assert_eq!(first.total, 6);

        let failed_entry = &first.entries[0];
        assert_eq!(failed_entry.queued, Some(failed));
        assert_eq!((&failed_entry.at, &failed_entry.answer), (&None, &None));
        assert_eq!(failed_entry.players, ["Mochi"]);

        // The grown file: the new state, the last answer and its players now.
        let grown = &first.entries[1];
        assert_eq!(grown.queued, Some(QueueState::Playing));
        assert!(grown.answer.is_some());
        assert_eq!(grown.players, ["Mochi"]);

        assert_eq!(
            files(&page(&record, SERVER, &queue, 1, 3)),
            ["Log-4.txt", "Log-2.txt", "Log-1.txt"]
        );
    }

    #[test]
    fn a_zero_page_size_still_lists() {
        assert_eq!(files(&page(&record(), SERVER, &[], 0, 0)), ["Log-5.txt"]);
    }
}
