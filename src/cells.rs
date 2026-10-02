//! How the terminal draws cells, for `display.cells = "auto"` ([`Cells`]).
//!
//! macOS Terminal draws block glyphs (`█ ▀ ▄`) short of the cell's top, so
//! wax drawn as glyphs shows a dark line along every row; it's told by
//! `TERM_PROGRAM=Apple_Terminal` ([`short_blocks`]). Ghostex's renderer
//! draws them a hair short of the cell's side now and then, dark ticks in
//! moving wax ([`hosted`]): both get [`Cells::Background`].
//!
//! Ghostty may show cell backgrounds see-through while glyphs
//! stay opaque. It does with
//! `background-opacity` < 1 and `background-opacity-cells = true`: a
//! half-block cell split across two wax colours then shows its background
//! half darker, a seam (see `render::cell::half_block`). Nothing in the
//! terminal's replies says so, so this reads Ghostty's own config files,
//! once at start. Other terminals that blend backgrounds this way can be
//! told with `display.cells = "translucent"`.
//!
//! Only native Ghostty: hosts that embed Ghostty's terminal (Ghostex, with
//! its `zmx` sessions) report `TERM_PROGRAM=ghostty` too, but their
//! renderer never reads Ghostty's config ([`hosted`]).

use std::path::{Path, PathBuf};

/// Includes followed at most (Ghostty's `config-file`), against cycles.
const MAX_FILES: usize = 16;

/// How cells are drawn so they look right in the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cells {
    /// Glyphs and backgrounds both opaque, blocks filling the cell.
    Opaque,
    /// See-through backgrounds, opaque glyphs: wax only ever as glyphs
    /// (`render::cell::half_block`).
    Translucent,
    /// Block glyphs short of the cell: wax as background wherever it can
    /// be (`render::fill_from_background`).
    Background,
}

/// From the environment and Ghostty's config files; never in tests. With
/// [`Cells::Translucent`], the window's opacity too.
pub fn detect() -> (Cells, Option<f32>) {
    if cfg!(test) {
        return (Cells::Opaque, None);
    }
    let var = |k: &str| std::env::var(k).ok();
    let names = std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned());
    if short_blocks(var) || hosted(names) {
        return (Cells::Background, None);
    }
    let home = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf());
    match opacity(var, home, |p| std::fs::read_to_string(p).ok()) {
        Some(o) => (Cells::Translucent, Some(o)),
        None => (Cells::Opaque, None),
    }
}

/// Whether the terminal (`var` reads one variable) draws block glyphs
/// short of the cell's top: macOS Terminal, measured at its default font
/// and spacing (the glyph fills the bottom 25 of 30 pixels).
pub fn short_blocks(var: impl Fn(&str) -> Option<String>) -> bool {
    var("TERM_PROGRAM").as_deref() == Some("Apple_Terminal")
}

/// Whether the environment (its variable `names`) says we run inside a
/// host that embeds Ghostty's terminal rather than in Ghostty itself:
/// Ghostex (`GHOSTEX_*`) or its `zmx` sessions (`ZMX_SESSION`).
pub fn hosted(mut names: impl Iterator<Item = String>) -> bool {
    names.any(|k| k == "ZMX_SESSION" || k.starts_with("GHOSTEX_"))
}

/// Whether glyphs beyond the ones every terminal font draws should be
/// left out (the music controls' `◂◂ ‖ ▸▸ ♡ ≡`): in hosts that embed
/// Ghostty's terminal ([`hosted`]), whose renderer drew none of them
/// (lava-1xk.21). `LAVATUI_GLYPHS=safe` or `rich` says so either way.
/// Never in tests.
pub fn safe_glyphs() -> bool {
    if cfg!(test) {
        return false;
    }
    let names = std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned());
    glyphs_safe(std::env::var("LAVATUI_GLYPHS").ok().as_deref(), names)
}

/// [`safe_glyphs`] from `LAVATUI_GLYPHS` and the environment's variable
/// `names`.
pub fn glyphs_safe(choice: Option<&str>, names: impl Iterator<Item = String>) -> bool {
    match choice.map(str::trim) {
        Some(c) if c.eq_ignore_ascii_case("safe") => true,
        Some(c) if c.eq_ignore_ascii_case("rich") => false,
        _ => hosted(names),
    }
}

/// Whether we're in Ghostty (`var` reads one variable) and its config
/// (`read` reads one file) has translucent cell backgrounds. `home` is
/// the user's home directory.
#[cfg(test)]
fn translucent(
    var: impl Fn(&str) -> Option<String>,
    home: Option<PathBuf>,
    read: impl Fn(&Path) -> Option<String>,
) -> bool {
    opacity(var, home, read).is_some()
}

/// As [`translucent`], with the window's `background-opacity` (0..1).
pub fn opacity(
    var: impl Fn(&str) -> Option<String>,
    home: Option<PathBuf>,
    read: impl Fn(&Path) -> Option<String>,
) -> Option<f32> {
    let ghostty = var("TERM_PROGRAM").is_some_and(|p| p.eq_ignore_ascii_case("ghostty"))
        || var("TERM").as_deref() == Some("xterm-ghostty")
        || var("GHOSTTY_RESOURCES_DIR").is_some();
    if !ghostty {
        return None;
    }
    let mut config = Ghostty::default();
    // A stack: the next file to load last.
    let mut queue = config_files(&var, home.as_deref());
    queue.reverse();
    let mut loaded = 0;
    while let Some(path) = queue.pop() {
        loaded += 1;
        if loaded > MAX_FILES {
            break;
        }
        // Missing files (an include not there) are skipped, as Ghostty does.
        let Some(text) = read(&path) else {
            continue;
        };
        // A file's includes load after it, in order.
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for file in config.apply(&text).into_iter().rev() {
            queue.push(dir.join(file));
        }
    }
    (config.opacity < 1.0 && config.cells).then_some(config.opacity.max(0.0))
}

/// Ghostty's default config files, in the order it loads them (later
/// ones win): the XDG one, then on macOS the Application Support one;
/// each as `config` and the newer `config.ghostty`.
fn config_files(var: &impl Fn(&str) -> Option<String>, home: Option<&Path>) -> Vec<PathBuf> {
    let xdg = var("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".config")));
    let mut dirs: Vec<PathBuf> = xdg.into_iter().map(|d| d.join("ghostty")).collect();
    if cfg!(target_os = "macos")
        && let Some(home) = home
    {
        dirs.push(home.join("Library/Application Support/com.mitchellh.ghostty"));
    }
    dirs.iter()
        .flat_map(|d| [d.join("config"), d.join("config.ghostty")])
        .collect()
}

/// The two settings that matter, as Ghostty's defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Ghostty {
    opacity: f32,
    cells: bool,
}

impl Default for Ghostty {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            cells: false,
        }
    }
}

impl Ghostty {
    /// Apply one file's `key = value` lines; returns its `config-file`
    /// includes (a leading `?`, optional, dropped).
    fn apply(&mut self, text: &str) -> Vec<String> {
        let mut includes = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            match key.trim() {
                // An empty value resets to the default.
                "background-opacity" => {
                    self.opacity = match value {
                        "" => 1.0,
                        v => v.parse().unwrap_or(self.opacity),
                    };
                }
                "background-opacity-cells" => {
                    self.cells = match value {
                        "" | "false" => false,
                        "true" => true,
                        _ => self.cells,
                    };
                }
                "config-file" if !value.is_empty() => {
                    includes.push(value.strip_prefix('?').unwrap_or(value).to_string());
                }
                _ => {}
            }
        }
        includes
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn files(pairs: &[(&str, &str)]) -> impl Fn(&Path) -> Option<String> + use<> {
        let map: HashMap<PathBuf, String> = pairs
            .iter()
            .map(|(k, v)| (PathBuf::from(k), v.to_string()))
            .collect();
        move |p| map.get(p).cloned()
    }

    const GHOSTTY: &[(&str, &str)] = &[("TERM_PROGRAM", "ghostty")];
    const XDG: &str = "/h/.config/ghostty/config";

    fn home() -> Option<PathBuf> {
        Some(PathBuf::from("/h"))
    }

    #[test]
    fn macos_terminal_has_short_blocks() {
        assert!(short_blocks(env(&[("TERM_PROGRAM", "Apple_Terminal")])));
        assert!(!short_blocks(env(GHOSTTY)));
        assert!(!short_blocks(env(&[("TERM_PROGRAM", "iTerm.app")])));
        assert!(!short_blocks(env(&[])));
    }

    #[test]
    fn embedded_ghostty_hosts_are_not_ghostty() {
        let names = |n: &[&str]| {
            n.iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into_iter()
        };
        assert!(hosted(names(&["TERM_PROGRAM", "ZMX_SESSION"])));
        assert!(hosted(names(&["HOME", "GHOSTEX_SESSION_ID"])));
        assert!(!hosted(names(&[
            "TERM_PROGRAM",
            "TERM",
            "GHOSTTY_RESOURCES_DIR"
        ])));
        assert!(!hosted(names(&[])));
    }

    #[test]
    fn safe_glyphs_in_hosts_unless_told() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let ghostex = names(&["TERM_PROGRAM", "ZMX_SESSION"]);
        let ghostty = names(&["TERM_PROGRAM", "GHOSTTY_RESOURCES_DIR"]);
        assert!(glyphs_safe(None, ghostex.clone().into_iter()));
        assert!(!glyphs_safe(None, ghostty.clone().into_iter()));
        assert!(!glyphs_safe(Some("rich"), ghostex.into_iter()));
        assert!(glyphs_safe(Some(" Safe "), ghostty.clone().into_iter()));
        assert!(!glyphs_safe(Some("what"), ghostty.into_iter()));
    }

    #[test]
    fn needs_ghostty_with_both_settings() {
        let both = "background-opacity = 0.75\nbackground-opacity-cells = true\n";
        assert!(translucent(env(GHOSTTY), home(), files(&[(XDG, both)])));
        assert!(translucent(
            env(&[("TERM", "xterm-ghostty")]),
            home(),
            files(&[(XDG, both)])
        ));
        // Another terminal, even with the file there.
        assert!(!translucent(
            env(&[("TERM_PROGRAM", "iTerm.app")]),
            home(),
            files(&[(XDG, both)])
        ));
        // Opaque, or only the window translucent.
        for text in [
            "background-opacity = 1\nbackground-opacity-cells = true",
            "background-opacity = 0.75",
            "background-opacity = 0.75\nbackground-opacity-cells = false",
            "# background-opacity = 0.75\nbackground-opacity-cells = true",
            "",
        ] {
            assert!(
                !translucent(env(GHOSTTY), home(), files(&[(XDG, text)])),
                "{text:?}"
            );
        }
        assert!(!translucent(env(GHOSTTY), home(), files(&[])));
    }

    #[test]
    fn later_lines_and_files_win_and_includes_are_followed() {
        // The last value wins; an empty one resets.
        let reset =
            "background-opacity = 0.8\nbackground-opacity-cells = true\nbackground-opacity =";
        assert!(!translucent(env(GHOSTTY), home(), files(&[(XDG, reset)])));
        // XDG_CONFIG_HOME moves the file; config.ghostty is read after config.
        let custom = files(&[
            ("/x/ghostty/config", "background-opacity = 0.5"),
            (
                "/x/ghostty/config.ghostty",
                "background-opacity-cells = \"true\"",
            ),
        ]);
        let mut vars = GHOSTTY.to_vec();
        vars.push(("XDG_CONFIG_HOME", "/x"));
        assert!(translucent(env(&vars), home(), custom));
        // Includes, relative to the including file, optional or not.
        let included = files(&[
            (XDG, "config-file = ?extra\nconfig-file = more"),
            ("/h/.config/ghostty/extra", "background-opacity = 0.9"),
            ("/h/.config/ghostty/more", "background-opacity-cells = true"),
        ]);
        assert!(translucent(env(GHOSTTY), home(), included));
        // A cycle ends.
        let cycle = files(&[(XDG, "config-file = config\nbackground-opacity = 0.5")]);
        assert!(!translucent(env(GHOSTTY), home(), cycle));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_application_support_on_macos() {
        let app = "/h/Library/Application Support/com.mitchellh.ghostty/config";
        let both = "background-opacity = 0.75\nbackground-opacity-cells = true";
        assert!(translucent(env(GHOSTTY), home(), files(&[(app, both)])));
        // It loads after the XDG file, so it wins.
        let off = files(&[(XDG, both), (app, "background-opacity-cells = false")]);
        assert!(!translucent(env(GHOSTTY), home(), off));
    }
}
