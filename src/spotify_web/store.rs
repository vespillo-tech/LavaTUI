//! Where the login lives between runs: the OS credential store (macOS
//! Keychain, Windows Credential Manager, Secret Service on Linux, via
//! `keyring`), else a file only the user can read (0600 on Unix) in the data
//! dir. Tokens are kept per Client ID; changing the ID means logging in
//! again.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// An OAuth token set. `expires_at` is Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    #[serde(default)]
    pub scope: String,
}

pub trait TokenStore: Send {
    /// The saved tokens for `client_id`, if any (a set saved for another
    /// Client ID counts as none).
    fn load(&self, client_id: &str) -> Option<Tokens>;
    fn save(&self, tokens: &Tokens) -> Result<(), String>;
    /// Forgets the saved login. Best effort.
    fn clear(&self);
}

const SERVICE: &str = "lavatui";
const ACCOUNT: &str = "spotify-tokens";
const FILE_NAME: &str = "spotify-tokens.json";

/// Keyring first, the 0600 file when no keyring is available.
pub struct SystemStore {
    account: &'static str,
    file: Option<PathBuf>,
}

impl SystemStore {
    pub fn new() -> Self {
        Self {
            account: ACCOUNT,
            file: default_file(),
        }
    }

    fn entry(&self) -> Option<keyring::Entry> {
        keyring::Entry::new(SERVICE, self.account).ok()
    }
}

impl TokenStore for SystemStore {
    fn load(&self, client_id: &str) -> Option<Tokens> {
        let from_keyring = self.entry().and_then(|e| e.get_password().ok());
        let json = from_keyring.or_else(|| fs::read_to_string(self.file.as_ref()?).ok())?;
        parse(&json, client_id)
    }

    fn save(&self, tokens: &Tokens) -> Result<(), String> {
        let json = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
        let keyring = self
            .entry()
            .ok_or_else(|| "no keyring".to_string())
            .and_then(|e| e.set_password(&json).map_err(|e| e.to_string()));
        match keyring {
            Ok(()) => {
                // An older fallback copy would otherwise linger in plain text.
                if let Some(file) = &self.file {
                    let _ = fs::remove_file(file);
                }
                Ok(())
            }
            Err(keyring_err) => {
                let file = self.file.as_ref().ok_or(keyring_err)?;
                write_private(file, &json).map_err(|e| format!("{}: {e}", file.display()))
            }
        }
    }

    fn clear(&self) {
        if let Some(e) = self.entry() {
            let _ = e.delete_credential();
        }
        if let Some(file) = &self.file {
            let _ = fs::remove_file(file);
        }
    }
}

/// A file-only store (also what [`SystemStore`] falls back to).
pub struct FileStore(pub PathBuf);

impl TokenStore for FileStore {
    fn load(&self, client_id: &str) -> Option<Tokens> {
        parse(&fs::read_to_string(&self.0).ok()?, client_id)
    }

    fn save(&self, tokens: &Tokens) -> Result<(), String> {
        let json = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
        write_private(&self.0, &json).map_err(|e| e.to_string())
    }

    fn clear(&self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn parse(json: &str, client_id: &str) -> Option<Tokens> {
    serde_json::from_str::<Tokens>(json)
        .ok()
        .filter(|t| t.client_id == client_id)
}

/// `$XDG_DATA_HOME/lavatui/…` (absolute only, as for the config), else the
/// platform data dir.
pub fn default_file() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join("lavatui"));
    let dir = match xdg {
        Some(dir) => dir,
        None => directories::ProjectDirs::from("", "", "lavatui")?
            .data_dir()
            .to_path_buf(),
    };
    Some(dir.join(FILE_NAME))
}

/// Writes `contents` atomically to `path`, readable by the owner only: a
/// 0600 temp file in the same dir, then a rename.
fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut file = opts.open(&tmp)?;
        // `mode` only applies on create; a stale temp keeps its old mode.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// In-memory store for tests.
#[cfg(test)]
#[derive(Default, Clone)]
pub struct MemoryStore(pub std::sync::Arc<std::sync::Mutex<Option<Tokens>>>);

#[cfg(test)]
impl TokenStore for MemoryStore {
    fn load(&self, client_id: &str) -> Option<Tokens> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .filter(|t| t.client_id == client_id)
    }
    fn save(&self, tokens: &Tokens) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(tokens.clone());
        Ok(())
    }
    fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(client_id: &str) -> Tokens {
        Tokens {
            client_id: client_id.into(),
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: 42,
            scope: "s".into(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lavatui-spotify-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn file_store_round_trips_privately_and_per_client() {
        let dir = temp_dir("store");
        let store = FileStore(dir.join("nested").join(FILE_NAME));
        assert_eq!(store.load("id"), None);
        store.save(&tokens("id")).unwrap();
        assert_eq!(store.load("id"), Some(tokens("id")));
        assert_eq!(store.load("other-id"), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&store.0).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Overwrite keeps the mode and leaves no temp files behind.
        store.save(&tokens("id2")).unwrap();
        assert_eq!(store.load("id2"), Some(tokens("id2")));
        let names: Vec<_> = fs::read_dir(store.0.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(FILE_NAME)]);
        store.clear();
        assert_eq!(store.load("id2"), None);
        let _ = fs::remove_dir_all(dir);
    }

    /// The real OS credential store, under a throwaway account name, with
    /// no file fallback: save, load, clear.
    /// `cargo test -- --ignored live_keyring`
    #[test]
    #[ignore = "touches the OS keyring"]
    fn live_keyring_round_trip() {
        let store = SystemStore {
            account: "spotify-tokens-test",
            file: None,
        };
        store.save(&tokens("id")).expect("keyring save");
        assert_eq!(store.load("id"), Some(tokens("id")));
        store.clear();
        assert_eq!(store.load("id"), None);
    }

    #[test]
    fn garbage_file_loads_as_none() {
        let dir = temp_dir("garbage");
        fs::create_dir_all(&dir).unwrap();
        let store = FileStore(dir.join(FILE_NAME));
        fs::write(&store.0, "{not json").unwrap();
        assert_eq!(store.load("id"), None);
        let _ = fs::remove_dir_all(dir);
    }
}
