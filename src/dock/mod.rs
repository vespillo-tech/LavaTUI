//! The widget dock: the things that sit around or on the lamp (the clock,
//! the pomodoro, …), each in a place the user picks — the side panel, on
//! the lava, or off.
//!
//! A widget is a unit struct implementing [`DockWidget`], listed once in
//! [`WIDGETS`]. Like a clock face it offers a few fixed-size *forms*,
//! most preferred first; the layout (`ui/layout.rs`, pure) picks the
//! largest combination that fits the side panel or the lava, in the §1.3
//! hide order, and the widget draws the form it was given. A widget whose
//! place has no room falls back to the one-line chip ([`DockWidget::chip`]).
//!
//! Widgets are stateless views: whatever they show lives on the app
//! [`Model`] (the pomodoro, the local time, the player's snapshot) and
//! they read it from there.
//! Colours come from the model's theme, as everywhere else.

mod clock;
mod lyrics;
mod music;
mod pomodoro;

use std::collections::BTreeMap;

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect, Size};
use serde::{Deserialize, Serialize};

use crate::app::Model;
use crate::theme::Role;

pub use clock::Clock;
#[cfg(test)]
pub use clock::{clock_forms, clock_parts};
pub use lyrics::Lyrics;
#[cfg(test)]
pub use lyrics::{Show as LyricsShow, lyrics_forms};
pub use music::{Music, fit, hit as music_hit};
#[cfg(test)]
pub use music::{Show, music_forms};
pub use pomodoro::Pomodoro;
#[cfg(test)]
pub use pomodoro::pomodoro_forms;

/// Every widget, in stacking order: the first sits on top of the panel
/// (and of the stack on the lava) and is the last to shrink.
pub static WIDGETS: &[&dyn DockWidget] = &[&Clock, &Pomodoro, &Music, &Lyrics];

/// Where a widget sits (`dock.<name>` in the config).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Place {
    /// In the panel beside (or below) the lamp.
    Side,
    /// On the lava, at the dock's [`Anchor`].
    Overlay,
    Off,
}

impl Place {
    /// The next place a widget's key moves it to.
    pub fn next(self) -> Self {
        match self {
            Place::Side => Place::Overlay,
            Place::Overlay => Place::Off,
            Place::Off => Place::Side,
        }
    }

    /// For toasts: `clock · on the lava`.
    pub fn describe(self) -> &'static str {
        match self {
            Place::Side => "side panel",
            Place::Overlay => "on the lava",
            Place::Off => "off",
        }
    }
}

/// Where on the lava a widget sits (`dock.anchor.<name>`). Widgets that
/// share an anchor stack there; different anchors spread across the lamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    Center,
    Top,
    TopRight,
    BottomRight,
    Bottom,
    BottomLeft,
    TopLeft,
}

impl Anchor {
    /// Cycle order: the centre, then once round the edge, clockwise.
    pub const ALL: [Anchor; 7] = [
        Anchor::Center,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::BottomRight,
        Anchor::Bottom,
        Anchor::BottomLeft,
        Anchor::TopLeft,
    ];

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&a| a == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            Anchor::Center => "centre",
            Anchor::Top => "top",
            Anchor::TopRight => "top right",
            Anchor::BottomRight => "bottom right",
            Anchor::Bottom => "bottom",
            Anchor::BottomLeft => "bottom left",
            Anchor::TopLeft => "top left",
        }
    }

    /// Horizontal alignment of the stack's widgets.
    pub fn align(self) -> Alignment {
        match self {
            Anchor::TopLeft | Anchor::BottomLeft => Alignment::Left,
            Anchor::TopRight | Anchor::BottomRight => Alignment::Right,
            _ => Alignment::Center,
        }
    }

    /// Where in the free space the stack goes, per axis: 0 start, 1
    /// middle, 2 end.
    pub fn grid(self) -> (u8, u8) {
        match self {
            Anchor::Center => (1, 1),
            Anchor::Top => (1, 0),
            Anchor::TopRight => (2, 0),
            Anchor::BottomRight => (2, 2),
            Anchor::Bottom => (1, 2),
            Anchor::BottomLeft => (0, 2),
            Anchor::TopLeft => (0, 0),
        }
    }
}

/// The `[dock]` config section: each widget's place by name, and its
/// anchor on the lava (`anchor = { clock = "top", … }`). Widgets add their
/// own keys just by being in [`WIDGETS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockSettings {
    pub anchor: Anchors,
    /// What the widgets on the lava sit on.
    pub backing: Backing,
    #[serde(flatten)]
    pub places: BTreeMap<String, Place>,
}

/// `dock.backing`: what the widgets on the lava sit on (§4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backing {
    /// Nothing: the text floats on the lamp, each cell keeping its colour.
    #[default]
    None,
    /// A soft pool of veiled liquid behind them (the v1.2 look).
    Soft,
}

/// `dock.anchor`: one per widget. Files from before v1.2 have a single
/// anchor for all of them (`anchor = "top-left"`), which still loads (and
/// is written back per widget at the next save).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Anchors {
    All(Anchor),
    Each(BTreeMap<String, Anchor>),
}

impl Default for Anchors {
    fn default() -> Self {
        Self::Each(
            WIDGETS
                .iter()
                .map(|w| (w.name().to_owned(), w.default_anchor()))
                .collect(),
        )
    }
}

impl Default for DockSettings {
    fn default() -> Self {
        Self {
            anchor: Anchors::default(),
            backing: Backing::default(),
            places: WIDGETS
                .iter()
                .map(|w| (w.name().to_owned(), w.default_place()))
                .collect(),
        }
    }
}

impl DockSettings {
    pub fn place(&self, widget: &dyn DockWidget) -> Place {
        self.places
            .get(widget.name())
            .copied()
            .unwrap_or_else(|| widget.default_place())
    }

    pub fn set(&mut self, widget: &dyn DockWidget, place: Place) {
        self.places.insert(widget.name().to_owned(), place);
    }

    pub fn anchor(&self, widget: &dyn DockWidget) -> Anchor {
        match &self.anchor {
            Anchors::All(anchor) => *anchor,
            Anchors::Each(each) => each
                .get(widget.name())
                .copied()
                .unwrap_or_else(|| widget.default_anchor()),
        }
    }

    pub fn set_anchor(&mut self, widget: &dyn DockWidget, anchor: Anchor) {
        self.split_anchors();
        if let Anchors::Each(each) = &mut self.anchor {
            each.insert(widget.name().to_owned(), anchor);
        }
    }

    /// An old single anchor becomes one per widget (each gets it).
    pub fn split_anchors(&mut self) {
        if let Anchors::All(all) = self.anchor {
            self.anchor =
                Anchors::Each(WIDGETS.iter().map(|w| (w.name().to_owned(), all)).collect());
        }
    }
}

/// What a form needs of the terminal beyond its own size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Needs {
    /// Only in Huge terminals (§1.2): the XL faces.
    pub huge: bool,
    /// Only in tall ones (≥ 36 rows): the date line.
    pub tall: bool,
}

/// The room a terminal has, which [`Needs`] are checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Room {
    pub huge: bool,
    pub tall: bool,
}

impl Needs {
    pub fn met(self, room: Room) -> bool {
        (!self.huge || room.huge) && (!self.tall || room.tall)
    }
}

/// One way to draw a widget, at a size fixed in advance (sizes never
/// depend on the time, so nothing jitters from one minute to the next).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetForm {
    /// The height, exactly; the width, exactly unless `fill`.
    pub size: Size,
    /// Takes the whole width of its column (`size.width` at least).
    pub fill: bool,
    pub needs: Needs,
    /// The readout changes every second (a frozen lamp wakes for it).
    pub seconds: bool,
    /// Which of the widget's own variants this is; only it reads this.
    pub variant: u16,
}

impl WidgetForm {
    pub fn fixed(width: u16, height: u16, variant: u16) -> Self {
        Self {
            size: Size::new(width, height),
            fill: false,
            needs: Needs::default(),
            seconds: false,
            variant,
        }
    }

    pub fn fill(min_width: u16, height: u16, variant: u16) -> Self {
        Self {
            fill: true,
            ..Self::fixed(min_width, height, variant)
        }
    }
}

/// What a widget is drawn over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backdrop {
    /// The app background, beside the lamp.
    Panel,
    /// The lava. Spaces stay see-through; the ui composites the rest onto
    /// the lamp (and the soft backing, if `dock.backing` asks for one).
    Lava,
}

/// How to draw a form in its rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub backdrop: Backdrop,
    /// Where a form narrower than its rect sits in it (odd cells go right).
    pub align: Alignment,
}

/// A widget's one-line fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipText {
    pub text: String,
    pub ink: Role,
}

pub trait DockWidget: Sync {
    /// Lowercase name: the config key `dock.<name>` and the toast.
    fn name(&self) -> &'static str;

    fn default_place(&self) -> Place {
        Place::Side
    }

    /// Where it sits on the lava until moved (`l`).
    fn default_anchor(&self) -> Anchor {
        Anchor::Center
    }

    /// How much it matters right now: the higher, the longer it keeps its
    /// size, and the later it's dropped to the chip row (ties: earlier in
    /// [`WIDGETS`] wins). Clock 1; a running pomodoro 3; music 2 while
    /// playing.
    fn rank(&self, _model: &Model) -> u8 {
        1
    }

    /// The forms this widget offers in `place` (side or overlay), most
    /// preferred first. The layout filters them by room and size.
    fn forms(&self, model: &Model, place: Place) -> Vec<WidgetForm>;

    /// Draw `form` (one of [`forms`](Self::forms)) inside `area`, which
    /// is the form's size (wider for a fill form). Never outside `area`.
    fn draw(&self, model: &Model, form: WidgetForm, area: Rect, look: Look, buf: &mut Buffer);

    /// The one-line chip shown when this widget's place has no room, or
    /// `None` when it has nothing to say there.
    fn chip(&self, model: &Model) -> Option<ChipText>;
}

pub fn by_name(name: &str) -> Option<(usize, &'static dyn DockWidget)> {
    WIDGETS
        .iter()
        .copied()
        .enumerate()
        .find(|(_, w)| w.name() == name)
}

/// `x` offset of something `w` wide in `avail` columns.
pub fn align_x(align: Alignment, avail: u16, w: u16) -> u16 {
    let spare = avail.saturating_sub(w);
    match align {
        Alignment::Left => 0,
        Alignment::Center => spare / 2,
        Alignment::Right => spare,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_lowercase() {
        for (i, w) in WIDGETS.iter().enumerate() {
            assert_eq!(w.name(), w.name().to_lowercase());
            assert!(WIDGETS[i + 1..].iter().all(|o| o.name() != w.name()));
            assert_ne!(w.name(), "anchor", "dock.anchor is taken");
            assert_ne!(w.name(), "backing", "dock.backing is taken");
        }
    }

    #[test]
    fn places_and_anchors_cycle_through_every_value() {
        let mut p = Place::Side;
        for _ in 0..3 {
            p = p.next();
        }
        assert_eq!(p, Place::Side);
        let mut a = Anchor::default();
        let mut seen = vec![a];
        for _ in 1..Anchor::ALL.len() {
            a = a.next();
            assert!(!seen.contains(&a));
            seen.push(a);
        }
        assert_eq!(a.next(), Anchor::default());
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let mut d = DockSettings::default();
        d.set(&Clock, Place::Overlay);
        d.set_anchor(&Clock, Anchor::TopLeft);
        let text = toml::to_string(&d).unwrap();
        assert!(text.contains("clock = \"top-left\""), "{text}");
        assert!(text.contains("clock = \"overlay\""), "{text}");
        assert_eq!(toml::from_str::<DockSettings>(&text).unwrap(), d);
        assert_eq!(d.place(&Pomodoro), Place::Side);
        assert_eq!(d.anchor(&Pomodoro), Anchor::Center);
        assert_eq!(d.anchor(&Music), Anchor::TopLeft);
        // A file from before per-widget anchors: one for all.
        let old: DockSettings = toml::from_str("anchor = \"bottom\"").unwrap();
        assert_eq!(old.anchor(&Clock), Anchor::Bottom);
        assert_eq!(old.anchor(&Music), Anchor::Bottom);
        let mut split = old.clone();
        split.set_anchor(&Clock, Anchor::Top);
        assert_eq!(
            (split.anchor(&Clock), split.anchor(&Pomodoro)),
            (Anchor::Top, Anchor::Bottom)
        );
    }
}
