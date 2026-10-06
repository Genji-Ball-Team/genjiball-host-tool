//! A diagnostics export for bug reports (Advanced → Debug → "Export diagnostics"): one JSON file
//! with the app version and OS, `settings.json`, `uploads.json`, `afk.json`, the upload and update
//! status the window shows, the debug panel's latest uploads, the newest of the tool's own log
//! (up to `DIAGNOSTICS_LOG_BYTES`), and the names, sizes and times of the files in the Workshop log
//! folder and the tool's log folder (not the Workshop logs themselves).
//!
//! Never a token: `tokens.json` and the credential store aren't read, and every token passed in
//! `secrets` is blanked out of the whole export in case one slipped into a file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local, SecondsFormat};
use serde_json::{json, Value};

use crate::config;

/// What stands in for a token found in the export.
const REDACTED: &str = "[host token]";

/// Where the export's parts come from.
pub struct Sources<'a> {
    pub version: &'a str,
    /// The WebView2 version, if it could be read.
    pub webview: Option<String>,
    /// The app's config folder: `settings.json`, `uploads.json` and `afk.json` are read from it.
    pub config_dir: &'a Path,
    /// The tool's log folder, `None` if it isn't known.
    pub log_dir: Option<&'a Path>,
    /// The Workshop log folder in use, `None` when there's none.
    pub workshop_folder: Option<&'a Path>,
    /// What the window shows: `UploadStatus` and `UpdateStatus`, serialized.
    pub upload_status: Value,
    pub update_status: Value,
    /// The debug panel's latest uploads and dry runs (`debug::Attempt`), serialized.
    pub recent_uploads: Value,
}

/// The export, as pretty JSON, with every one of `secrets` blanked out. `now` is when it's made.
pub fn collect(sources: &Sources, secrets: &[String], now: DateTime<Local>) -> String {
    let file = |name| json_file(&sources.config_dir.join(name));
    let export = json!({
        "exportedAt": now.to_rfc3339_opts(SecondsFormat::Secs, false),
        "app": { "version": sources.version },
        "os": {
            "os": std::env::consts::OS,
            "family": std::env::consts::FAMILY,
            "arch": std::env::consts::ARCH,
            "webview": sources.webview,
        },
        "settings": file(config::SETTINGS_FILE),
        "uploads": file(config::UPLOADS_FILE),
        "afk": file(config::AFK_FILE),
        "uploadStatus": sources.upload_status,
        "updateStatus": sources.update_status,
        "recentUploads": sources.recent_uploads,
        "workshopFolder": sources.workshop_folder.map(folder),
        "logFolder": sources.log_dir.map(folder),
        "logs": sources.log_dir.map(|dir| own_logs(dir, config::DIAGNOSTICS_LOG_BYTES)),
    });
    let text = serde_json::to_string_pretty(&export).unwrap_or_default();
    redact(text, secrets)
}

/// `text` with every one of `secrets` (as is, and as it's escaped in JSON) replaced.
fn redact(mut text: String, secrets: &[String]) -> String {
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        let escaped = serde_json::to_string(secret).unwrap_or_default();
        let escaped = escaped.trim_matches('"');
        for form in [secret.as_str(), escaped] {
            if !form.is_empty() {
                text = text.replace(form, REDACTED);
            }
        }
    }
    text
}

/// A JSON file as JSON; one that isn't valid JSON as its text, and why it can't be read otherwise.
/// `null` when it isn't there.
fn json_file(path: &Path) -> Value {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .unwrap_or_else(|e| json!({ "invalidJson": e.to_string(), "text": text })),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Value::Null,
        Err(e) => json!({ "unreadable": e.to_string() }),
    }
}

/// A file in a folder listing.
struct Listed {
    name: String,
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

fn list(dir: &Path) -> io::Result<Vec<Listed>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            files.push(Listed {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
                size: meta.len(),
                modified: meta.modified().ok(),
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

fn time(at: Option<SystemTime>) -> Option<String> {
    at.map(|t| DateTime::<Local>::from(t).to_rfc3339_opts(SecondsFormat::Secs, false))
}

/// The folder's path and its files' names, sizes and times, not what's in them.
fn folder(dir: &Path) -> Value {
    match list(dir) {
        Ok(files) => json!({
            "path": dir,
            "files": files
                .iter()
                .map(|f| json!({ "name": f.name, "size": f.size, "modified": time(f.modified) }))
                .collect::<Vec<_>>(),
        }),
        Err(e) => json!({ "path": dir, "unreadable": e.to_string() }),
    }
}

/// The tool's own log files in `dir`, newest first, while they fit in `budget` bytes: the one
/// that doesn't fit is cut to its end, and older ones are left out.
fn own_logs(dir: &Path, budget: u64) -> Value {
    let mut files = match list(dir) {
        Ok(files) => files,
        Err(e) => return json!({ "unreadable": e.to_string() }),
    };
    files.retain(|f| f.name.starts_with(config::LOG_FILE_NAME) && f.name.ends_with(".log"));
    files.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.name.cmp(&a.name)));
    let mut left = budget;
    let mut logs = Vec::new();
    for file in files {
        if left == 0 {
            break;
        }
        let bytes = match fs::read(&file.path) {
            Ok(bytes) => bytes,
            Err(e) => {
                logs.push(json!({ "name": file.name, "unreadable": e.to_string() }));
                continue;
            }
        };
        let keep = (bytes.len() as u64).min(left) as usize;
        let cut = keep < bytes.len();
        let text = String::from_utf8_lossy(&bytes[bytes.len() - keep..]).into_owned();
        left -= keep as u64;
        logs.push(json!({ "name": file.name, "cut": cut, "text": text }));
    }
    Value::Array(logs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources<'a>(config_dir: &'a Path, log_dir: &'a Path, workshop: &'a Path) -> Sources<'a> {
        Sources {
            version: "0.3.0",
            webview: Some("140.0".into()),
            config_dir,
            log_dir: Some(log_dir),
            workshop_folder: Some(workshop),
            upload_status: json!({ "waiting": 1 }),
            update_status: json!({ "available": null }),
            recent_uploads: json!([]),
        }
    }

    fn export(sources: &Sources, secrets: &[String]) -> Value {
        serde_json::from_str(&collect(sources, secrets, Local::now())).unwrap()
    }

    #[test]
    fn holds_the_files_and_listings() {
        let dir = tempfile::tempdir().unwrap();
        let (config_dir, logs, workshop) = (
            dir.path().join("config"),
            dir.path().join("logs"),
            dir.path().join("Workshop"),
        );
        for d in [&config_dir, &logs, &workshop] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(
            config_dir.join(config::SETTINGS_FILE),
            r#"{ "region": "eu" }"#,
        )
        .unwrap();
        fs::write(config_dir.join(config::AFK_FILE), "{ broken").unwrap();
        fs::write(logs.join("host-tool.log"), "line one\n").unwrap();
        fs::write(
            workshop.join("Log-2026-10-02-20-15-33.txt"),
            "MATCH_START x\n",
        )
        .unwrap();

        let got = export(&sources(&config_dir, &logs, &workshop), &[]);
        assert_eq!(got["app"]["version"], "0.3.0");
        assert_eq!(got["settings"]["region"], "eu");
        assert_eq!(got["uploads"], Value::Null);
        assert_eq!(got["afk"]["text"], "{ broken");
        assert_eq!(got["uploadStatus"]["waiting"], 1);
        assert_eq!(got["logs"][0]["text"], "line one\n");
        let listed = &got["workshopFolder"]["files"][0];
        assert_eq!(listed["name"], "Log-2026-10-02-20-15-33.txt");
        assert_eq!(listed["size"], 14);
        assert!(listed["modified"].is_string());
        // The Workshop logs are listed, not read.
        assert!(!got.to_string().contains("MATCH_START"));
    }

    #[test]
    fn keeps_the_newest_log_within_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("host-tool_2026-10-01.log"), "old old\n").unwrap();
        // Newer, by name when the times are the same.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(dir.path().join("host-tool.log"), "newest\n").unwrap();
        fs::write(dir.path().join("other.txt"), "not ours").unwrap();
        let logs = own_logs(dir.path(), 10);
        assert_eq!(
            logs,
            json!([
                { "name": "host-tool.log", "cut": false, "text": "newest\n" },
                { "name": "host-tool_2026-10-01.log", "cut": true, "text": "ld\n" },
            ])
        );
    }

    #[test]
    fn blanks_out_tokens() {
        let text = r#"{"a":"Bearer abc\"def","b":"abc"}"#.to_string();
        assert_eq!(
            redact(text, &["abc\"def".into(), String::new()]),
            r#"{"a":"Bearer [host token]","b":"abc"}"#
        );
    }

    /// A token saved through the credential store, and one in the DPAPI fallback file, are in no
    /// export, even when one ends up in a file that's exported.
    #[cfg(windows)]
    #[test]
    fn never_holds_a_token() {
        use crate::credentials::fake::{self, FakeStore};

        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path();
        let logs = dir.path().join("logs");
        fs::create_dir_all(&logs).unwrap();
        let in_store = "store-token-0123456789";
        let in_file = "file-token-9876543210";
        fake::tokens(
            FakeStore::default(),
            config_dir.join(config::TOKENS_FALLBACK_FILE),
        )
        .set("https://genjiball.us", in_store)
        .unwrap();
        let full = fake::tokens(
            FakeStore {
                full: true,
                ..FakeStore::default()
            },
            config_dir.join(config::TOKENS_FALLBACK_FILE),
        );
        full.set("https://test.genjiball.us", in_file).unwrap();
        let sealed: std::collections::BTreeMap<String, String> = serde_json::from_str(
            &fs::read_to_string(config_dir.join(config::TOKENS_FALLBACK_FILE)).unwrap(),
        )
        .unwrap();
        // A token that slipped into the log and the settings.
        fs::write(logs.join("host-tool.log"), format!("oops {in_file}\n")).unwrap();
        fs::write(
            config_dir.join(config::SETTINGS_FILE),
            format!(r#"{{ "note": "{in_store}" }}"#),
        )
        .unwrap();

        let secrets = [in_store.to_string(), in_file.to_string()];
        let text = collect(&sources(config_dir, &logs, &logs), &secrets, Local::now());
        assert!(!text.contains(in_store) && !text.contains(in_file));
        assert!(sealed.values().all(|hex| !text.contains(hex.as_str())));
        assert!(!text.contains(config::TOKENS_FALLBACK_FILE));
        assert!(text.contains("oops [host token]"));
    }
}
