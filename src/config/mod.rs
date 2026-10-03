//! Settings: the persisted TOML surface of docs/design.md §9, plus the
//! session-only CLI overrides.
//!
//! [`Settings`] is what lives in `config.toml` (XDG config dir, see
//! [`store`]). Every field has a default and every section is optional, so
//! a partial file is fine. [`Settings::parse`] is tolerant per field: a
//! value of the wrong type, or a style / palette / face name that doesn't
//! exist, is reported and replaced by its default while every other value
//! is kept. Values out of range (or NaN) are clamped and reported too
//! ([`Parsed::clamped`]). Keys it doesn't know are reported separately:
//! saving keeps them. A stale or hand-mangled file never stops the lamp.
//!
//! [`Session`] is what the CLI adds on top: flags win for this run only and
//! are never written back (§9).

pub mod store;

use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::clock;
use crate::dock::cover::ArtSettings;
use crate::dock::{self, DockSettings, Place};
use crate::render::StyleId;
use crate::sim::SimSpeed;
use crate::theme::Palette;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub display: Display,
    pub lamp: Lamp,
    pub theme: ThemeSettings,
    pub clock: Clock,
    pub pomodoro: Pomodoro,
    pub ui: Ui,
    pub minimal: Minimal,
    pub input: Input,
    pub dock: DockSettings,
    /// Covers: the cover widget's detail and size, the music card's own.
    pub art: ArtSettings,
    pub lyrics: Lyrics,
    pub spotify: Spotify,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    /// Target render fps, 1..=240.
    pub fps: u32,
    pub color: ColorChoice,
    /// Cell height ÷ width, used only when the terminal doesn't report
    /// its pixel size (§2.3).
    pub cell_aspect: f64,
    /// Whether the terminal shows cell backgrounds see-through.
    pub cells: CellsChoice,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lamp {
    pub style: String,
    /// 1..=5.
    pub heat: u8,
    /// 0.25 | 0.5 | 1 | 2 | 4 (snapped to the nearest).
    pub speed: f64,
    /// A thin layer of wax resting under the top, as in a real lamp.
    pub top_wax: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    pub palette: String,
    /// Leave the background to the terminal: never paint `bg`.
    pub transparent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    pub face: String,
    pub hour24: bool,
    /// Seconds on the bigger clock forms (side panel, L/XL). Off: `14:32`
    /// at every size, no second hand.
    pub seconds: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pomodoro {
    pub focus_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    pub cycles: u32,
    pub bell: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub mode: UiMode,
    pub status_bar: bool,
    /// Show the welcome card at start. Dismissing it turns this off (its
    /// own change, saved at once); `w` shows the card again.
    pub welcome: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Minimal {
    pub clock: MinimalClock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Input {
    /// Mouse capture: clicks on the music widget, the pickers and the wax.
    /// On by default since v1.3; the terminal's own text selection then
    /// needs shift (option on macOS Terminal / iTerm2) held while dragging.
    pub mouse: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self { mouse: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Lyrics {
    /// Lyrics timing, in ms: positive shows them later, negative sooner
    /// (by ear: Bluetooth headphones play late). Lines and words alike.
    pub delay_ms: i32,
    /// Word by word: `on`, `timed` (only lyrics that time each word) or
    /// `off` (the whole line at once).
    pub karaoke: crate::lyrics::Karaoke,
}

/// `lyrics.delay_ms`'s range, and the step the settings screen moves it by.
pub const LYRICS_DELAY: (i32, i32) = (-1000, 1000);
pub const LYRICS_DELAY_STEP: i32 = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Spotify {
    /// Your Spotify app's Client ID, for the Web API library features
    /// (`docs/spotify.md`). Empty: off, unless `LAVATUI_SPOTIFY_CLIENT_ID`
    /// is set. Playback control needs none of this.
    pub client_id: String,
    /// Where the login is kept between runs.
    pub store: LoginStore,
    /// A login is saved (kept up to date by the app). Not a secret: it lets
    /// the app say "connected" without reading the saved login, which on
    /// macOS can make the Keychain ask for permission (lava-1xk.38).
    pub logged_in: bool,
}

/// `spotify.store`: where the Spotify login is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LoginStore {
    /// The OS credential store (macOS Keychain, Windows Credential Manager,
    /// Secret Service on Linux), else the private file.
    #[default]
    System,
    /// A file only you can read (0600) in the app's data folder: never
    /// prompts, but any program running as you can read it.
    File,
}

/// `display.color` / `--color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ColorChoice {
    #[default]
    Auto,
    Truecolor,
    #[serde(rename = "256")]
    #[value(name = "256")]
    Ansi256,
    #[serde(rename = "16")]
    #[value(name = "16")]
    Ansi16,
    None,
}

/// `display.cells`: how the terminal draws cells (`cells::Cells`).
/// `translucent`: it blends cell backgrounds over the window while glyphs
/// stay opaque (Ghostty's `background-opacity-cells`), so half blocks never
/// split wax across a glyph and its background (no half-row seams).
/// `background`: its block glyphs don't fill the cell (macOS Terminal), so
/// wax is drawn as background wherever it can be (no lines between rows).
/// `auto` picks by terminal (and reads Ghostty's config).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CellsChoice {
    #[default]
    Auto,
    Opaque,
    Translucent,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UiMode {
    #[default]
    Full,
    Minimal,
}

/// Whether minimal mode shows its tiny clock, in the corner (§3). `under`
/// (under the glass lamp, which v1.1 dropped) loads as `corner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MinimalClock {
    #[default]
    #[serde(alias = "under")]
    Corner,
    Off,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            fps: 60,
            color: ColorChoice::Auto,
            cell_aspect: 2.0,
            cells: CellsChoice::Auto,
        }
    }
}

impl Default for Lamp {
    fn default() -> Self {
        Self {
            style: "solid".into(),
            heat: 3,
            speed: 1.0,
            top_wax: false,
        }
    }
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            palette: "lava".into(),
            transparent: false,
        }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            face: "blocks".into(),
            hour24: true,
            seconds: true,
        }
    }
}

impl Default for Pomodoro {
    fn default() -> Self {
        Self {
            focus_min: 25,
            short_break_min: 5,
            long_break_min: 15,
            cycles: 4,
            bell: true,
        }
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            mode: UiMode::Full,
            status_bar: true,
            welcome: true,
        }
    }
}

/// The result of [`Settings::parse`].
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// Sanitized settings: every valid value from the file, defaults for
    /// the rest.
    pub settings: Settings,
    /// Dotted keys whose value was dropped for the default (`lamp.frame`),
    /// in file order: a save overwrites them.
    pub ignored: Vec<String>,
    /// Keys that aren't settings (`lamp.future_key`): unused, but a save
    /// leaves them in the file.
    pub unknown: Vec<String>,
    /// Values [`Settings::sanitized`] changed, sorted by key:
    /// `("lamp.heat", "99 → 5")`. The app runs on the new value; the file
    /// keeps the old one until that setting is changed in the app.
    pub clamped: Vec<(String, String)>,
    /// The settings as the file has them, before values carried over from
    /// retired keys ([`RETIRED_KEYS`]): what a save compares against, so
    /// the next save writes the carried-over value as the old key goes.
    pub baseline: Settings,
}

/// Settings earlier versions had, as `section.key`: v1.1 dropped the glass
/// frame and the lighting pass, and `clock.show` became `dock.clock`
/// (`show = false` loads as `"off"`). A file that still has one loads
/// without a word (it isn't an unknown key), and the next save takes it
/// out.
pub const RETIRED_KEYS: [&str; 3] = ["lamp.frame", "lamp.lighting", "clock.show"];

/// Styles earlier versions had (v1.1 dropped them): a file naming one gets
/// the default style, without a word. The command line rejects them like
/// any other unknown name.
const RETIRED_STYLES: [&str; 3] = ["heatmap", "dither", "crt"];

impl Settings {
    /// The sections of `config.toml`, in file order.
    const SECTIONS: [&str; 12] = [
        "display", "lamp", "theme", "clock", "pomodoro", "ui", "minimal", "input", "dock", "art",
        "lyrics", "spotify",
    ];

    /// Parse a hand-editable `config.toml`. Only a TOML syntax error fails;
    /// a bad value costs just that one key (see [`Parsed::ignored`]).
    pub fn parse(text: &str) -> Result<Parsed, toml::de::Error> {
        let file: toml::Table = toml::from_str(text)?;
        let mut out = Parsed {
            settings: Settings::default(),
            ignored: Vec::new(),
            unknown: Vec::new(),
            clamped: Vec::new(),
            baseline: Settings::default(),
        };
        for key in file.keys() {
            if !Self::SECTIONS.contains(&key.as_str()) {
                out.unknown.push(key.clone());
            }
        }
        let (ig, un) = (&mut out.ignored, &mut out.unknown);
        let mut settings = Settings {
            display: section("display", &file, ig, un),
            lamp: section("lamp", &file, ig, un),
            theme: section("theme", &file, ig, un),
            clock: section("clock", &file, ig, un),
            pomodoro: section("pomodoro", &file, ig, un),
            ui: section("ui", &file, ig, un),
            minimal: section("minimal", &file, ig, un),
            input: section("input", &file, ig, un),
            dock: section("dock", &file, ig, un),
            art: section("art", &file, ig, un),
            lyrics: section("lyrics", &file, ig, un),
            spotify: section("spotify", &file, ig, un),
        };
        settings.check_names(&mut out.ignored);
        out.baseline = settings.clone().sanitized();
        settings.carry_over(&file);
        out.settings = settings.clone().sanitized();
        out.clamped = changed(Some(&settings), &out.settings)
            .into_iter()
            .map(|(key, old, new)| {
                let old = old.map_or_else(String::new, |v| v.to_string());
                (key, format!("{old} → {new}"))
            })
            .collect();
        Ok(out)
    }

    /// Put back the default for a style, palette or face name that names
    /// nothing, reporting it in `ignored` (a retired style quietly).
    fn check_names(&mut self, ignored: &mut Vec<String>) {
        if RETIRED_STYLES.contains(&self.lamp.style.as_str()) {
            self.lamp.style = Lamp::default().style;
        }
        let mut check = |key: &str, value: &mut String, known: bool, default: String| {
            if !known {
                ignored.push(key.to_owned());
                *value = default;
            }
        };
        let known = StyleId::by_name(&self.lamp.style).is_some();
        check(
            "lamp.style",
            &mut self.lamp.style,
            known,
            Lamp::default().style,
        );
        let known = Palette::by_name(&self.theme.palette).is_some();
        let default = ThemeSettings::default().palette;
        check("theme.palette", &mut self.theme.palette, known, default);
        let known = clock::face_by_name(&self.clock.face).is_some();
        check(
            "clock.face",
            &mut self.clock.face,
            known,
            Clock::default().face,
        );
    }

    /// Values from retired keys: `clock.show = false` hides the clock,
    /// unless the file also says where the clock goes; a single
    /// `dock.anchor` (before v1.2) becomes one per widget.
    fn carry_over(&mut self, file: &toml::Table) {
        self.dock.split_anchors();
        let old = |section: &str, key: &str| file.get(section)?.as_table()?.get(key).cloned();
        let clock = &dock::Clock as &dyn dock::DockWidget;
        let placed = old("dock", clock.name()).is_some();
        if old("clock", "show") == Some(toml::Value::Boolean(false)) && !placed {
            self.dock.set(clock, Place::Off);
        }
    }

    /// Clamp every numeric field into its valid range (hand-edited files).
    pub fn sanitized(mut self) -> Self {
        let d = &mut self.display;
        d.fps = d.fps.clamp(1, 240);
        if !(1.6..=2.6).contains(&d.cell_aspect) {
            d.cell_aspect = if d.cell_aspect.is_finite() {
                d.cell_aspect.clamp(1.6, 2.6)
            } else {
                2.0
            };
        }
        self.lamp.heat = self.lamp.heat.clamp(1, 5);
        self.lamp.speed = if self.lamp.speed.is_finite() && self.lamp.speed > 0.0 {
            SimSpeed::from_factor(self.lamp.speed).factor()
        } else {
            1.0
        };
        let p = &mut self.pomodoro;
        for min in [
            &mut p.focus_min,
            &mut p.short_break_min,
            &mut p.long_break_min,
        ] {
            *min = (*min).clamp(1, 24 * 60);
        }
        p.cycles = p.cycles.clamp(1, 12);
        self.lyrics.delay_ms = self.lyrics.delay_ms.clamp(LYRICS_DELAY.0, LYRICS_DELAY.1);
        // A Client ID is 32 hex digits; anything with other characters (a
        // pasted secret, a quote) is no ID at all.
        let id = self.spotify.client_id.trim();
        self.spotify.client_id = if id.chars().all(|c| c.is_ascii_alphanumeric()) {
            id.to_owned()
        } else {
            String::new()
        };
        self
    }

    /// The Spotify Client ID to use: `spotify.client_id`, else
    /// `LAVATUI_SPOTIFY_CLIENT_ID`; `None` keeps the Web API features off.
    pub fn spotify_client_id(&self) -> Option<String> {
        let id = self.spotify.client_id.trim();
        if id.is_empty() {
            crate::spotify_web::client_id_from_env()
        } else {
            Some(id.to_owned())
        }
    }

    pub fn minimal(&self) -> bool {
        self.ui.mode == UiMode::Minimal
    }
}

/// The values of `new` that differ from `old` (every value when `old` is
/// `None`), as `(section.key, old value, new value)`, sorted by key.
/// Generic over the sections, like [`section`]: new fields need no code.
pub fn changed(
    old: Option<&Settings>,
    new: &Settings,
) -> Vec<(String, Option<toml::Value>, toml::Value)> {
    let table = |s: &Settings| toml::Table::try_from(s).unwrap_or_default();
    let old = old.map(table);
    let mut out = Vec::new();
    for (name, section) in table(new) {
        let Some(section) = section.as_table() else {
            continue;
        };
        let old_section = old.as_ref().and_then(|o| o.get(&name)?.as_table().cloned());
        for (key, value) in section {
            let before = old_section.as_ref().and_then(|o| o.get(key)).cloned();
            if before.as_ref() != Some(value) {
                out.push((format!("{name}.{key}"), before, value.clone()));
            }
        }
    }
    out
}

/// One section, key by key: each user value is tried on top of the
/// defaults and kept only if the section still deserializes. Generic over
/// the section type, so new fields need no code here.
fn section<T>(
    name: &str,
    file: &toml::Table,
    ignored: &mut Vec<String>,
    unknown: &mut Vec<String>,
) -> T
where
    T: Serialize + DeserializeOwned + Default,
{
    let Some(value) = file.get(name) else {
        return T::default();
    };
    let Some(user) = value.as_table() else {
        ignored.push(name.to_owned());
        return T::default();
    };
    let Ok(mut accepted) = toml::Table::try_from(T::default()) else {
        return T::default();
    };
    for (key, value) in user {
        if !accepted.contains_key(key) {
            let dotted = format!("{name}.{key}");
            if !RETIRED_KEYS.contains(&dotted.as_str()) {
                unknown.push(dotted);
            }
            continue;
        }
        let mut trial = accepted.clone();
        trial.insert(key.clone(), value.clone());
        if toml::Value::Table(trial.clone()).try_into::<T>().is_ok() {
            accepted = trial;
        } else {
            ignored.push(format!("{name}.{key}"));
        }
    }
    toml::Value::Table(accepted).try_into().unwrap_or_default()
}

/// Session-only settings from the command line, layered over the file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Session {
    pub minimal: bool,
    pub fps: Option<u32>,
    pub style: Option<String>,
    pub palette: Option<String>,
    pub color: Option<ColorChoice>,
    /// Simulation seed (`None` = a new one every launch, from the clock).
    pub seed: Option<u64>,
    /// Quit after this many rendered frames (`None` = run until quit).
    pub max_frames: Option<u64>,
    /// Use this config file instead of the XDG one.
    pub config_path: Option<PathBuf>,
    /// `--demo`: the made-up player, cover and lyrics (`crate::demo`).
    pub demo: bool,
}

/// A setting the CLI overrode for this session. Once the user changes it
/// in the app the override is dropped and the new value is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overridden {
    Mode,
    Fps,
    Style,
    Palette,
    Color,
}

impl Session {
    /// The file's settings with the flags applied, plus which ones the
    /// flags touched (so saving can put the file's values back).
    pub fn apply(&self, file: &Settings) -> (Settings, Vec<Overridden>) {
        let mut live = file.clone();
        let mut touched = Vec::new();
        if self.minimal {
            live.ui.mode = UiMode::Minimal;
            touched.push(Overridden::Mode);
        }
        if let Some(fps) = self.fps {
            live.display.fps = fps;
            touched.push(Overridden::Fps);
        }
        if let Some(style) = &self.style {
            live.lamp.style = style.clone();
            touched.push(Overridden::Style);
        }
        if let Some(palette) = &self.palette {
            live.theme.palette = palette.clone();
            touched.push(Overridden::Palette);
        }
        if let Some(color) = self.color {
            live.display.color = color;
            touched.push(Overridden::Color);
        }
        (live, touched)
    }
}

/// What to write: the live settings, except fields still held by a CLI
/// override, which keep the file's value.
pub fn to_persist(live: &Settings, file: &Settings, overridden: &[Overridden]) -> Settings {
    let mut out = live.clone();
    for o in overridden {
        match o {
            Overridden::Mode => out.ui.mode = file.ui.mode,
            Overridden::Fps => out.display.fps = file.display.fps,
            Overridden::Style => out.lamp.style.clone_from(&file.lamp.style),
            Overridden::Palette => out.theme.palette.clone_from(&file.theme.palette),
            Overridden::Color => out.display.color = file.display.color,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_is_on_unless_a_file_says_otherwise() {
        assert!(Settings::default().input.mouse);
        assert!(Settings::parse("").unwrap().settings.input.mouse);
        let off = Settings::parse("[input]\nmouse = false\n").unwrap();
        assert!(!off.settings.input.mouse, "an explicit value is kept");
    }

    #[test]
    fn word_by_word_reads_names_and_yes_no() {
        use crate::lyrics::Karaoke;
        let k = |text: &str| Settings::parse(text).unwrap().settings.lyrics.karaoke;
        assert_eq!(k("[lyrics]\nkaraoke = \"timed\"\n"), Karaoke::Timed);
        assert_eq!(k("[lyrics]\nkaraoke = \"off\"\n"), Karaoke::Off);
        assert_eq!(k("[lyrics]\nkaraoke = false\n"), Karaoke::Off);
        assert_eq!(k("[lyrics]\nkaraoke = true\n"), Karaoke::On);
        assert_eq!(k(""), Karaoke::On);
        let bad = Settings::parse("[lyrics]\nkaraoke = \"loud\"\n").unwrap();
        assert_eq!(bad.ignored, ["lyrics.karaoke"]);
        assert_eq!(bad.settings.lyrics.karaoke, Karaoke::On);
        let text = toml::to_string(&Settings::default()).unwrap();
        assert!(text.contains("karaoke = \"on\""), "{text}");
    }

    #[test]
    fn lyrics_timing_stays_within_a_second() {
        let parsed = Settings::parse("[lyrics]\ndelay_ms = -250\n").unwrap();
        assert_eq!(parsed.settings.lyrics.delay_ms, -250);
        assert!(parsed.ignored.is_empty() && parsed.unknown.is_empty());
        let parsed = Settings::parse("[lyrics]\ndelay_ms = 5000\n").unwrap();
        assert_eq!(parsed.settings.lyrics.delay_ms, 1000);
        assert_eq!(parsed.clamped.len(), 1, "{:?}", parsed.clamped);
        assert_eq!(Settings::default().lyrics.delay_ms, 0);
    }

    #[test]
    fn spotify_client_id_is_a_plain_setting() {
        let parsed = Settings::parse("[spotify]\nclient_id = \" 0123abcdEF \"\n").unwrap();
        assert_eq!(parsed.settings.spotify.client_id, "0123abcdEF");
        assert_eq!(
            parsed.settings.spotify_client_id().as_deref(),
            Some("0123abcdEF")
        );
        assert!(parsed.ignored.is_empty() && parsed.unknown.is_empty());
        // Not an ID (a quote, a URL): no ID, reported like any clamp.
        let parsed = Settings::parse("[spotify]\nclient_id = \"abc\\\"; rm\"\n").unwrap();
        assert_eq!(parsed.settings.spotify.client_id, "");
        assert_eq!(parsed.clamped.len(), 1, "{:?}", parsed.clamped);
        // A wrong type costs just that key.
        let parsed = Settings::parse("[spotify]\nclient_id = 5\n").unwrap();
        assert_eq!(parsed.ignored, ["spotify.client_id"]);
        let text = toml::to_string(&Settings::default()).unwrap();
        assert!(text.contains("[spotify]\nclient_id = \"\""), "{text}");
    }

    #[test]
    fn spotify_login_store_and_flag_load() {
        let parsed = Settings::parse("[spotify]\nstore = \"file\"\nlogged_in = true\n").unwrap();
        assert_eq!(parsed.settings.spotify.store, LoginStore::File);
        assert!(parsed.settings.spotify.logged_in);
        assert!(parsed.ignored.is_empty() && parsed.unknown.is_empty());
        let parsed = Settings::parse("[spotify]\nstore = \"vault\"\n").unwrap();
        assert_eq!(parsed.ignored, ["spotify.store"]);
        assert_eq!(parsed.settings.spotify.store, LoginStore::System);
        assert!(!Settings::default().spotify.logged_in);
    }

    #[test]
    fn empty_file_is_defaults() {
        let s: Settings = toml::from_str("").unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let s: Settings = toml::from_str("[lamp]\nstyle = \"braille\"\n").unwrap();
        assert_eq!(s.lamp.style, "braille");
        assert_eq!(s.lamp.heat, 3);
        assert_eq!(s.clock, Clock::default());
    }

    #[test]
    fn round_trips_every_field() {
        let mut s = Settings::default();
        s.display.color = ColorChoice::Ansi256;
        s.ui.mode = UiMode::Minimal;
        s.minimal.clock = MinimalClock::Corner;
        s.pomodoro.focus_min = 50;
        s.clock.seconds = false;
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("color = \"256\""), "{text}");
        assert!(text.contains("seconds = false"), "{text}");
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
    }

    #[test]
    fn one_bad_value_keeps_the_rest() {
        for bad in ["heat = -1", "heat = 300", "speed = \"fast\""] {
            let text = format!("[lamp]\nstyle = \"ascii\"\n{bad}\n[clock]\nface = \"words\"\n");
            let p = Settings::parse(&text).unwrap();
            assert_eq!(p.settings.lamp.style, "ascii", "{bad}");
            assert_eq!(p.settings.clock.face, "words", "{bad}");
            assert_eq!(p.ignored.len(), 1, "{bad}: {:?}", p.ignored);
            assert!(p.ignored[0].starts_with("lamp."), "{bad}");
        }
        for (section, bad) in [
            ("display", "fps = 60.0"),
            ("display", "color = \"24bit\""),
            ("pomodoro", "focus_min = -5"),
        ] {
            let text = format!("[lamp]\nstyle = \"ascii\"\n[{section}]\n{bad}\n");
            let p = Settings::parse(&text).unwrap();
            assert_eq!(p.settings.lamp.style, "ascii", "{bad}");
            let key = bad.split(' ').next().unwrap();
            assert_eq!(p.ignored, [format!("{section}.{key}")]);
        }
    }

    #[test]
    fn bad_value_takes_the_default_and_neighbours_survive() {
        let text = "[pomodoro]\nfocus_min = -5\nshort_break_min = 7\ncycles = 2\n";
        let p = Settings::parse(text).unwrap();
        assert_eq!(p.settings.pomodoro.focus_min, 25);
        assert_eq!(p.settings.pomodoro.short_break_min, 7);
        assert_eq!(p.settings.pomodoro.cycles, 2);
    }

    #[test]
    fn unknown_keys_and_malformed_sections_are_reported() {
        let text = "colour = 1\nlamp = 5\n[clock]\nfase = \"x\"\nhour24 = false\n";
        let p = Settings::parse(text).unwrap();
        assert_eq!(p.ignored, ["lamp"]);
        assert_eq!(p.unknown, ["colour", "clock.fase"]);
        assert!(!p.settings.clock.hour24);
        assert_eq!(p.settings.lamp, Lamp::default());
    }

    #[test]
    fn unknown_names_take_the_default() {
        let text = "[lamp]\nstyle = \"nope\"\nheat = 4\n[theme]\npalette = \"beige\"\n[clock]\nface = \"sundial\"\n";
        let p = Settings::parse(text).unwrap();
        assert_eq!(p.ignored, ["lamp.style", "theme.palette", "clock.face"]);
        assert_eq!(p.settings.lamp.style, Lamp::default().style);
        assert_eq!(p.settings.theme.palette, ThemeSettings::default().palette);
        assert_eq!(p.settings.clock.face, Clock::default().face);
        assert_eq!(p.settings.lamp.heat, 4);
        // An old style name is still a style.
        let p = Settings::parse("[lamp]\nstyle = \"glass\"\n").unwrap();
        assert!(p.ignored.is_empty());
        assert_eq!(p.settings.lamp.style, "glass");
        // A retired style quietly becomes the default.
        for style in RETIRED_STYLES {
            let p = Settings::parse(&format!("[lamp]\nstyle = \"{style}\"\n")).unwrap();
            assert_eq!(p.ignored, [] as [&str; 0]);
            assert_eq!(p.settings.lamp.style, Lamp::default().style);
            assert!(StyleId::by_name(style).is_none(), "{style}");
        }
    }

    #[test]
    fn retired_keys_load_quietly() {
        let text = "[lamp]\nframe = \"glass\"\nlighting = true\nheat = 4\n";
        let p = Settings::parse(text).unwrap();
        assert!(p.ignored.is_empty() && p.unknown.is_empty(), "{p:?}");
        assert!(p.clamped.is_empty());
        assert_eq!(p.settings.lamp.heat, 4);
    }

    #[test]
    fn dock_backing_loads_and_defaults_to_none() {
        assert_eq!(Settings::default().dock.backing, dock::Backing::None);
        let p = Settings::parse("[dock]\nbacking = \"soft\"\n").unwrap();
        assert_eq!(p.settings.dock.backing, dock::Backing::Soft);
        assert!(p.ignored.is_empty() && p.unknown.is_empty());
        let p = Settings::parse("[dock]\nbacking = \"glass\"\n").unwrap();
        assert_eq!(p.ignored, ["dock.backing"]);
        assert_eq!(p.settings.dock.backing, dock::Backing::None);
    }

    #[test]
    fn dock_text_loads_and_defaults_to_auto() {
        assert_eq!(Settings::default().dock.text, dock::TextInk::Auto);
        for (name, ink) in [
            ("light", dock::TextInk::Light),
            ("dark", dock::TextInk::Dark),
        ] {
            let p = Settings::parse(&format!("[dock]\ntext = \"{name}\"\n")).unwrap();
            assert_eq!(p.settings.dock.text, ink);
            assert!(p.ignored.is_empty() && p.unknown.is_empty());
        }
        let p = Settings::parse("[dock]\ntext = \"white\"\nclock = \"overlay\"\n").unwrap();
        assert_eq!(p.ignored, ["dock.text"]);
        assert_eq!(p.settings.dock.text, dock::TextInk::Auto);
        assert_eq!(p.settings.dock.place(&dock::Clock), dock::Place::Overlay);
    }

    #[test]
    fn art_settings_load_and_a_bad_value_costs_only_itself() {
        use crate::dock::cover::{CoverSize, Detail};
        let art = Settings::default().art;
        assert_eq!(
            (art.detail, art.size, art.inline),
            (Detail::Auto, CoverSize::Medium, true)
        );
        let p = Settings::parse("[art]\ndetail = \"pixels\"\nsize = \"fill\"\n").unwrap();
        assert_eq!(
            (p.settings.art.detail, p.settings.art.size),
            (Detail::Sharp, CoverSize::Fill)
        );
        assert!(p.ignored.is_empty() && p.unknown.is_empty());
        let p = Settings::parse("[art]\ndetail = \"sixel\"\nsize = \"small\"\n").unwrap();
        assert_eq!(p.ignored, ["art.detail"]);
        assert_eq!(
            (p.settings.art.detail, p.settings.art.size),
            (Detail::Auto, CoverSize::Small)
        );
    }

    #[test]
    fn dock_places_load_and_a_hidden_clock_carries_over() {
        let clock = &dock::Clock;
        let p = Settings::parse("[dock]\nclock = \"overlay\"\nanchor = \"top-left\"\n").unwrap();
        assert_eq!(p.settings.dock.place(clock), Place::Overlay);
        assert_eq!(p.settings.dock.anchor(clock), dock::Anchor::TopLeft);
        assert_eq!(p.settings.dock.anchor(&dock::Music), dock::Anchor::TopLeft);
        assert_eq!(p.settings.dock.place(&dock::Pomodoro), Place::Side);
        // The old single anchor is split per widget, so the next save
        // writes the new form.
        assert!(matches!(p.settings.dock.anchor, dock::Anchors::Each(_)));
        assert!(matches!(p.baseline.dock.anchor, dock::Anchors::All(_)));
        let p = Settings::parse("[dock]\nanchor = { music = \"bottom-left\" }\n").unwrap();
        assert!(p.ignored.is_empty() && p.unknown.is_empty(), "{p:?}");
        assert_eq!(
            p.settings.dock.anchor(&dock::Music),
            dock::Anchor::BottomLeft
        );
        assert_eq!(p.settings.dock.anchor(clock), dock::Anchor::Center);
        // A bad place costs just that key; a widget that doesn't exist
        // (yet) is an unknown key, kept in the file.
        let p = Settings::parse("[dock]\nclock = \"sideways\"\nradio = \"side\"\n").unwrap();
        assert_eq!(p.ignored, ["dock.clock"]);
        assert_eq!(p.unknown, ["dock.radio"]);
        assert_eq!(p.settings.dock, DockSettings::default());
        // v1.1's `clock.show = false` hides the clock, quietly…
        let p = Settings::parse("[clock]\nshow = false\n").unwrap();
        assert!(p.ignored.is_empty() && p.unknown.is_empty(), "{p:?}");
        assert_eq!(p.settings.dock.place(clock), Place::Off);
        assert_eq!(p.baseline.dock.place(clock), Place::Side);
        // …unless the file also places it.
        let p = Settings::parse("[clock]\nshow = false\n[dock]\nclock = \"overlay\"\n").unwrap();
        assert_eq!(p.settings.dock.place(clock), Place::Overlay);
        let p = Settings::parse("[clock]\nshow = true\n").unwrap();
        assert_eq!(p.settings, Settings::default());
    }

    #[test]
    fn parse_clamps_and_rejects_syntax_errors() {
        let p = Settings::parse("[lamp]\nheat = 9\n").unwrap();
        assert_eq!(p.settings.lamp.heat, 5);
        assert!(p.ignored.is_empty());
        assert!(Settings::parse("[lamp\n").is_err());
    }

    #[test]
    fn sanitize_clamps() {
        let mut s = Settings::default();
        s.display.fps = 0;
        s.display.cell_aspect = f64::NAN;
        s.lamp.heat = 9;
        s.lamp.speed = -1.0;
        s.pomodoro.focus_min = 0;
        s.pomodoro.cycles = 0;
        let s = s.sanitized();
        assert_eq!(s.display.fps, 1);
        assert_eq!(s.display.cell_aspect, 2.0);
        assert_eq!(s.lamp.heat, 5);
        assert_eq!(s.lamp.speed, 1.0);
        assert_eq!(s.pomodoro.focus_min, 1);
        assert_eq!(s.pomodoro.cycles, 1);
    }

    #[test]
    fn overrides_are_not_persisted() {
        let file = Settings::default();
        let session = Session {
            minimal: true,
            style: Some("ascii".into()),
            ..Session::default()
        };
        let (mut live, touched) = session.apply(&file);
        assert!(live.minimal());
        assert_eq!(live.lamp.style, "ascii");
        live.lamp.heat = 5;
        let out = to_persist(&live, &file, &touched);
        assert_eq!(out.ui.mode, UiMode::Full);
        assert_eq!(out.lamp.style, "solid");
        assert_eq!(out.lamp.heat, 5);
    }
}
