//! Settings: the persisted TOML surface of docs/design.md §9, plus the
//! session-only CLI overrides.
//!
//! [`Settings`] is what lives in `config.toml` (XDG config dir, see
//! [`store`]). Every field has a default and every section is optional, so
//! a partial file is fine. Values out of range are clamped and unknown
//! names (a style that no longer exists) fall back where they're resolved,
//! so a stale file never stops the lamp.
//!
//! [`Session`] is what the CLI adds on top: flags win for this run only and
//! are never written back (§9).

pub mod store;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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

impl Settings {
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
        if !self.lamp.speed.is_finite() || self.lamp.speed <= 0.0 {
            self.lamp.speed = 1.0;
        }
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
