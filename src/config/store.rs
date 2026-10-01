//! Loading and saving `config.toml`.
//!
//! Location: `--config <path>` if given, else `$XDG_CONFIG_HOME/lavatui/
//! config.toml` (only an absolute `$XDG_CONFIG_HOME`; the spec says a
//! relative one is ignored), else the platform config dir from `directories`
//! (`~/.config/lavatui` on Linux).
//!
//! Loading never fails the app. A missing file is the defaults. A bad
//! value costs only its own key and a value out of range is clamped
//! ([`Settings::parse`]); a file that isn't TOML at all is the defaults.
//! The toast names the first problem (`config: lamp.heat 99 → 5 +2 more ·
//! listed on exit`); when there are several, all of them go to stderr
//! when the lamp quits ([`Store::report`]).
//!
//! What a save does depends on what the path holds:
//!
//! | The path holds | Load | Save |
//! |---|---|---|
//! | nothing | defaults | writes every setting |
//! | a TOML file | its values | re-reads the file and writes only the settings changed in the app since the load (or last save): hand edits to other keys made while the lamp runs are kept |
//! | an ignored or clamped value | default / clamped value, toast | the file keeps it until that setting is changed in the app; that save copies the file to `.bak` first |
//! | bytes that aren't UTF-8 | values kept, toast | `.bak` first (the bytes become U+FFFD) |
//! | not TOML when loaded | defaults, toast with the line | `.bak` first, then replaced |
//! | not TOML any more (an edit in progress) | — | not written, toast; the next change retries |
//! | a read-only file (or folder, for a new file) | values, toast | never written |
//! | not a regular file (`/dev/null`, a fifo, a folder) | defaults, toast, never read | never written |
//! | something unreadable | defaults, toast | never written |
//!
//! "Never written" is for the whole session, and is toasted once (at load,
//! or at the first save if it only happened while the lamp ran). A save
//! that writes keeps comments, key order, formatting and unknown keys
//! (`toml_edit`; a changed value keeps its trailing comment, in its
//! column). Saves go through symlinks to the real file (dotfile managers)
//! and are atomic: a temp file unique to this process in the target's
//! directory, fsynced, then renamed over the target. Two lamps saving at once can't tear the
//! file; for a key both changed, the last writer wins.

use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item};

use super::Settings;

const UNREADABLE: &str = "unreadable · not saving";
const NOT_A_FILE: &str = "not a regular file · not saving";
const READ_ONLY: &str = "read-only · not saving";
const FOLDER_READ_ONLY: &str = "folder read-only · not saving";
const SAVE_FAILED: &str = "couldn't save settings";

/// A changed setting: `section.key`, the value before, the value now.
type Change = (String, Option<toml::Value>, toml::Value);

#[derive(Debug)]
pub struct Store {
    path: Option<PathBuf>,
    /// What the file is known to hold (as loaded, or last saved). A save
    /// writes only the settings that differ from it; `None` (never
    /// loaded) writes them all.
    known: Option<Settings>,
    /// The file wasn't TOML when loaded (and the user was told so): a
    /// save may replace it, after the backup.
    replace_invalid: bool,
    /// False once saving is off for the session.
    writable: bool,
    /// Every problem the load found, one per item, for [`Store::report`].
    notes: Vec<String>,
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
            known: None,
            replace_invalid: false,
            writable: true,
            notes: Vec::new(),
        }
    }

    pub fn load(&mut self) -> Loaded {
        self.notes.clear();
        self.replace_invalid = false;
        let settings = self.read();
        self.known = Some(settings.clone());
        let problem = match &self.notes[..] {
            [] => None,
            [one] => Some(format!("config: {one}")),
            [first, rest @ ..] => Some(format!(
                "config: {first} +{} more · listed on exit",
                rest.len()
            )),
        };
        Loaded { settings, problem }
    }

    /// What to print on stderr after the lamp quits: every problem the
    /// load found, when the toast could only name the first.
    pub fn report(&self) -> Vec<String> {
        if self.notes.len() < 2 {
            return Vec::new();
        }
        let path = self.path.as_deref().unwrap_or(Path::new("config"));
        let path = path.display();
        self.notes.iter().map(|n| format!("{path}: {n}")).collect()
    }

    /// Read and parse the file, noting each problem.
    fn read(&mut self) -> Settings {
        let Some(path) = &self.path else {
            return Settings::default();
        };
        let target = resolve(path);
        // Look before reading: reading a fifo would block forever.
        match fs::metadata(&target) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => return self.closed_on_load(NOT_A_FILE),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Settings::default(),
            Err(_) => return self.closed_on_load(UNREADABLE),
        }
        let Ok(bytes) = fs::read(&target) else {
            return self.closed_on_load(UNREADABLE);
        };
        if read_only(&target) {
            self.writable = false;
            self.notes.push(READ_ONLY.into());
        }
        let (text, utf8) = decode(&bytes);
        if !utf8 {
            self.notes.push(if self.writable {
                "not UTF-8 · backed up on save".into()
            } else {
                "not UTF-8".into()
            });
        }
        match Settings::parse(&text) {
            Ok(parsed) => {
                let notes = &mut self.notes;
                notes.extend(parsed.ignored.iter().map(|key| format!("ignored {key}")));
                notes.extend(
                    parsed
                        .clamped
                        .iter()
                        .map(|(key, how)| format!("{key} {how}")),
                );
                notes.extend(
                    parsed
                        .unknown
                        .iter()
                        .map(|key| format!("unknown key {key}")),
                );
                parsed.settings
            }
            Err(err) => {
                self.replace_invalid = self.writable;
                let line = line_of(&text, err.span());
                self.notes
                    .insert(0, format!("invalid TOML{line} · using defaults"));
                Settings::default()
            }
        }
    }

    fn closed_on_load(&mut self, why: &str) -> Settings {
        self.writable = false;
        self.notes.push(why.into());
        Settings::default()
    }

    /// Turn saving off for the session; the error is the one toast.
    fn close(&mut self, why: &str) -> Result<(), String> {
        self.writable = false;
        Err(format!("config: {why}"))
    }

    /// Write the settings that changed since the load or last save into
    /// the file as it is now. `Err` is a toast: the save failed, or saving
    /// just turned off (said once; later saves are quiet no-ops).
    pub fn save(&mut self, settings: &Settings) -> Result<(), String> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        if !self.writable {
            return Ok(());
        }
        let changed = super::changed(self.known.as_ref(), settings);
        if changed.is_empty() {
            return Ok(());
        }
        let target = resolve(&path);
        let fresh = toml::to_string(settings).map_err(|_| SAVE_FAILED.to_owned())?;
        let text = match fs::metadata(&target) {
            Ok(meta) if !meta.is_file() => return self.close(NOT_A_FILE),
            Ok(_) => {
                let Ok(bytes) = fs::read(&target) else {
                    return self.close(UNREADABLE);
                };
                if read_only(&target) {
                    return self.close(READ_ONLY);
                }
                let (text, lossy) = self.merged(&bytes, &fresh, &changed)?;
                if lossy {
                    self.write(&with_suffix(&path, ".bak"), &bytes)?;
                }
                text
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                if let Some(dir) = target.parent().filter(|d| !d.as_os_str().is_empty())
                    && let Err(err) = fs::create_dir_all(dir)
                {
                    return self.failed(&err);
                }
                fresh
            }
            Err(_) => return self.close(UNREADABLE),
        };
        self.write(&target, text.as_bytes())?;
        self.known = Some(settings.clone());
        self.replace_invalid = false;
        Ok(())
    }

    /// The file's new text: the `changed` values of `fresh` (the
    /// serialized settings) merged into `bytes` (the file now), and
    /// whether that drops anything only a `.bak` would keep.
    fn merged(
        &self,
        bytes: &[u8],
        fresh: &str,
        changed: &[Change],
    ) -> Result<(String, bool), String> {
        let (text, utf8) = decode(bytes);
        match text.parse::<DocumentMut>() {
            Ok(mut doc) => {
                let lossy = !utf8 || drops(&text, changed);
                merge(&mut doc, fresh, changed);
                Ok((doc.to_string(), lossy))
            }
            Err(_) if self.replace_invalid => Ok((fresh.to_owned(), true)),
            // Broken since the load: most likely an edit in progress.
            Err(err) => Err(format!(
                "config: invalid TOML{} · not saved",
                line_of(&text, err.span())
            )),
        }
    }

    /// [`write_atomic`], turning saving off if the folder is read-only.
    fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        write_atomic(path, bytes).or_else(|err| self.failed(&err))
    }

    fn failed(&mut self, err: &io::Error) -> Result<(), String> {
        if denied(err) {
            self.close(FOLDER_READ_ONLY)
        } else {
            Err(SAVE_FAILED.into())
        }
    }
}

fn denied(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
    )
}

/// Whether we may not write `path` (its permissions, a read-only mount).
/// Opening for append changes nothing in the file.
fn read_only(path: &Path) -> bool {
    match fs::OpenOptions::new().append(true).open(path) {
        Ok(_) => false,
        Err(err) => denied(&err),
    }
}

/// ` line 3` for an error at `span` in `text`, or nothing.
fn line_of(text: &str, span: Option<Range<usize>>) -> String {
    span.and_then(|span| text.get(..span.start))
        .map(|before| format!(" line {}", before.matches('\n').count() + 1))
        .unwrap_or_default()
}

/// The file as text; bytes that aren't UTF-8 (a Latin-1 comment) become
/// U+FFFD so the values around them still load. The bool is "was UTF-8".
fn decode(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_owned(), true),
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), false),
    }
}

/// Whether writing the `changed` keys over `text` overwrites a value the
/// app couldn't use as written (ignored or clamped): only the `.bak`
/// would keep it.
fn drops(text: &str, changed: &[Change]) -> bool {
    let Ok(parsed) = Settings::parse(text) else {
        return true;
    };
    let mut lost = parsed
        .ignored
        .iter()
        .chain(parsed.clamped.iter().map(|(key, _)| key));
    lost.any(|lost| {
        changed.iter().any(|(key, ..)| {
            key == lost
                || key
                    .strip_prefix(lost.as_str())
                    .is_some_and(|k| k.starts_with('.'))
        })
    })
}

/// Write the `changed` values of `fresh` (the serialized settings) into
/// `doc`, and take out any [`RETIRED_KEYS`](super::RETIRED_KEYS).
/// Everything else in it is left alone. A changed value keeps its
/// comments; a missing section is added at the end (below a file's
/// comments if that's all it holds); a section that isn't a table is
/// replaced by one at the end.
fn merge(doc: &mut DocumentMut, fresh: &str, changed: &[Change]) {
    let Ok(fresh) = fresh.parse::<DocumentMut>() else {
        return;
    };
    for (name, key) in super::RETIRED_KEYS.iter().filter_map(|k| k.split_once('.')) {
        if let Some(table) = doc.get_mut(name).and_then(Item::as_table_like_mut) {
            table.remove(key);
        }
    }
    for (dotted, ..) in changed {
        let Some((name, key)) = dotted.split_once('.') else {
            continue;
        };
        let Some(new) = fresh
            .get(name)
            .and_then(|t| t.get(key))
            .and_then(Item::as_value)
        else {
            continue;
        };
        if doc.get(name).and_then(Item::as_table_like).is_none() {
            // Not a table (`lamp = 5`, `[[lamp]]`): out, so the new table
            // goes at the end like any added section.
            doc.remove(name);
            let mut table = toml_edit::Table::new();
            if !doc.as_table().is_empty() {
                table.decor_mut().set_prefix("\n");
            } else if let Some(comments) = doc
                .trailing()
                .as_str()
                .filter(|t| !t.trim().is_empty())
                .map(str::to_owned)
            {
                // A file of only comments: they stay on top.
                table.decor_mut().set_prefix(format!("{comments}\n"));
                doc.set_trailing("");
            }
            doc.insert(name, Item::Table(table));
        }
        let Some(table) = doc.get_mut(name).and_then(Item::as_table_like_mut) else {
            continue;
        };
        match table.get_mut(key) {
            Some(Item::Value(old)) if plain(old) == plain(new) => {}
            Some(Item::Value(old)) => replace_value(old, new),
            Some(other) => *other = Item::Value(new.clone()),
            None => {
                table.insert(key, Item::Value(new.clone()));
            }
        }
    }
}

/// Put `new` in `old`'s place, keeping `old`'s comments. A trailing
/// comment padded with spaces stays in its column, so a block of aligned
/// comments stays aligned.
fn replace_value(old: &mut toml_edit::Value, new: &toml_edit::Value) {
    let width = |v: &toml_edit::Value| {
        let mut v = v.clone();
        v.decor_mut().clear();
        v.to_string().chars().count()
    };
    let mut decor = old.decor().clone();
    if let Some(suffix) = decor.suffix().and_then(|s| s.as_str()) {
        let comment = suffix.trim_start_matches(' ');
        if comment.starts_with('#') {
            let pad = (suffix.len() - comment.len() + width(old))
                .saturating_sub(width(new))
                .max(1);
            decor.set_suffix(format!("{}{comment}", " ".repeat(pad)));
        }
    }
    *old = new.clone();
    *old.decor_mut() = decor;
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
    let dir = match xdg_dir(std::env::var_os("XDG_CONFIG_HOME")) {
        Some(dir) => dir,
        None => directories::ProjectDirs::from("", "", "lavatui")?
            .config_dir()
            .to_path_buf(),
    };
    Some(dir.join("config.toml"))
}

/// `$XDG_CONFIG_HOME/lavatui`, if the variable is set to an absolute path.
/// The XDG spec says a relative one is invalid and must be ignored.
fn xdg_dir(xdg: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let xdg = PathBuf::from(xdg?);
    xdg.is_absolute().then(|| xdg.join("lavatui"))
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
    fn xdg_config_home_only_counts_when_absolute() {
        let abs = std::env::temp_dir().join("xdg");
        assert_eq!(xdg_dir(Some(abs.clone().into())), Some(abs.join("lavatui")));
        assert_eq!(xdg_dir(Some("rel/dir".into())), None);
        assert_eq!(xdg_dir(Some("".into())), None);
        assert_eq!(xdg_dir(None), None);
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
        fs::write(&path, "# mine\n[lamp\nstyle = ").unwrap();
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(
            loaded.problem.as_deref(),
            Some("config: invalid TOML line 2 · using defaults")
        );

        // Nothing changed: nothing is written.
        store.save(&Settings::default()).unwrap();
        assert_eq!(dir.names(), ["config.toml"]);

        store.save(&ascii()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("config.toml.bak")).unwrap(),
            "# mine\n[lamp\nstyle = "
        );
        let reloaded = Store::new(Some(path)).load();
        assert_eq!(reloaded.settings, ascii());
        assert!(reloaded.problem.is_none());
    }

    #[test]
    fn duplicate_key_is_a_syntax_error_with_its_line() {
        let dir = TempDir::new("dupkey");
        let path = dir.join("config.toml");
        fs::write(&path, "[lamp]\nheat = 2\n\nheat = 4\n").unwrap();
        let loaded = Store::new(Some(path)).load();
        assert_eq!(
            loaded.problem.as_deref(),
            Some("config: invalid TOML line 4 · using defaults")
        );
    }

    #[test]
    fn one_bad_value_keeps_the_rest_and_names_the_key() {
        for (section, bad) in [
            ("minimal", "clock = \"round\""),
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
            let dotted = format!("{section}.{key}");
            assert_eq!(
                loaded.problem.as_deref(),
                Some(format!("config: ignored {dotted}").as_str())
            );

            // A save that doesn't touch the bad value leaves it be.
            let mut settings = loaded.settings.clone();
            settings.input.mouse = true;
            store.save(&settings).unwrap();
            assert!(!dir.join("config.toml.bak").exists(), "{bad}");
            assert!(fs::read_to_string(&path).unwrap().contains(bad));

            // Changing it in the app drops it, so it's backed up first.
            let before = fs::read_to_string(&path).unwrap();
            let mut settings2 = settings.clone();
            set(&mut settings2, &dotted);
            store.save(&settings2).unwrap();
            assert_eq!(
                fs::read_to_string(dir.join("config.toml.bak")).unwrap(),
                before,
                "{bad}"
            );
            let reloaded = Store::new(Some(path)).load();
            assert_eq!(reloaded.settings, settings2, "{bad}");
            assert!(reloaded.problem.is_none(), "{:?}", reloaded.problem);
        }
    }

    /// Give `key` some valid non-default value.
    fn set(s: &mut Settings, key: &str) {
        match key {
            "minimal.clock" => s.minimal.clock = super::super::MinimalClock::Off,
            "lamp.heat" => s.lamp.heat = 4,
            "display.fps" => s.display.fps = 30,
            "pomodoro.focus_min" => s.pomodoro.focus_min = 50,
            "display.color" => s.display.color = super::super::ColorChoice::Ansi16,
            _ => unreachable!("{key}"),
        }
    }

    #[test]
    fn several_problems_are_summarised_and_all_reported() {
        let dir = TempDir::new("several");
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "colour = 1\n[lamp]\nstyle = 1\nheat = 99\nspeed = nan\n[pomodoro]\nfocus_min = 0\n",
        )
        .unwrap();
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(
            loaded.problem.as_deref(),
            Some("config: ignored lamp.style +4 more · listed on exit")
        );
        let p = path.display();
        assert_eq!(
            store.report(),
            [
                format!("{p}: ignored lamp.style"),
                format!("{p}: lamp.heat 99 → 5"),
                format!("{p}: lamp.speed nan → 1.0"),
                format!("{p}: pomodoro.focus_min 0 → 1"),
                format!("{p}: unknown key colour"),
            ]
        );
        // One problem fits the toast: nothing for stderr.
        fs::write(&path, "[lamp]\nheat = 9\n").unwrap();
        let loaded = store.load();
        assert_eq!(loaded.problem.as_deref(), Some("config: lamp.heat 9 → 5"));
        assert!(store.report().is_empty());
    }

    /// lava-ebq.37: a clamped value stays in the file until the app
    /// changes that setting; then the file is backed up first.
    #[test]
    fn clamped_values_are_kept_until_changed_then_backed_up() {
        let dir = TempDir::new("clamped");
        let path = dir.join("config.toml");
        let text = "[display]\nfps = 1000\ncell_aspect = nan\n[lamp]\nheat = 99\nspeed = 1e308\n";
        fs::write(&path, text).unwrap();
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(loaded.settings.display.fps, 240);
        assert_eq!(loaded.settings.display.cell_aspect, 2.0);
        assert_eq!(loaded.settings.lamp.heat, 5);
        assert_eq!(loaded.settings.lamp.speed, 4.0);
        assert_eq!(store.notes.len(), 4, "{:?}", store.notes);

        let mut settings = loaded.settings.clone();
        settings.clock.show = false;
        store.save(&settings).unwrap();
        assert!(!dir.join("config.toml.bak").exists());
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("heat = 99\n"), "{saved}");
        assert!(saved.contains("fps = 1000\n"), "{saved}");

        settings.lamp.heat = 4;
        store.save(&settings).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("config.toml.bak")).unwrap(),
            saved
        );
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("heat = 4\n"), "{saved}");
        assert!(saved.contains("fps = 1000\n"), "{saved}");
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
            Some("config: unknown key lamp.future_key")
        );
        let mut settings = loaded.settings;
        settings.lamp.heat = 2;
        settings.pomodoro.cycles = 6;
        store.save(&settings).unwrap();
        // Nothing was dropped, so nothing was backed up.
        assert!(!dir.join("config.toml.bak").exists());

        // Only what changed is written: heat in place, a new section for
        // the cycles, and no other defaults.
        let saved = fs::read_to_string(&path).unwrap();
        assert_eq!(
            saved,
            "# my lamp\n[lamp]\nstyle = 'solid'  # the default\nheat = 2 # warm\nfuture_key = true\n\n[clock]\nface = \"words\"\n\n[pomodoro]\ncycles = 6\n"
        );
        assert_eq!(Store::new(Some(path)).load().settings, settings);
    }

    /// lava-ebq.33: the file is re-read at save time and only keys changed
    /// in the app are written, so edits made between two saves survive.
    #[test]
    fn external_edit_between_saves_survives() {
        let dir = TempDir::new("external");
        let path = dir.join("config.toml");
        fs::write(&path, "[lamp]\nheat = 2\n").unwrap();
        let mut store = Store::new(Some(path.clone()));
        let mut settings = store.load().settings;
        settings.lamp.style = "ascii".into();
        store.save(&settings).unwrap();

        // Edited by hand while the lamp runs.
        fs::write(
            &path,
            "[lamp]\nheat = 5\nstyle = \"ascii\"\n\n[pomodoro]\nfocus_min = 50 # long\n",
        )
        .unwrap();
        settings.lamp.speed = 2.0;
        store.save(&settings).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[lamp]\nheat = 5\nstyle = \"ascii\"\nspeed = 2.0\n\n[pomodoro]\nfocus_min = 50 # long\n"
        );
        // A value both changed: the app's newer change wins.
        settings.lamp.heat = 1;
        store.save(&settings).unwrap();
        let loaded = Store::new(Some(path)).load().settings;
        assert_eq!(loaded.lamp.heat, 1);
        assert_eq!(loaded.pomodoro.focus_min, 50);
    }

    /// Settings earlier versions had load without a toast; the next save
    /// takes them out (no backup: there's nothing to keep).
    #[test]
    fn retired_keys_load_quietly_and_go_on_save() {
        let dir = TempDir::new("retired");
        let path = dir.join("config.toml");
        fs::write(&path, "[lamp]\nlighting = true # glow\nheat = 2\n").unwrap();
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(loaded.problem, None);
        let mut settings = loaded.settings;
        settings.lamp.heat = 3;
        store.save(&settings).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "[lamp]\nheat = 3\n");
        assert_eq!(dir.names(), ["config.toml"]);
    }

    /// An edit in progress that breaks the file isn't clobbered.
    #[test]
    fn file_broken_while_running_is_not_written() {
        let dir = TempDir::new("broken");
        let path = dir.join("config.toml");
        fs::write(&path, "[lamp]\nheat = 2\n").unwrap();
        let mut store = Store::new(Some(path.clone()));
        let mut settings = store.load().settings;
        fs::write(&path, "[lamp]\nheat = \n").unwrap();
        settings.lamp.speed = 2.0;
        assert_eq!(
            store.save(&settings),
            Err("config: invalid TOML line 2 · not saved".into())
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "[lamp]\nheat = \n");
        assert_eq!(dir.names(), ["config.toml"]);
        // Fixed: the pending change goes in.
        fs::write(&path, "[lamp]\nheat = 4\n").unwrap();
        store.save(&settings).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[lamp]\nheat = 4\nspeed = 2.0\n"
        );
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
        // The new table goes at the end, with no leading blank line.
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[clock]\nshow = true\n\n[lamp]\nstyle = \"ascii\"\n"
        );
        assert_eq!(Store::new(Some(path.clone())).load().settings, ascii());
        assert!(dir.join("config.toml.bak").exists());

        // Alone in the file, it's replaced in place.
        fs::write(&path, "lamp = 5\n").unwrap();
        let mut store = Store::new(Some(path.clone()));
        store.load();
        store.save(&ascii()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[lamp]\nstyle = \"ascii\"\n"
        );
    }

    #[test]
    fn a_comments_only_file_keeps_its_comments_on_top() {
        let dir = TempDir::new("comments-only");
        let path = dir.join("config.toml");
        fs::write(&path, "# my lamp\n# see README\n").unwrap();
        let mut store = Store::new(Some(path.clone()));
        store.load();
        store.save(&ascii()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# my lamp\n# see README\n\n[lamp]\nstyle = \"ascii\"\n"
        );
    }

    #[test]
    fn aligned_trailing_comments_stay_aligned() {
        let dir = TempDir::new("aligned");
        let path = dir.join("config.toml");
        let text = "[lamp]\nstyle = \"ascii\"      # look\nheat = 3            # warm\n";
        fs::write(&path, text).unwrap();
        let mut store = Store::new(Some(path.clone()));
        let mut settings = store.load().settings;
        settings.lamp.style = "halftone".into();
        store.save(&settings).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[lamp]\nstyle = \"halftone\"   # look\nheat = 3            # warm\n"
        );
        // Too long for the column: one space, never touching.
        settings.lamp.style = "chrome-and-more".into();
        store.save(&settings).unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("style = \"chrome-and-more\" # look\n")
        );
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
        assert_eq!(
            loaded.problem.as_deref(),
            Some("config: not UTF-8 · backed up on save")
        );
        let mut settings = loaded.settings;
        settings.lamp.heat = 1;
        store.save(&settings).unwrap();
        assert_eq!(fs::read(dir.join("config.toml.bak")).unwrap(), bytes);
        assert_eq!(Store::new(Some(path)).load().settings, settings);
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
            Some("config: not a regular file · not saving")
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
        assert_eq!(
            store.load().problem.as_deref(),
            Some("config: unreadable · not saving")
        );
        store.save(&ascii()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[lamp]\nstyle = \"ascii\"\n"
        );
    }

    /// lava-ebq.34: a read-only file is never written, and says so once.
    #[cfg(unix)]
    #[test]
    fn read_only_file_is_never_written() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("readonly");
        let path = dir.join("config.toml");
        let text = "[lamp]\nheat = 2\n";
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        if fs::OpenOptions::new().append(true).open(&path).is_ok() {
            return; // Running as root: permissions don't bite.
        }
        let mut store = Store::new(Some(path.clone()));
        let loaded = store.load();
        assert_eq!(loaded.settings.lamp.heat, 2);
        assert_eq!(
            loaded.problem.as_deref(),
            Some("config: read-only · not saving")
        );
        let mut settings = loaded.settings;
        settings.clock.show = false;
        assert_eq!(store.save(&settings), Ok(()));
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert_eq!(dir.names(), ["config.toml"]);

        // Made read-only while the lamp runs: the first save says so, once.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut store = Store::new(Some(path.clone()));
        let mut settings = store.load().settings;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        settings.clock.show = false;
        assert_eq!(
            store.save(&settings),
            Err("config: read-only · not saving".into())
        );
        settings.lamp.heat = 4;
        assert_eq!(store.save(&settings), Ok(()));
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert_eq!(dir.names(), ["config.toml"]);
    }

    /// lava-ebq.34: `/dev/null`, a fifo: never read, never renamed over.
    #[cfg(unix)]
    #[test]
    fn non_regular_files_are_never_written() {
        let dir = TempDir::new("special");
        let fifo = dir.join("fifo.toml");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        let mut paths = vec![PathBuf::from("/dev/null")];
        if made.is_ok_and(|s| s.success()) {
            paths.push(fifo.clone());
        }
        for path in paths {
            let mut store = Store::new(Some(path.clone()));
            // Reading the fifo would block: the load must not.
            let loaded = store.load();
            assert_eq!(loaded.settings, Settings::default());
            assert_eq!(
                loaded.problem.as_deref(),
                Some("config: not a regular file · not saving"),
                "{path:?}"
            );
            assert_eq!(store.save(&ascii()), Ok(()));
            assert_eq!(store.save(&Settings::default()), Ok(()));
            assert!(!fs::metadata(&path).unwrap().is_file(), "{path:?}");
        }
        assert!(
            dir.names().iter().all(|n| n == "fifo.toml"),
            "{:?}",
            dir.names()
        );

        // A file replaced by a fifo while running: one toast, no write.
        let path = dir.join("config.toml");
        fs::write(&path, "").unwrap();
        let mut store = Store::new(Some(path.clone()));
        store.load();
        fs::remove_file(&path).unwrap();
        if std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success())
        {
            assert_eq!(
                store.save(&ascii()),
                Err("config: not a regular file · not saving".into())
            );
            assert_eq!(store.save(&Settings::default()), Ok(()));
            use std::os::unix::fs::FileTypeExt;
            assert!(fs::metadata(&path).unwrap().file_type().is_fifo());
        }
    }

    #[cfg(unix)]
    #[test]
    fn read_only_folder_turns_saving_off_once() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("rofolder");
        let path = dir.join("config.toml");
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o555)).unwrap();
        let mut store = Store::new(Some(path.clone()));
        assert!(store.load().problem.is_none());
        let first = store.save(&ascii());
        let second = store.save(&Settings::default());
        fs::set_permissions(&dir.0, fs::Permissions::from_mode(0o755)).unwrap();
        if path.exists() {
            return; // Running as root: permissions don't bite.
        }
        assert_eq!(first, Err("config: folder read-only · not saving".into()));
        assert_eq!(second, Ok(()));
        assert!(dir.names().is_empty());
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
