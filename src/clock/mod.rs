//! Clock faces (`Face` trait + registry) and the pomodoro state machine.
//!
//! Pure logic: time is always passed in (no `Instant::now()` / wall clock
//! reads in here) and faces draw into a ratatui [`Buffer`] only, never the
//! terminal. Colours come from the caller via [`FaceStyle`] /
//! [`PomodoroStyle`].
//!
//! **Sizing** (design.md §4.5): every face offers a few fixed-size *forms*,
//! largest first. A face is drawn in the largest form that fits the rect it
//! is given, dropping seconds, then `am`/`pm`, then shrinking a size tier,
//! and finally falling back to plain text (`14:32`, 5×1). Below 5×1 nothing
//! is drawn at all: a face never truncates or overflows.

mod analog;
mod binary;
mod blocks;
mod draw;
mod pomodoro;
mod pomodoro_view;
mod segment;
mod text;
mod words;

pub use analog::Analog;
pub use binary::Binary;
pub use blocks::Blocks;
pub use pomodoro::{PhaseEnd, Pomodoro, PomodoroConfig, Status, format_remaining};
pub use pomodoro_view::{PomodoroStyle, PomodoroWidget};
pub use segment::Segment;
pub use text::{Text, readout};
pub use words::Words;

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;

/// A wall-clock time of day. Built by the caller from whatever local-time
/// source the app uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl ClockTime {
    /// `None` unless `hour < 24`, `minute < 60` and `second < 60`.
    pub fn new(hour: u8, minute: u8, second: u8) -> Option<Self> {
        (hour < 24 && minute < 60 && second < 60).then_some(Self {
            hour,
            minute,
            second,
        })
    }

    /// From seconds since local midnight (wraps past 24h).
    pub fn from_secs_of_day(secs: u32) -> Self {
        let secs = secs % 86_400;
        Self {
            hour: (secs / 3600) as u8,
            minute: (secs / 60 % 60) as u8,
            second: (secs % 60) as u8,
        }
    }

    /// 0–23, or 1–12 in 12h mode.
    pub fn display_hour(self, hour24: bool) -> u8 {
        match (hour24, self.hour % 12) {
            (true, _) => self.hour,
            (false, 0) => 12,
            (false, h) => h,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceOptions {
    /// 24h (`14:32`) vs 12h (` 2:32 pm`).
    pub hour24: bool,
    /// Show seconds where the form has room for them. They're the first
    /// thing dropped when space is short.
    pub seconds: bool,
}

impl Default for FaceOptions {
    fn default() -> Self {
        Self {
            hour24: true,
            seconds: false,
        }
    }
}

/// Caller-supplied styling. `main` is the readout; `dim` is secondary ink
/// (analog ticks, unlit binary bits and word-grid letters, ghost segments,
/// `am`/`pm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceStyle {
    pub main: Style,
    pub dim: Style,
}

impl Default for FaceStyle {
    fn default() -> Self {
        Self {
            main: Style::new(),
            dim: Style::new().add_modifier(Modifier::DIM),
        }
    }
}

/// Size class of a form (design.md §4.5 columns).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Text,
    S,
    M,
    L,
    XL,
}

/// One concrete way to draw a face: a tier plus which optional parts are
/// shown, and the exact size that takes. Sizes don't depend on the time,
/// so a layout never jitters from one minute to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Form {
    pub tier: Tier,
    pub seconds: bool,
    pub meridiem: bool,
    pub size: Size,
}

/// A clock face. Implementors list their own forms and draw them; fitting,
/// the text fallback and the widget are provided.
pub trait Face: Sync {
    /// Lowercase registry / config name, e.g. `"blocks"`.
    fn name(&self) -> &'static str;

    /// This face's own forms, most preferred first. Exclude the text
    /// fallback; [`Face::all_forms`] appends it.
    fn forms(&self, opts: FaceOptions) -> Vec<Form>;

    /// Draw `form` (one this face returned) at the top-left of `area`.
    /// Must stay inside `form.size` ∩ `area`; use [`draw::Pen`] for that.
    fn draw(
        &self,
        form: Form,
        time: ClockTime,
        opts: FaceOptions,
        area: Rect,
        buf: &mut Buffer,
        style: FaceStyle,
    );

    /// Own forms followed by the text fallback, most preferred first.
    fn all_forms(&self, opts: FaceOptions) -> Vec<Form> {
        let mut forms = self.forms(opts);
        if forms.last().is_none_or(|f| f.tier != Tier::Text) {
            forms.extend(Text.forms(opts));
        }
        forms
    }

    /// The largest form that fits in `avail`, or `None` if not even the
    /// text fallback does (draw nothing then).
    fn fit(&self, opts: FaceOptions, avail: Size) -> Option<Form> {
        self.all_forms(opts)
            .into_iter()
            .find(|f| f.size.width <= avail.width && f.size.height <= avail.height)
    }

    /// Size of the most preferred form.
    fn preferred_size(&self, opts: FaceOptions) -> Size {
        self.all_forms(opts)[0].size
    }

    /// Size of the smallest form (always the text fallback, 5×1).
    fn min_size(&self, opts: FaceOptions) -> Size {
        let forms = self.all_forms(opts);
        forms[forms.len() - 1].size
    }
}

/// Every face, in cycling order. `blocks` is the default.
pub static FACES: &[&dyn Face] = &[&Blocks, &Segment, &Analog, &Binary, &Words, &Text];

pub fn default_face() -> &'static dyn Face {
    FACES[0]
}

pub fn face_by_name(name: &str) -> Option<&'static dyn Face> {
    FACES.iter().copied().find(|f| f.name() == name)
}

/// The face after `name` in [`FACES`], wrapping (unknown names → default).
pub fn next_face(name: &str) -> &'static dyn Face {
    match FACES.iter().position(|f| f.name() == name) {
        Some(i) => FACES[(i + 1) % FACES.len()],
        None => default_face(),
    }
}

/// A face drawing one time, as a ratatui widget. Picks the largest form
/// that fits the render area; draws nothing if even `14:32` doesn't fit.
#[derive(Clone, Copy)]
pub struct ClockWidget<'a> {
    face: &'a dyn Face,
    time: ClockTime,
    opts: FaceOptions,
    style: FaceStyle,
    alignment: Alignment,
}

impl<'a> ClockWidget<'a> {
    pub fn new(face: &'a dyn Face, time: ClockTime) -> Self {
        Self {
            face,
            time,
            opts: FaceOptions::default(),
            style: FaceStyle::default(),
            alignment: Alignment::Left,
        }
    }

    pub fn options(mut self, opts: FaceOptions) -> Self {
        self.opts = opts;
        self
    }

    pub fn style(mut self, style: FaceStyle) -> Self {
        self.style = style;
        self
    }

    /// Horizontal placement of the form inside the area (top-aligned).
    /// Odd leftover cells go right, per design.md §1.4.
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// The form this widget would draw in an area of `avail`.
    pub fn fit(&self, avail: Size) -> Option<Form> {
        self.face.fit(self.opts, avail)
    }
}

impl Widget for ClockWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

impl Widget for &ClockWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let Some(form) = self.fit(area.as_size()) else {
            return;
        };
        let spare = area.width - form.size.width;
        let dx = match self.alignment {
            Alignment::Left => 0,
            Alignment::Center => spare / 2,
            Alignment::Right => spare,
        };
        let rect = Rect::new(area.x + dx, area.y, form.size.width, form.size.height);
        let face = if form.tier == Tier::Text {
            &Text
        } else {
            self.face
        };
        face.draw(form, self.time, self.opts, rect, buf, self.style);
    }
}

/// `(seconds, meridiem)` combinations to try for a digital readout, most
/// complete first: seconds go before `am`/`pm`.
fn readout_variants(opts: FaceOptions) -> &'static [(bool, bool)] {
    match (opts.seconds, opts.hour24) {
        (true, true) => &[(true, false), (false, false)],
        (false, true) => &[(false, false)],
        (true, false) => &[(true, true), (false, true), (false, false)],
        (false, false) => &[(false, true), (false, false)],
    }
}

/// Forms for each tier × readout variant, with `size(tier, seconds, meridiem)`.
fn readout_forms(
    opts: FaceOptions,
    tiers: &[Tier],
    size: impl Fn(Tier, bool, bool) -> (usize, usize),
) -> Vec<Form> {
    let mut forms = Vec::new();
    for &tier in tiers {
        for &(seconds, meridiem) in readout_variants(opts) {
            let (w, h) = size(tier, seconds, meridiem);
            forms.push(Form {
                tier,
                seconds,
                meridiem,
                size: Size::new(w as u16, h as u16),
            });
        }
    }
    forms
}

#[cfg(test)]
mod tests;
