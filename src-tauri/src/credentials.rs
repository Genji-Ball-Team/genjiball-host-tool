//! The host token, one per server URL, so switching to the test server and back keeps both.
//! It never goes in a plain file, a log or an error message.
//!
//! Kept in the OS credential store (Windows Credential Manager). When that refuses to store it
//! (a Credential Manager filled by the Xbox app's tokens answers "not enough memory"), the token
//! goes in `tokens.json` in the app's config folder instead, encrypted with DPAPI for this
//! Windows user (`dpapi.rs`).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use keyring::{Entry, Error};

use crate::{config, dpapi, settings};

/// Where a token goes first: Windows Credential Manager (`Keyring`), or a stand-in in the tests.
pub trait CredentialStore {
    fn get(&self, server_url: &str) -> Result<Option<String>, String>;
    fn set(&self, server_url: &str, token: &str) -> Result<(), String>;
    fn delete(&self, server_url: &str) -> Result<(), String>;
}

pub struct Tokens<S = Keyring> {
    store: S,
    /// The DPAPI fallback file.
    fallback: PathBuf,
}

impl Tokens {
    pub fn new(fallback: PathBuf) -> Self {
        Tokens {
            store: Keyring,
            fallback,
        }
    }
}

impl<S: CredentialStore> Tokens<S> {
    pub fn get(&self, server_url: &str) -> Result<Option<String>, String> {
        // A fallback entry is authoritative until a native save removes it. The credential
        // store may have refused both a replacement and deletion of its older token.
        match read_file(&self.fallback)?.get(server_url) {
            Some(hex) => {
                let sealed =
                    from_hex(hex).ok_or("The saved host token is damaged. Enter it again")?;
                let token = dpapi::unprotect(&sealed).map_err(|e| {
                    format!("Couldn't read the saved host token ({e}). Enter it again")
                })?;
                String::from_utf8(token)
                    .map(Some)
                    .map_err(|_| "The saved host token is damaged. Enter it again".into())
            }
            // A native read error still falls through to no token, as it does when the
            // credential store cannot be reached from a non-interactive session.
            None => Ok(self.store.get(server_url).ok().flatten()),
        }
    }

    pub fn set(&self, server_url: &str, token: &str) -> Result<(), String> {
        match self.store.set(server_url, token) {
            Ok(()) => self.remove_from_file(server_url),
            Err(store_error) => {
                let sealed = dpapi::protect(token.as_bytes())
                    .map_err(|e| format!("Couldn't save the host token: {store_error}, and {e}"))?;
                let mut tokens = read_file(&self.fallback)?;
                tokens.insert(server_url.to_string(), to_hex(&sealed));
                write_file(&self.fallback, &tokens)?;
                // Clean up the older native token if possible. The saved fallback takes
                // precedence even when the credential store refuses deletion too.
                let _ = self.store.delete(server_url);
                Ok(())
            }
        }
    }

    pub fn delete(&self, server_url: &str) -> Result<(), String> {
        let stored = self.store.delete(server_url);
        self.remove_from_file(server_url)?;
        stored
    }

    fn remove_from_file(&self, server_url: &str) -> Result<(), String> {
        let mut tokens = read_file(&self.fallback)?;
        if tokens.remove(server_url).is_some() {
            write_file(&self.fallback, &tokens)?;
        }
        Ok(())
    }
}

/// The OS credential store (Windows Credential Manager).
pub struct Keyring;

fn entry(server_url: &str) -> Result<Entry, String> {
    Entry::new(config::CREDENTIAL_SERVICE, server_url)
        .map_err(|e| format!("the credential store isn't available ({e})"))
}

impl CredentialStore for Keyring {
    fn get(&self, server_url: &str) -> Result<Option<String>, String> {
        match entry(server_url)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("the credential store couldn't read it ({e})")),
        }
    }

    fn set(&self, server_url: &str, token: &str) -> Result<(), String> {
        entry(server_url)?
            .set_password(token)
            .map_err(|e| format!("the credential store refused it ({e})"))
    }

    fn delete(&self, server_url: &str) -> Result<(), String> {
        match entry(server_url)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(e) => Err(format!(
                "Couldn't remove the host token from the credential store: {e}"
            )),
        }
    }
}

/// Server URL → DPAPI-encrypted token, hex.
type FallbackFile = BTreeMap<String, String>;

fn read_file(path: &Path) -> Result<FallbackFile, String> {
    match fs::read_to_string(path) {
        // A damaged file only loses the tokens in it: they can be entered again.
        Ok(text) => Ok(serde_json::from_str(&text).unwrap_or_default()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(FallbackFile::new()),
        Err(e) => Err(format!("Couldn't read {}: {e}", path.display())),
    }
}

fn write_file(path: &Path, tokens: &FallbackFile) -> Result<(), String> {
    if tokens.is_empty() {
        return match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                Err(format!("Couldn't remove {}: {e}", path.display()))
            }
            _ => Ok(()),
        };
    }
    settings::save_json(path, tokens)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let bytes = [0u8, 1, 0xab, 0xff];
        assert_eq!(to_hex(&bytes), "0001abff");
        assert_eq!(from_hex("0001abff").unwrap(), bytes);
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn the_fallback_file_holds_one_token_per_server_and_goes_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.json");
        let mut tokens = FallbackFile::new();
        tokens.insert("http://localhost:8787".into(), "00ff".into());
        tokens.insert("https://test.genjiball.us".into(), "ab".into());
        write_file(&path, &tokens).unwrap();
        assert_eq!(read_file(&path).unwrap(), tokens);

        write_file(&path, &FallbackFile::new()).unwrap();
        assert!(!path.exists());
        assert!(read_file(&path).unwrap().is_empty());
    }

    #[test]
    fn a_damaged_fallback_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokens.json");
        fs::write(&path, "{ nope").unwrap();
        assert!(read_file(&path).unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_token_in_the_fallback_file_is_encrypted_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let tokens = Tokens::new(dir.path().join("tokens.json"));
        // A server URL no credential store entry exists for, written straight to the file.
        let url = "http://fallback-test.invalid";
        let mut file = FallbackFile::new();
        file.insert(
            url.into(),
            to_hex(&dpapi::protect(b"secret-token").unwrap()),
        );
        write_file(&tokens.fallback, &file).unwrap();

        assert!(!fs::read_to_string(&tokens.fallback)
            .unwrap()
            .contains("secret-token"));
        assert_eq!(tokens.get(url).unwrap().as_deref(), Some("secret-token"));
        tokens.remove_from_file(url).unwrap();
        assert_eq!(tokens.get(url).unwrap(), None);
    }

    /// A credential store that holds tokens in memory, and refuses new ones when `full`.
    #[cfg(windows)]
    #[derive(Default)]
    struct FakeStore {
        tokens: std::sync::Mutex<BTreeMap<String, String>>,
        full: bool,
        delete_fails: bool,
    }

    #[cfg(windows)]
    impl CredentialStore for FakeStore {
        fn get(&self, server_url: &str) -> Result<Option<String>, String> {
            Ok(self.tokens.lock().unwrap().get(server_url).cloned())
        }

        fn set(&self, server_url: &str, token: &str) -> Result<(), String> {
            if self.full {
                return Err("the credential store refused it (not enough memory)".into());
            }
            let mut tokens = self.tokens.lock().unwrap();
            tokens.insert(server_url.into(), token.into());
            Ok(())
        }

        fn delete(&self, server_url: &str) -> Result<(), String> {
            if self.delete_fails {
                return Err("the credential store refused deletion".into());
            }
            self.tokens.lock().unwrap().remove(server_url);
            Ok(())
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_new_token_in_the_fallback_file_replaces_the_one_in_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://genjiball.us";
        let store = FakeStore::default();
        store.set(url, "old-token").unwrap();
        // The credential store fills up (the Xbox app's tokens): the new token goes in the file.
        let tokens = Tokens {
            store: FakeStore {
                full: true,
                ..store
            },
            fallback: dir.path().join("tokens.json"),
        };
        tokens.set(url, "new-token").unwrap();
        assert_eq!(tokens.get(url).unwrap().as_deref(), Some("new-token"));
        assert_eq!(tokens.store.get(url).unwrap(), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_fallback_replacement_wins_even_when_native_deletion_fails() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://genjiball.us";
        let store = FakeStore::default();
        store.set(url, "old-token").unwrap();
        let tokens = Tokens {
            store: FakeStore {
                full: true,
                delete_fails: true,
                ..store
            },
            fallback: dir.path().join("tokens.json"),
        };
        tokens.set(url, "new-token").unwrap();
        assert_eq!(tokens.store.get(url).unwrap().as_deref(), Some("old-token"));
        assert_eq!(tokens.get(url).unwrap().as_deref(), Some("new-token"));
        // The choice survives a restart; there is no in-memory backend preference.
        let restarted = Tokens {
            store: tokens.store,
            fallback: tokens.fallback,
        };
        assert_eq!(restarted.get(url).unwrap().as_deref(), Some("new-token"));
    }

    #[cfg(windows)]
    #[test]
    fn a_token_the_store_takes_leaves_the_fallback_file() {
        let dir = tempfile::tempdir().unwrap();
        let url = "https://genjiball.us";
        let mut tokens = Tokens {
            store: FakeStore {
                full: true,
                ..FakeStore::default()
            },
            fallback: dir.path().join("tokens.json"),
        };
        tokens.set(url, "file-token").unwrap();
        tokens.store.full = false;
        tokens.set(url, "store-token").unwrap();
        assert_eq!(tokens.get(url).unwrap().as_deref(), Some("store-token"));
        assert!(!tokens.fallback.exists());
    }
}
