//! Responsive layout (docs/design.md §1): a pure function from the
//! terminal size and a few settings to non-overlapping placed elements.
//! No terminal access and no state, so it is exhaustively testable (see
//! `ui/tests.rs`).
//!
//! The rules, in brief:
//!
//! * The lamp is always placed (unless the screen is under 4×2). It has no
//!   frame: it fills whatever the status bar and the panel leave.
//! * The panel (clock + pomodoro) goes right of the lamp when the screen
//!   is wider than tall, else below it; if it doesn't fit it collapses
//!   into a one-line chip over the lamp's corner.
//! * Things drop out whole in the §1.3 hide order: the date line, then
//!   face size, before the panel goes.
//!
//! Odd leftover cells always go right/bottom so nothing jitters by a cell
//! between neighbouring sizes.

use ratatui::layout::Rect;

use crate::clock::{Face, FaceOptions, Form, Tier};
use crate::config::MinimalClock;

/// Everything the layout depends on besides the terminal size.
#[derive(Clone, Copy)]
pub struct LayoutInput<'a> {
    pub minimal: bool,
    pub status_bar: bool,
    pub show_clock: bool,
    pub face: &'a dyn Face,
    pub hour24: bool,
    /// What a chip would show, and its text width (without padding).
    pub chip: Option<(ChipKind, u16)>,
    pub minimal_clock: MinimalClock,
    /// Cell height ÷ width (§2.3).
    pub cell_aspect: f64,
}

/// The clock + pomodoro block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panel {
    /// The whole block, including 1 col of padding each side.
    pub rect: Rect,
    /// The clock face and the form to draw it in (none when hidden).
    pub face: Option<(Rect, Form)>,
    pub date: Option<Rect>,
    /// Three rows: label + dots, time, bar.
    pub pomodoro: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipKind {
    Clock,
    Pomodoro,
}

/// A one-line clock or pomodoro readout, drawn over the lamp's corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chip {
    pub rect: Rect,
    pub kind: ChipKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Layout {
    pub area: Rect,
    /// Where the wax is drawn ([`crate::render::LampView`]'s rect).
    pub lamp: Option<Rect>,
    pub status: Option<Rect>,
    pub panel: Option<Panel>,
    pub chip: Option<Chip>,
    /// The row toasts are centred in.
    pub toast: Option<Rect>,
}

/// Panel width bounds (incl. 1-col padding each side).
const PANEL_W: (u16, u16) = (22, 36);
/// From Huge's column count up, the panel may grow to this so the widest
/// faces fit (blocks XL 51, blocks L with seconds 54); it only grows as
/// far as the face in it needs.
const PANEL_W_WIDE: u16 = 56;
/// Rows of the pomodoro block, and the gap above it.
const POMODORO_ROWS: u16 = 3;
const FACE_GAP: u16 = 2;

/// Size cuts, `(cols, rows)`: a terminal is in a tier when it has at
/// least both (§1.2). Below `TINY` is Micro; Huge is its own check
/// ([`is_huge`]), as only the panel and face size care about it.
pub const TINY: (u16, u16) = (20, 8);
pub const SMALL: (u16, u16) = (40, 14);
pub const MEDIUM: (u16, u16) = (80, 24);
pub const HUGE: (u16, u16) = (200, 56);
/// Overlays pick their form by their own cuts: the help's centred sheet
/// (§4.3) and the picker's side sheet (§4.4).
pub const HELP_SHEET: (u16, u16) = (68, 20);
pub const PICKER_SHEET: (u16, u16) = (80, 16);
/// §1.3: the status bar, toasts, and the panel's date line (rows only).
const STATUS_BAR: (u16, u16) = (30, 14);
const TOASTS: (u16, u16) = (16, 4);
const DATE_ROWS: u16 = 36;

/// Whether a `cols × rows` terminal reaches the cut `(min_cols, min_rows)`.
pub fn reaches(cols: u16, rows: u16, (min_cols, min_rows): (u16, u16)) -> bool {
    cols >= min_cols && rows >= min_rows
}

/// Which tiers a terminal is in, for overlays (§4.3, §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SizeTier {
    Micro,
    Tiny,
    Small,
    Medium,
}

impl SizeTier {
    pub fn of(area: Rect) -> Self {
        let (c, r) = (area.width, area.height);
        if reaches(c, r, MEDIUM) {
            SizeTier::Medium
        } else if reaches(c, r, SMALL) {
            SizeTier::Small
        } else if reaches(c, r, TINY) {
            SizeTier::Tiny
        } else {
            SizeTier::Micro
        }
    }
}

/// The side margin (§1.3): the status bar and the side sheets keep this
/// many columns from the screen's left and right edges.
pub fn side_margin(cols: u16) -> u16 {
    match cols {
        0..120 => 2,
        120..200 => 4,
        _ => (f64::from(cols) * 0.03).round() as u16,
    }
}

/// Visual aspect (on-screen width ÷ height) of a `cols × rows` block.
pub fn visual_aspect(cols: u16, rows: u16, cell_aspect: f64) -> f64 {
    if cols == 0 || rows == 0 {
        return 1.0;
    }
    f64::from(cols) / (f64::from(rows) * cell_aspect)
}

pub fn layout(area: Rect, input: &LayoutInput) -> Layout {
    let mut out = Layout {
        area,
        ..Layout::default()
    };
    let (cols, rows) = (area.width, area.height);
    if cols < 4 || rows < 2 {
        return out;
    }
    out.status = status_row(area, input);
    let content = Rect {
        height: rows - u16::from(out.status.is_some()),
        ..area
    };

    let micro = !reaches(cols, rows, TINY);
    let with_panel = (!input.minimal && !micro)
        .then(|| with_panel(area, content, input))
        .flatten();
    let lamp = match with_panel {
        Some((lamp, panel)) => {
            out.panel = Some(panel);
            lamp
        }
        None => {
            out.chip = corner_chip(area, content, input);
            content
        }
    };
    out.lamp = Some(lamp);
    if reaches(cols, rows, TOASTS) {
        out.toast = Some(toast_row(area, lamp, out.panel.as_ref()));
    }
    out
}

/// The status bar's row, if it's on and fits (§4.1).
fn status_row(area: Rect, input: &LayoutInput) -> Option<Rect> {
    let (cols, rows) = (area.width, area.height);
    let on = !input.minimal && input.status_bar && reaches(cols, rows, STATUS_BAR);
    let inset = side_margin(cols);
    on.then(|| Rect::new(area.x + inset, area.bottom() - 1, cols - 2 * inset, 1))
}

/// The face forms a panel `inner_w` wide may use, largest first; `[None]`
/// when the clock is hidden. Seconds only in L/XL; XL only when huge.
fn panel_faces(input: &LayoutInput, inner_w: u16, huge: bool) -> Vec<Option<Form>> {
    if !input.show_clock {
        return vec![None];
    }
    let opts = FaceOptions {
        hour24: input.hour24,
        seconds: true,
    };
    input
        .face
        .all_forms(opts)
        .into_iter()
        .filter(|f| !f.seconds || f.tier >= Tier::L)
        .filter(|f| f.tier != Tier::XL || huge)
        .filter(|f| f.size.width <= inner_w)
        .map(Some)
        .collect()
}

/// Panel height for a face form and an optional date line (§1.4).
fn panel_height(form: Option<Form>, date: bool) -> u16 {
    match form {
        Some(f) => f.size.height + if date { 2 } else { 0 } + FACE_GAP + POMODORO_ROWS,
        None => POMODORO_ROWS,
    }
}

/// Lay the panel's parts out in a block at (`x`, `y`), `w` wide.
fn place_panel(x: u16, y: u16, w: u16, form: Option<Form>, date: bool) -> Panel {
    let h = panel_height(form, date);
    let (ix, iw) = (x + 1, w - 2);
    let face = form.map(|f| (Rect::new(ix, y, f.size.width, f.size.height), f));
    let date = match (form, date) {
        (Some(f), true) => Some(Rect::new(ix, y + f.size.height + 1, iw, 1)),
        _ => None,
    };
    Panel {
        rect: Rect::new(x, y, w, h),
        face,
        date,
        pomodoro: Rect::new(ix, y + h - POMODORO_ROWS, iw, POMODORO_ROWS),
    }
}

/// The panel's width before the face is chosen (§1.4).
fn panel_width(cols: u16) -> u16 {
    ((f64::from(cols) * 0.30).round() as u16).clamp(PANEL_W.0, PANEL_W.1)
}

/// The widest a panel `base` wide may grow for its face at `cols`: only
/// from Huge's column count up.
fn panel_max(cols: u16, base: u16) -> u16 {
    if cols >= HUGE.0 {
        base.max(PANEL_W_WIDE)
    } else {
        base
    }
}

/// `w`, widened (up to `max`) to hold `form` with its padding.
fn grow_for(w: u16, form: Option<Form>, max: u16) -> u16 {
    form.map_or(w, |f| w.max(f.size.width + 2).min(max))
}

/// Date line candidates, preferred first: it's the first thing to go.
fn date_options(area_rows: u16, show_clock: bool) -> &'static [bool] {
    if show_clock && area_rows >= DATE_ROWS {
        &[true, false]
    } else {
        &[false]
    }
}

fn is_huge(cols: u16, rows: u16) -> bool {
    reaches(cols, rows, HUGE)
}

/// The lamp with the panel beside it (wider than tall) or under it, or
/// `None` if the panel doesn't fit (hide order: date, face size).
fn with_panel(area: Rect, content: Rect, input: &LayoutInput) -> Option<(Rect, Panel)> {
    let (cols, rows) = (area.width, area.height);
    let huge = is_huge(cols, rows);
    let dates = date_options(rows, input.show_clock);
    if visual_aspect(content.width, content.height, input.cell_aspect) >= 1.0 {
        // Right panel if the lamp keeps ≥ 60 % of the width and ≥ 24 cols.
        let base_w = panel_width(cols);
        let max_w = panel_max(cols, base_w);
        for form in panel_faces(input, max_w - 2, huge) {
            let pw = grow_for(base_w, form, max_w);
            let Some(lamp_w) = content.width.checked_sub(pw) else {
                continue;
            };
            if lamp_w < 24 || u32::from(lamp_w) * 10 < u32::from(content.width) * 6 {
                continue;
            }
            for &date in dates {
                let ph = panel_height(form, date);
                if ph > content.height {
                    continue;
                }
                let lamp = Rect {
                    width: lamp_w,
                    ..content
                };
                let py = content.y + (content.height - ph) / 2;
                return Some((lamp, place_panel(content.x + lamp_w, py, pw, form, date)));
            }
        }
    } else {
        // Bottom panel if the lamp keeps ≥ 60 % of the rows and ≥ 10 rows.
        let base_w = PANEL_W.1.min(content.width);
        let max_w = panel_max(cols, base_w).min(content.width);
        if base_w < PANEL_W.0 {
            return None;
        }
        for form in panel_faces(input, max_w - 2, huge) {
            let bw = grow_for(base_w, form, max_w);
            for &date in dates {
                // One blank row between the lamp and the panel.
                let ph = panel_height(form, date) + 1;
                let Some(lamp_h) = content.height.checked_sub(ph) else {
                    continue;
                };
                if lamp_h < 10 || u32::from(lamp_h) * 10 < u32::from(content.height) * 6 {
                    continue;
                }
                let lamp = Rect {
                    height: lamp_h,
                    ..content
                };
                let px = content.x + (content.width - bw) / 2;
                return Some((
                    lamp,
                    place_panel(px, content.y + lamp_h + 1, bw, form, date),
                ));
            }
        }
    }
    None
}

/// The chip in the lamp's bottom-right corner (the screen's, in minimal
/// mode, where the lamp is the whole screen).
fn corner_chip(area: Rect, lamp: Rect, input: &LayoutInput) -> Option<Chip> {
    let (kind, text_w) = input.chip?;
    if !reaches(area.width, area.height, TINY) {
        return None;
    }
    if input.minimal && kind == ChipKind::Clock && input.minimal_clock == MinimalClock::Off {
        return None;
    }
    let w = (text_w + 2).min(area.width);
    Some(Chip {
        rect: Rect::new(lamp.right() - w, lamp.bottom() - 1, w, 1),
        kind,
    })
}

/// The toast row: the lamp's top row, spanning symmetrically around its
/// centre, clear of the panel.
fn toast_row(area: Rect, lamp: Rect, panel: Option<&Panel>) -> Rect {
    let y = lamp.y;
    let mut right = area.right();
    if let Some(p) = panel
        && p.rect.y <= y
        && y < p.rect.bottom()
        && p.rect.x > lamp.x
    {
        right = right.min(p.rect.x);
    }
    // In half columns from the screen's left edge.
    let centre2 = 2 * u32::from(lamp.x) + u32::from(lamp.width);
    let half = (centre2 / 2 - u32::from(area.x)).min(u32::from(right) - centre2.div_ceil(2));
    let x = (centre2 / 2) as u16 - half as u16;
    let w = (2 * half + centre2 % 2) as u16;
    Rect::new(x, y, w.min(right - x), 1)
}
