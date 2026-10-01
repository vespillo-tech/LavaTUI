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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lamp {
    pub style: String,
    pub frame: FrameMode,
    pub lighting: bool,
    /// 1..=5.
    pub heat: u8,
    /// 0.25 | 0.5 | 1 | 2 | 4 (snapped to the nearest).
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeSettings {
    pub palette: String,
    /// Never paint `bg` outside the glass.
    pub transparent: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    pub face: String,
    pub show: bool,
    pub hour24: bool,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Minimal {
    pub clock: MinimalClock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Input {
    /// Mouse capture (off by default: it breaks native text selection).
    pub mouse: bool,
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

/// `lamp.frame`; `f` cycles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FrameMode {
    #[default]
    Auto,
    Glass,
    Bleed,
}

impl FrameMode {
    pub fn next(self) -> Self {
        match self {
            FrameMode::Auto => FrameMode::Glass,
            FrameMode::Glass => FrameMode::Bleed,
            FrameMode::Bleed => FrameMode::Auto,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FrameMode::Auto => "auto",
            FrameMode::Glass => "glass",
            FrameMode::Bleed => "bleed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UiMode {
    #[default]
    Full,
    Minimal,
}

/// Where minimal mode puts its tiny clock (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MinimalClock {
    #[default]
    Under,
    Corner,
    Off,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            fps: 60,
            color: ColorChoice::Auto,
            cell_aspect: 2.0,
        }
    }
}

impl Default for Lamp {
    fn default() -> Self {
        Self {
            style: "solid".into(),
            frame: FrameMode::Auto,
            lighting: false,
            heat: 3,
            speed: 1.0,
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
            show: true,
            hour24: true,
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
}

impl Settings {
    /// The sections of `config.toml`, in file order.
    const SECTIONS: [&str; 8] = [
        "display", "lamp", "theme", "clock", "pomodoro", "ui", "minimal", "input",
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
        };
        settings.check_names(&mut out.ignored);
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
    /// nothing, reporting it in `ignored`.
    fn check_names(&mut self, ignored: &mut Vec<String>) {
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
        self
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
            unknown.push(format!("{name}.{key}"));
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
    fn empty_file_is_defaults() {
        let s: Settings = toml::from_str("").unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let s: Settings = toml::from_str("[lamp]\nstyle = \"dither\"\n").unwrap();
        assert_eq!(s.lamp.style, "dither");
        assert_eq!(s.lamp.heat, 3);
        assert_eq!(s.clock, Clock::default());
    }

    #[test]
    fn round_trips_every_field() {
        let mut s = Settings::default();
        s.display.color = ColorChoice::Ansi256;
        s.lamp.frame = FrameMode::Bleed;
        s.ui.mode = UiMode::Minimal;
        s.minimal.clock = MinimalClock::Corner;
        s.pomodoro.focus_min = 50;
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("color = \"256\""), "{text}");
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), s);
    }

    #[test]
    fn one_bad_value_keeps_the_rest() {
        for bad in ["frame = \"round\"", "heat = 300", "speed = \"fast\""] {
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
