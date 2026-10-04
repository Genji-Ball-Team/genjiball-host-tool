//! The settings file: what the host chose, with `None` meaning "use the default".
//! The host token is never here: it lives in the OS credential store (`credentials.rs`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use url::{Host, Url};

use crate::config;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Overrides `config::DEFAULT_SERVER_URL`. Stored normalized (`normalize_server_url`).
    pub server_url: Option<String>,
    /// Overrides the detected Workshop log folder.
    pub log_folder: Option<PathBuf>,
}

impl Settings {
    pub fn server_url(&self) -> &str {
        self.server_url
            .as_deref()
            .unwrap_or(config::DEFAULT_SERVER_URL)
    }
}

/// The settings in `path`, or the defaults when the file doesn't exist yet. A file that can't be
/// read as settings is an error rather than silently reset, so a typo doesn't lose the others.
/// So is a server URL the window wouldn't take (`normalize_server_url`): the token is sent there.
pub fn load(path: &Path) -> Result<Settings, String> {
    let mut settings: Settings = match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{} isn't valid settings: {e}", path.display()))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(format!("Couldn't read {}: {e}", path.display())),
    };
    if let Some(url) = &settings.server_url {
        settings.server_url = normalize_server_url(url)
            .map_err(|e| format!("The server URL in {} won't do ({e})", path.display()))?;
    }
    Ok(settings)
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    save_json(path, settings)
}

/// Writes `value` to a temporary file next to `path`, then moves it over, so a crash mid-write
/// never leaves half a file. Every JSON file the tool keeps is written through here. The
/// temporary file's name is this write's own, so two writes at once never share one.
pub fn save_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let write = WRITES.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.{}-{write}.tmp", std::process::id()));
    fs::write(&tmp, text).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Couldn't write {}: {e}", path.display())
    })
}

/// A server URL as the host typed it, as `scheme://host[:port][/path]` without a trailing slash,
/// or why it's not one. Empty means "use the default" (`None`).
///
/// The token is sent to this URL, so it must be `https://`, except `http://` to this PC
/// (`localhost` or a loopback address) for a local `wrangler dev`. The host is checked on the
/// parsed URL, so `http://localhost@example.com` (host `example.com`) is refused.
pub fn normalize_server_url(input: &str) -> Result<Option<String>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let not_a_url = || "That isn't a server URL".to_string();
    let url = Url::parse(input).map_err(|_| {
        if input.contains("://") {
            not_a_url()
        } else {
            "The server URL starts with https://".to_string()
        }
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(not_a_url());
    }
    let host = url.host().ok_or_else(not_a_url)?;
    match url.scheme() {
        "https" => {}
        "http" if is_this_pc(&host) => {}
        "http" => {
            return Err(
                "Use https://. Plain http:// is only for a server on this PC (localhost)".into(),
            )
        }
        _ => return Err("The server URL starts with https://".into()),
    }
    let normalized = url.as_str().trim_end_matches('/').to_string();
    Ok(if normalized == config::DEFAULT_SERVER_URL {
        None
    } else {
        Some(normalized)
    })
}

fn is_this_pc(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => name.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = load(&dir.path().join("settings.json")).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.server_url(), config::DEFAULT_SERVER_URL);
    }

    #[test]
    fn saves_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let settings = Settings {
            server_url: Some("http://localhost:8787".into()),
            log_folder: Some(PathBuf::from(r"D:\Logs")),
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        // No temporary file left behind.
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn writes_at_the_same_time_dont_share_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uploads.json");
        let writers: Vec<_> = (0..8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        // Renaming over a file another write is renaming over may be refused on
                        // Windows; what mustn't happen is half a file or another write's text.
                        let _ = save_json(&path, &vec![n; 1000]);
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        let written: Vec<u32> = serde_json::from_str(&text).unwrap();
        assert!(written.iter().all(|&n| n == written[0]) && written.len() == 1000);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn ignores_unknown_and_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{ "serverUrl": "http://localhost:8787", "later": 1 }"#,
        )
        .unwrap();
        assert_eq!(load(&path).unwrap().server_url(), "http://localhost:8787");
    }

    #[test]
    fn checks_the_server_url_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // Edited by hand: plain http to another PC would send it the token unencrypted.
        for bad in [
            "http://192.168.1.20:8787",
            "https://user:pass@genjiball.us",
            "nope",
        ] {
            fs::write(&path, format!(r#"{{ "serverUrl": "{bad}" }}"#)).unwrap();
            assert!(load(&path).is_err(), "{bad}");
        }
        // Stored as the window would have: normalized, and the default as `None`.
        fs::write(&path, r#"{ "serverUrl": "https://test.genjiball.us/" }"#).unwrap();
        assert_eq!(
            load(&path).unwrap().server_url.as_deref(),
            Some("https://test.genjiball.us")
        );
        fs::write(&path, r#"{ "serverUrl": "https://genjiball.us" }"#).unwrap();
        assert_eq!(load(&path).unwrap().server_url, None);
    }

    #[test]
    fn a_broken_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{ not json").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn normalizes_server_urls() {
        assert_eq!(
            normalize_server_url(" https://test.genjiball.us/ "),
            Ok(Some("https://test.genjiball.us".into()))
        );
        assert_eq!(
            normalize_server_url("http://localhost:8787"),
            Ok(Some("http://localhost:8787".into()))
        );
        assert_eq!(normalize_server_url(""), Ok(None));
        assert_eq!(normalize_server_url("https://genjiball.us/"), Ok(None));
        assert!(normalize_server_url("genjiball.us").is_err());
        assert!(normalize_server_url("https://").is_err());
        assert!(normalize_server_url("https://genji ball.us").is_err());
    }

    #[test]
    fn allows_plain_http_only_to_this_pc() {
        for local in [
            "http://localhost:8787",
            "http://LOCALHOST:8787",
            "http://127.0.0.1:8787",
            "http://[::1]:8787",
        ] {
            assert!(normalize_server_url(local).is_ok(), "{local}");
        }
        for remote in [
            "http://genjiball.us",
            "http://192.168.1.20:8787",
            "http://localhost.example.com",
            // The host here is example.com: "localhost" is a user name.
            "http://localhost@example.com",
            "http://localhost:secret@example.com",
        ] {
            assert!(normalize_server_url(remote).is_err(), "{remote}");
        }
    }

    #[test]
    fn refuses_credentials_queries_and_other_schemes() {
        for bad in [
            "https://user:pass@genjiball.us",
            "https://genjiball.us/?x=1",
            "https://genjiball.us/#x",
            "ftp://genjiball.us",
            "file:///C:/x",
        ] {
            assert!(normalize_server_url(bad).is_err(), "{bad}");
        }
    }
}
