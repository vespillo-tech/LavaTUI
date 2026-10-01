//! Loading and saving `config.toml`.
//!
//! Location: `--config <path>` if given, else `$XDG_CONFIG_HOME/lavatui/
//! config.toml`, else the platform config dir from `directories`
//! (`~/.config/lavatui` on Linux).
//!
//! Loading never fails the app: a missing file is the defaults, an
//! unreadable or corrupt one is the defaults plus a message for a toast.
//! A corrupt file is moved aside to `config.toml.bak` before the first save
//! overwrites it, so a typo never silently costs the user their file.
//! Saves are atomic (write a temp file, then rename).

use std::fs;
use std::io;
use std::path::PathBuf;

use super::Settings;

#[derive(Debug)]
pub struct Store {
    path: Option<PathBuf>,
    /// The file on disk is corrupt: back it up before overwriting.
    backup_first: bool,
}

/// The result of [`Store::load`].
#[derive(Debug)]
pub struct Loaded {
    pub settings: Settings,
    /// Why the file was ignored, for a toast.
    pub problem: Option<String>,
}

impl Store {
    /// The store at `path`, or at the default location.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path: path.or_else(default_path),
            backup_first: false,
        }
    }

    pub fn load(&mut self) -> Loaded {
        let Some(path) = &self.path else {
            return Loaded {
                settings: Settings::default(),
                problem: None,
            };
        };
        let (settings, problem) = match fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Settings>(&text) {
                Ok(settings) => (settings.sanitized(), None),
                Err(_) => {
                    self.backup_first = true;
                    (
                        Settings::default(),
                        Some("config unreadable · using defaults"),
                    )
                }
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => (Settings::default(), None),
            Err(_) => (
                Settings::default(),
                Some("config unreadable · using defaults"),
            ),
        };
        Loaded {
            settings,
            problem: problem.map(str::to_owned),
        }
    }

    pub fn save(&mut self, settings: &Settings) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        if self.backup_first {
            let mut bak = path.clone().into_os_string();
            bak.push(".bak");
            fs::rename(path, bak)?;
            self.backup_first = false;
        }
        let text = toml::to_string(settings).map_err(io::Error::other)?;
        let mut tmp = path.clone().into_os_string();
        tmp.push(".tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)
    }
}

fn default_path() -> Option<PathBuf> {
    let dir = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(xdg) if !xdg.is_empty() => PathBuf::from(xdg).join("lavatui"),
        _ => directories::ProjectDirs::from("", "", "lavatui")?
            .config_dir()
            .to_path_buf(),
    };
    Some(dir.join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lavatui-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn missing_file_is_defaults_without_a_problem() {
        let dir = temp_dir("missing");
        let loaded = Store::new(Some(dir.join("config.toml"))).load();
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.problem.is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("nested/config.toml");
        let mut store = Store::new(Some(path.clone()));
        let mut settings = Settings::default();
        settings.lamp.style = "ascii".into();
        settings.pomodoro.cycles = 3;
        store.save(&settings).unwrap();
        assert_eq!(Store::new(Some(path)).load().settings, settings);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_falls_back_and_is_backed_up_on_save() {
        let dir = temp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, "[lamp\nstyle = ").unwrap();
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.problem.is_some());

        store.save(&Settings::default()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("config.toml.bak")).unwrap(),
            "[lamp\nstyle = "
        );
        assert!(Store::new(Some(path)).load().problem.is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn wrong_types_fall_back() {
        let dir = temp_dir("types");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, "[display]\nfps = \"fast\"\n").unwrap();
        let loaded = Store::new(Some(path)).load();
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.problem.is_some());
        let _ = fs::remove_dir_all(dir);
    }
}
