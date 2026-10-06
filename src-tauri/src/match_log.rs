//! A log's text for the window's match view (#16). The window parses it with the ranked server's
//! parser (`src/parser/`); this only reads the file. Only complete lines (`log_scan::complete_lines`),
//! as the uploader sends them, and only files in the log folder in use, named as Overwatch names them.

use std::fs;
use std::io;
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::{log_scan, watcher};

/// A log's complete lines, as the window gets them.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogText {
    pub file: String,
    /// Bytes of complete lines.
    pub size: u64,
    /// `None` when the window already has this file at this size (`Known`): nothing new to parse.
    pub text: Option<String>,
}

/// The file and size the window last read, so an unchanged log isn't sent again each poll.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Known {
    pub file: String,
    pub size: u64,
}

/// A plain Workshop log name: no folder, drive or `..`, so the window can't read anything else.
pub fn is_plain_log_name(name: &str) -> bool {
    log_scan::is_log_file(name) && !name.contains(['/', '\\', ':']) && !name.contains("..")
}

/// Log `file` in `folder`.
pub fn read(folder: &Path, file: &str, known: Option<&Known>) -> Result<LogText, String> {
    if !is_plain_log_name(file) {
        return Err(format!("\"{file}\" isn't a Workshop log name"));
    }
    let bytes = fs::read(folder.join(file)).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => {
            format!("{file} isn't in the log folder (deleted, or from another folder)")
        }
        // Locked for a moment while the game writes it: the next poll reads it.
        _ => format!("Couldn't read {file}: {e}"),
    })?;
    let complete = log_scan::complete_lines(&bytes);
    let size = complete.len() as u64;
    let unchanged = known.is_some_and(|k| k.file == file && k.size == size);
    Ok(LogText {
        file: file.to_string(),
        size,
        text: (!unchanged).then(|| String::from_utf8_lossy(complete).into_owned()),
    })
}

/// The live log's name: the newest in `folder` (`watcher::newest_log`), `None` while there's none.
fn live_name(folder: &Path) -> Result<Option<String>, String> {
    let newest =
        watcher::newest_log(folder).map_err(|e| format!("Couldn't read the log folder: {e}"))?;
    Ok(newest
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .map(str::to_string))
}

/// The live log, `None` while there's none.
pub fn read_live(folder: &Path, known: Option<&Known>) -> Result<Option<LogText>, String> {
    match live_name(folder)? {
        Some(name) => read(folder, &name, known).map(Some),
        None => Ok(None),
    }
}

/// The live log's name and when it last grew, for Home (#34): no text.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveFile {
    pub file: String,
    /// RFC 3339.
    pub written_at: String,
}

/// The live log without its text, `None` while there's none.
pub fn live_file(folder: &Path) -> Result<Option<LiveFile>, String> {
    let Some(file) = live_name(folder)? else {
        return Ok(None);
    };
    let modified = fs::metadata(folder.join(&file))
        .and_then(|m| m.modified())
        .map_err(|e| format!("Couldn't read {file}: {e}"))?;
    Ok(Some(LiveFile {
        file,
        written_at: DateTime::<Utc>::from(modified).to_rfc3339_opts(SecondsFormat::Secs, true),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "Log-2026-10-02-20-15-33.txt";

    #[test]
    fn takes_only_plain_log_names() {
        assert!(is_plain_log_name(NAME));
        for name in [
            "Log-../settings.txt",
            "Log-a/b.txt",
            "Log-a\\b.txt",
            "Log-C:x.txt",
            "settings.json",
            "Log-2026.txt.bak",
        ] {
            assert!(!is_plain_log_name(name), "{name}");
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(read(dir.path(), "../uploads.json", None).is_err());
    }

    #[test]
    fn reads_complete_lines_and_skips_an_unchanged_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(NAME), "GBR|1|1|1.3.3R|1\nMATCH_END|").unwrap();
        let first = read(dir.path(), NAME, None).unwrap();
        assert_eq!(first.text.as_deref(), Some("GBR|1|1|1.3.3R|1\n"));
        assert_eq!(first.size, 17);

        let known = Known {
            file: NAME.into(),
            size: first.size,
        };
        assert_eq!(read(dir.path(), NAME, Some(&known)).unwrap().text, None);
        // A half-written line finished: new text.
        fs::write(
            dir.path().join(NAME),
            "GBR|1|1|1.3.3R|1\nMATCH_END|9|TIME\n",
        )
        .unwrap();
        let grown = read(dir.path(), NAME, Some(&known)).unwrap();
        assert!(grown.text.unwrap().ends_with("TIME\n"));
    }

    #[test]
    fn a_missing_file_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let error = read(dir.path(), NAME, None).unwrap_err();
        assert!(error.contains("isn't in the log folder"), "{error}");
    }

    #[test]
    fn the_live_log_is_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_live(dir.path(), None).unwrap(), None);
        fs::write(dir.path().join("Log-2026-10-02-19-00-00.txt"), "old\n").unwrap();
        fs::write(dir.path().join(NAME), "new\n").unwrap();
        let live = read_live(dir.path(), None).unwrap().unwrap();
        assert_eq!(
            (live.file.as_str(), live.text.as_deref()),
            (NAME, Some("new\n"))
        );
        let file = live_file(dir.path()).unwrap().unwrap();
        assert_eq!(file.file, NAME);
        assert!(file.written_at.ends_with('Z'), "{}", file.written_at);
    }

    #[test]
    fn no_live_file_without_a_log() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("notes.txt"), "not a log\n").unwrap();
        assert_eq!(live_file(dir.path()).unwrap(), None);
    }
}
