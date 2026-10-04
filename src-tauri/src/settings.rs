//! The settings file: what the host chose, with `None` meaning "use the default".
//! The host token is never here: it lives in the OS credential store (`credentials.rs`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
pub fn load(path: &Path) -> Result<Settings, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{} isn't valid settings: {e}", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!("Couldn't read {}: {e}", path.display())),
    }
}

/// Writes the settings to a temporary file next to `path`, then moves it over, so a crash
/// mid-write never leaves half a file.
pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("Couldn't write {}: {e}", path.display()))
}

/// A server URL as the host typed it, trimmed and without a trailing slash, or why it's not one.
/// Empty means "use the default" (`None`).
pub fn normalize_server_url(input: &str) -> Result<Option<String>, String> {
    let url = input.trim().trim_end_matches('/');
    if url.is_empty() {
        return Ok(None);
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or("The server URL starts with https:// (or http:// for a local server)")?;
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return Err("That isn't a server URL".into());
    }
    Ok(if url == config::DEFAULT_SERVER_URL {
        None
    } else {
        Some(url.to_string())
    })
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
        assert!(!path.with_extension("json.tmp").exists());
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
}
