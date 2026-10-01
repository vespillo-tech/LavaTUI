//! Loading and saving `config.toml`.
//!
//! Location: `--config <path>` if given, else `$XDG_CONFIG_HOME/lavatui/
//! config.toml`, else the platform config dir from `directories`
//! (`~/.config/lavatui` on Linux).
//!
//! Loading never fails the app. A missing file is the defaults. A bad
//! value costs only its own key ([`Settings::parse`]); a file that isn't
//! TOML at all is the defaults. Either way the toast says what was ignored.
//!
//! The user's file is never lost:
//! * Saving edits the file in place with `toml_edit`: comments, key order,
//!   formatting and unknown keys survive; only values that changed are
//!   rewritten (keeping their trailing comment).
//! * Before a save drops anything (a syntax error, an ignored value,
//!   bytes that aren't UTF-8), the file is copied to `config.toml.bak`.
//! * A file that can't be read at all (permissions, a directory) is never
//!   written: saving is off for the session.
//! * Saves go through symlinks to the real file (dotfile managers) and are
//!   atomic: a temp file unique to this process in the target's directory,
//!   fsynced, then renamed over the target. Two lamps saving at once can't
//!   tear the file; the last writer wins.

use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item};

use super::Settings;

#[derive(Debug)]
pub struct Store {
    path: Option<PathBuf>,
    /// The file holds something the next save would drop: copy it to
    /// `.bak` first.
    backup_first: bool,
    /// False when the file exists but couldn't be read: never overwrite
    /// what we couldn't back up.
    writable: bool,
}

/// The result of [`Store::load`].
#[derive(Debug)]
pub struct Loaded {
    pub settings: Settings,
    /// What was ignored or went wrong, for a toast.
    pub problem: Option<String>,
}

impl Store {
    /// The store at `path`, or at the default location.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path: path.or_else(default_path),
            backup_first: false,
            writable: true,
        }
    }

    pub fn load(&mut self) -> Loaded {
        let defaults = |problem: Option<&str>| Loaded {
            settings: Settings::default(),
            problem: problem.map(str::to_owned),
        };
        let Some(path) = &self.path else {
            return defaults(None);
        };
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return defaults(None),
            Err(_) => {
                self.writable = false;
                return defaults(Some("config unreadable · not saving"));
            }
        };
        let (text, utf8) = decode(&bytes);
        match Settings::parse(&text) {
            Ok(parsed) => {
                self.backup_first = !utf8 || !parsed.ignored.is_empty();
                let problem = match (&parsed.ignored[..], utf8) {
                    ([], true) => None,
                    ([], false) => Some("config: not UTF-8 · backed up on save".to_owned()),
                    ([key], _) => Some(format!("config: ignored {key}")),
                    ([key, rest @ ..], _) => {
                        Some(format!("config: ignored {key} +{} more", rest.len()))
                    }
                };
                Loaded {
                    settings: parsed.settings,
                    problem,
                }
            }
            Err(_) => {
                self.backup_first = true;
                defaults(Some("config unreadable · using defaults"))
            }
        }
    }

    pub fn save(&mut self, settings: &Settings) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if !self.writable {
            return Ok(());
        }
        let target = resolve(path);
        if let Some(dir) = target.parent() {
            fs::create_dir_all(dir)?;
        }
        let fresh = toml::to_string(settings).map_err(io::Error::other)?;
        // Re-read every time: edits made while the lamp runs are kept.
        let text = match fs::read(&target) {
            Ok(bytes) => {
                let (text, utf8) = decode(&bytes);
                let doc = text.parse::<DocumentMut>().ok();
                if self.backup_first || !utf8 || doc.is_none() {
                    write_atomic(&with_suffix(path, ".bak"), &bytes)?;
                }
                match doc {
                    Some(mut doc) => {
                        merge(&mut doc, &fresh);
                        doc.to_string()
                    }
                    None => fresh,
                }
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => fresh,
            Err(err) => return Err(err),
        };
        self.backup_first = false;
        write_atomic(&target, text.as_bytes())
    }
}

/// The file as text; bytes that aren't UTF-8 (a Latin-1 comment) become
/// U+FFFD so the values around them still load. The bool is "was UTF-8".
fn decode(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), true),
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), false),
    }
}

/// Write `fresh` (the serialized settings) into `doc`, touching only the
/// values that differ. A changed value keeps its comments; a section that
/// isn't a table is replaced; keys we don't know are left alone.
fn merge(doc: &mut DocumentMut, fresh: &str) {
    let Ok(fresh) = fresh.parse::<DocumentMut>() else {
        return;
    };
    for (name, item) in fresh.iter() {
        let Some(fresh_table) = item.as_table() else {
            continue;
        };
        let Some(table) = doc.get_mut(name).and_then(Item::as_table_like_mut) else {
            let mut table = fresh_table.clone();
            table.set_position(None);
            if !doc.as_table().is_empty() {
                table.decor_mut().set_prefix("\n");
            }
            doc.insert(name, Item::Table(table));
            continue;
        };
        for (key, new) in fresh_table.iter() {
            let Some(new) = new.as_value() else {
                continue;
            };
            match table.get_mut(key) {
                Some(Item::Value(old)) if plain(old) == plain(new) => {}
                Some(Item::Value(old)) => {
                    let decor = old.decor().clone();
                    *old = new.clone();
                    *old.decor_mut() = decor;
                }
                Some(other) => *other = Item::Value(new.clone()),
                None => {
                    table.insert(key, Item::Value(new.clone()));
                }
            }
        }
    }
}

/// A value without its formatting, for comparing `'a'` with `"a"`.
fn plain(value: &toml_edit::Value) -> Option<toml::Value> {
    let mut value = value.clone();
    value.decor_mut().clear();
    let mut table: toml::Table = toml::from_str(&format!("v = {value}")).ok()?;
    table.remove("v")
}

/// Follow `path` through any symlinks to the file that really holds the
/// config (which may not exist yet). `canonicalize` can't be used: it
/// fails on a dangling link.
fn resolve(path: &Path) -> PathBuf {
    let mut path = path.to_path_buf();
    for _ in 0..40 {
        let Ok(link) = fs::read_link(&path) else {
            break;
        };
        path = match path.parent() {
            Some(dir) => dir.join(link),
            None => link,
        };
    }
    path
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

/// Replace `path` (through symlinks) with `bytes` atomically: a temp file
/// unique to this process and call in the same directory, fsynced, renamed
/// over the target. The target's permissions carry over.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = resolve(path);
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let (tmp, mut file) = create_temp(dir, &name)?;
    let written = (|| {
        if let Ok(meta) = fs::metadata(&target) {
            file.set_permissions(meta.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &target)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written?;
    // Make the rename itself durable (best effort; not on every platform).
    if let Ok(dir) = fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// `.config.toml.<pid>.<random>.tmp`, created exclusively.
fn create_temp(dir: &Path, name: &str) -> io::Result<(PathBuf, fs::File)> {
    let pid = std::process::id();
    let mut last = None;
    for _ in 0..16 {
        let nonce = RandomState::new().build_hasher().finish();
        let tmp = dir.join(format!(".{name}.{pid}.{nonce:016x}.tmp"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => return Ok((tmp, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => last = Some(err),
            Err(err) => return Err(err),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no temp name")))
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

    /// A fresh, empty temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lavatui-{name}-{}-{:x}",
                std::process::id(),
                RandomState::new().build_hasher().finish()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// File names in the dir, sorted.
        fn names(&self) -> Vec<String> {
            let mut names: Vec<_> = fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn ascii() -> Settings {
        let mut settings = Settings::default();
        settings.lamp.style = "ascii".into();
        settings
    }

    #[test]
    fn missing_file_is_defaults_without_a_problem() {
        let dir = TempDir::new("missing");
        let loaded = Store::new(Some(dir.join("config.toml"))).load();
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.problem.is_none());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new("roundtrip");
        let path = dir.join("nested/config.toml");
        let mut store = Store::new(Some(path.clone()));
        let mut settings = ascii();
        settings.pomodoro.cycles = 3;
        store.save(&settings).unwrap();
        let loaded = Store::new(Some(path)).load();
        assert_eq!(loaded.settings, settings);
        assert!(loaded.problem.is_none());
    }

    #[test]
    fn corrupt_file_falls_back_and_is_backed_up_on_save() {
        let dir = TempDir::new("corrupt");
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
    }

    #[test]
    fn one_bad_value_keeps_the_rest_and_names_the_key() {
        for (section, bad) in [
            ("lamp", "frame = \"round\""),
            ("lamp", "heat = 300"),
            ("display", "fps = 60.0"),
            ("pomodoro", "focus_min = -5"),
            ("display", "color = \"24bit\""),
        ] {
            let dir = TempDir::new("badvalue");
            let path = dir.join("config.toml");
            let header = if section == "lamp" {
                String::new()
            } else {
                format!("\n[{section}]\n")
            };
            let text = format!("[lamp]\nstyle = \"ascii\"\n{header}{bad}\n");
            fs::write(&path, &text).unwrap();
            let mut store = Store::new(Some(path.clone()));
            let loaded = store.load();
            assert_eq!(loaded.settings.lamp.style, "ascii", "{bad}");
            let key = bad.split(' ').next().unwrap();
            assert_eq!(
                loaded.problem.as_deref(),
                Some(format!("config: ignored {section}.{key}").as_str())
            );

            // The bad value is dropped on save, so it's backed up first.
            store.save(&loaded.settings).unwrap();
            assert_eq!(
                fs::read_to_string(dir.join("config.toml.bak")).unwrap(),
                text
            );
            let reloaded = Store::new(Some(path)).load();
            assert_eq!(reloaded.settings.lamp.style, "ascii");
            assert!(reloaded.problem.is_none(), "{:?}", reloaded.problem);
        }
    }

    #[test]
    fn several_ignored_keys_are_summarised() {
        let dir = TempDir::new("several");
        let path = dir.join("config.toml");
        fs::write(&path, "[lamp]\nframe = 1\nheat = \"x\"\nspeed = []\n").unwrap();
        let loaded = Store::new(Some(path)).load();
        assert_eq!(
            loaded.problem.as_deref(),
            Some("config: ignored lamp.frame +2 more")
        );
    }

    #[test]
    fn save_keeps_comments_layout_and_unknown_keys() {
        let dir = TempDir::new("comments");
        let path = dir.join("config.toml");
        let text = "# my lamp\n[lamp]\nstyle = 'solid'  # the default\nheat = 4 # warm\nfuture_key = true\n\n[clock]\nface = \"words\"\n";
        fs::write(&path, text).unwrap();
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(
            loaded.problem.as_deref(),
            Some("config: ignored lamp.future_key")
        );
        let mut settings = loaded.settings;
        settings.lamp.heat = 2;
        store.save(&settings).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with("# my lamp\n[lamp]\n"), "{saved}");
        assert!(saved.contains("\n\n[display]\n"), "{saved}");
        assert!(
            saved.contains("style = 'solid'  # the default\n"),
            "{saved}"
        );
        assert!(saved.contains("heat = 2 # warm\n"), "{saved}");
        assert!(saved.contains("future_key = true\n"), "{saved}");
        assert!(saved.contains("face = \"words\"\n"), "{saved}");
        assert_eq!(Store::new(Some(path)).load().settings, settings);
    }

    #[test]
    fn a_non_table_section_is_replaced() {
        let dir = TempDir::new("nontable");
        let path = dir.join("config.toml");
        fs::write(&path, "lamp = 5\n[clock]\nshow = false\n").unwrap();
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(loaded.problem.as_deref(), Some("config: ignored lamp"));
        store.save(&ascii()).unwrap();
        assert_eq!(Store::new(Some(path)).load().settings, ascii());
        assert!(dir.join("config.toml.bak").exists());
    }

    #[test]
    fn non_utf8_keeps_values_and_backs_up_the_bytes() {
        let dir = TempDir::new("latin1");
        let path = dir.join("config.toml");
        let bytes = b"# caf\xe9\n[lamp]\nstyle = \"ascii\"\n".to_vec();
        fs::write(&path, &bytes).unwrap();
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(loaded.settings.lamp.style, "ascii");
        assert!(loaded.problem.is_some());
        store.save(&loaded.settings).unwrap();
        assert_eq!(fs::read(dir.join("config.toml.bak")).unwrap(), bytes);
        assert_eq!(Store::new(Some(path)).load().settings, loaded.settings);
    }

    #[test]
    fn unreadable_path_is_never_written() {
        let dir = TempDir::new("isdir");
        let path = dir.join("config.toml");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), "x").unwrap();
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(
            loaded.problem.as_deref(),
            Some("config unreadable · not saving")
        );
        store.save(&ascii()).unwrap();
        assert!(path.join("keep").exists());
        assert_eq!(dir.names(), ["config.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_never_written() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("eacces");
        let path = dir.join("config.toml");
        fs::write(&path, "[lamp]\nstyle = \"ascii\"\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o200)).unwrap();
        if fs::read(&path).is_ok() {
            return; // Running as root: permissions don't bite.
        }
        let mut store = Store::new(Some(path.clone()));
        assert!(store.load().problem.is_some());
        store.save(&Settings::default()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[lamp]\nstyle = \"ascii\"\n"
        );
    }

    #[test]
    fn corrupt_file_deleted_before_save_is_fine() {
        let dir = TempDir::new("deleted");
        let path = dir.join("config.toml");
        fs::write(&path, "[lamp\n").unwrap();
        let mut store = Store::new(Some(path.clone()));
        store.load();
        fs::remove_file(&path).unwrap();
        store.save(&ascii()).unwrap();
        store.save(&Settings::default()).unwrap();
        assert_eq!(dir.names(), ["config.toml"]);
        assert_eq!(Store::new(Some(path)).load().settings, Settings::default());
    }

    #[cfg(unix)]
    #[test]
    fn save_writes_through_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new("symlink");
        fs::create_dir_all(dir.join("dotfiles")).unwrap();
        fs::create_dir_all(dir.join("config")).unwrap();
        let real = dir.join("dotfiles/lavatui.toml");
        fs::write(&real, "# tracked in git\n[lamp]\nstyle = \"solid\"\n").unwrap();
        let link = dir.join("config/config.toml");
        // A relative link, as stow makes them.
        symlink("../dotfiles/lavatui.toml", &link).unwrap();

        let mut store = Store::new(Some(link.clone()));
        let settings = store.load().settings;
        assert_eq!(settings, Settings::default());
        store.save(&ascii()).unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let saved = fs::read_to_string(&real).unwrap();
        assert!(saved.starts_with("# tracked in git\n"), "{saved}");
        assert!(saved.contains("style = \"ascii\""), "{saved}");
        assert_eq!(Store::new(Some(link)).load().settings, ascii());
        // No temp files left beside either end.
        assert_eq!(fs::read_dir(dir.join("dotfiles")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(dir.join("config")).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("perms");
        let path = dir.join("config.toml");
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        Store::new(Some(path.clone())).save(&ascii()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn temp_names_are_unique_and_cleaned_up() {
        let dir = TempDir::new("tmpnames");
        let (a, _fa) = create_temp(&dir.0, "config.toml").unwrap();
        let (b, _fb) = create_temp(&dir.0, "config.toml").unwrap();
        assert_ne!(a, b);
        let pid = std::process::id().to_string();
        assert!(a.to_string_lossy().contains(&pid));
        drop((_fa, _fb));
        fs::remove_file(a).unwrap();
        fs::remove_file(b).unwrap();
        Store::new(Some(dir.join("config.toml")))
            .save(&ascii())
            .unwrap();
        assert_eq!(dir.names(), ["config.toml"]);
    }

    #[test]
    fn concurrent_saves_never_tear_the_file() {
        let dir = TempDir::new("concurrent");
        let path = dir.join("config.toml");
        let writers: Vec<_> = (0..8u32)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let mut store = Store::new(Some(path));
                    let mut settings = Settings::default();
                    settings.pomodoro.cycles = i + 1;
                    for _ in 0..25 {
                        store.save(&settings).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        let loaded = Store::new(Some(path)).load();
        assert!(loaded.problem.is_none(), "{:?}", loaded.problem);
        assert!((1..=8).contains(&loaded.settings.pomodoro.cycles));
        assert_eq!(dir.names(), ["config.toml"]);
    }
}
