//! Where to watch for Workshop logs: the host's own folder if they picked one, else
//! `Documents\Overwatch\Workshop`. Documents comes from Windows' known folder, so a Documents
//! folder moved to another drive or into OneDrive is found too.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// The host picked it.
    Custom,
    /// `Documents\Overwatch\Workshop`.
    Detected,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFolder {
    pub path: PathBuf,
    pub source: Source,
    /// Whether the folder is there. Overwatch creates it the first time the Workshop writes a log.
    pub exists: bool,
}

/// `Documents\Overwatch\Workshop` under the given Documents folder.
pub fn workshop_folder(documents: &Path) -> PathBuf {
    config::WORKSHOP_LOG_SUBFOLDER
        .iter()
        .fold(documents.to_path_buf(), |path, part| path.join(part))
}

/// The folder to watch: `custom` if set, else the one under `documents`. `None` when there's no
/// custom folder and no Documents folder to look in.
pub fn resolve(custom: Option<&Path>, documents: Option<&Path>) -> Option<LogFolder> {
    let (path, source) = match custom {
        Some(path) => (path.to_path_buf(), Source::Custom),
        None => (workshop_folder(documents?), Source::Detected),
    };
    let exists = path.is_dir();
    Some(LogFolder {
        path,
        source,
        exists,
    })
}

/// `resolve` with this user's Documents folder.
pub fn current(custom: Option<&Path>) -> Option<LogFolder> {
    resolve(custom, dirs::document_dir().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_the_workshop_folder_under_documents() {
        let docs = tempfile::tempdir().unwrap();
        let workshop = docs.path().join("Overwatch").join("Workshop");
        fs::create_dir_all(&workshop).unwrap();
        assert_eq!(
            resolve(None, Some(docs.path())),
            Some(LogFolder {
                path: workshop,
                source: Source::Detected,
                exists: true
            })
        );
    }

    #[test]
    fn reports_a_workshop_folder_that_isnt_there_yet() {
        let docs = tempfile::tempdir().unwrap();
        let found = resolve(None, Some(docs.path())).unwrap();
        assert_eq!(found.source, Source::Detected);
        assert!(!found.exists);
    }

    #[test]
    fn a_custom_folder_wins() {
        let docs = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve(Some(custom.path()), Some(docs.path())),
            Some(LogFolder {
                path: custom.path().to_path_buf(),
                source: Source::Custom,
                exists: true
            })
        );
        assert_eq!(
            resolve(Some(custom.path()), None).unwrap().source,
            Source::Custom
        );
    }

    #[test]
    fn nothing_without_documents_or_a_custom_folder() {
        assert_eq!(resolve(None, None), None);
    }
}
