//! Responsive layout (docs/design.md §1): a pure function from the
//! terminal size and a few settings to non-overlapping placed elements.
//! No terminal access and no state, so it is exhaustively testable (see
//! `ui/tests.rs`).
//!
//! The rules, in brief:
//!
//! * The lamp is always placed (unless the screen is under 4×2). It has no
//!   frame: it fills whatever the status bar and the panel leave.
//! * The panel (the dock widgets placed `side`: clock + pomodoro by
//!   default) goes right of the lamp when the screen is wider than tall,
//!   else below it.
//! * Widgets placed `overlay` stack on the lava at the dock's anchor,
//!   never more than 60 % of the lamp's width or half its height (35 % of
//!   its area with their soft backing), clear of the toast row and the
//!   chip's corner.
//! * Things drop out whole in the §1.3 hide order: the date line, then
//!   face size (the last widget shrinks first), before the panel or the
//!   stack on the lava goes. A widget left without room collapses into a
//!   one-line chip over the lamp's corner.
//!
//! Odd leftover cells always go right/bottom so nothing jitters by a cell
//! between neighbouring sizes.

use ratatui::layout::{Alignment, Rect};

use crate::dock::{Anchor, Place, Room, WidgetForm, align_x};

/// One dock widget as the layout sees it (in [`crate::dock::WIDGETS`]
/// order): where it wants to be, the forms it offers there (most
/// preferred first) and, if it has one, its chip's text width and rank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockItem {
    pub place: Place,
    pub forms: Vec<WidgetForm>,
    pub chip: Option<(u16, u8)>,
}

/// Everything the layout depends on besides the terminal size.
#[derive(Clone, Copy)]
pub struct LayoutInput<'a> {
    pub minimal: bool,
    pub status_bar: bool,
    /// The dock's widgets, in registry order.
    pub dock: &'a [DockItem],
    /// Where the widgets on the lava gather.
    pub anchor: Anchor,
    /// Cell height ÷ width (§2.3).
    pub cell_aspect: f64,
}

/// A widget given a place: its index in [`crate::dock::WIDGETS`], its
/// rect and the form to draw there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub widget: usize,
    pub rect: Rect,
    pub form: WidgetForm,
}

/// Widgets stacked top to bottom: the side panel (`rect` includes 1 col
/// of padding each side) or the stack on the lava (`rect` is exactly the
/// widgets; their soft backing reaches [`HALO`] further).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    pub rect: Rect,
    pub items: Vec<Placed>,
    /// How narrower widgets line up in it.
    pub align: Alignment,
}

/// A one-line readout of a widget with no room where it was put, drawn
/// over the lamp's corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chip {
    pub rect: Rect,
    pub widget: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Layout {
    pub area: Rect,
    /// Where the wax is drawn ([`crate::render::LampView`]'s rect).
    pub lamp: Option<Rect>,
    pub status: Option<Rect>,
    pub panel: Option<Stack>,
    /// The widgets on the lava, inside the lamp.
    pub on_lava: Option<Stack>,
    pub chip: Option<Chip>,
    /// The row toasts are centred in.
    pub toast: Option<Rect>,
}

impl Layout {
    /// Where widget `index` is drawn, if anywhere but the chip.
    pub fn placed(&self, index: usize) -> Option<&Placed> {
        [&self.panel, &self.on_lava]
            .into_iter()
            .flatten()
            .flat_map(|s| &s.items)
            .find(|p| p.widget == index)
    }

    /// The cells the stack on the lava takes, its soft backing included.
    pub fn lava_footprint(&self) -> Option<Rect> {
        let s = self.on_lava.as_ref()?;
        Some(halo(s.rect))
    }
}

/// Panel width bounds (incl. 1-col padding each side).
const PANEL_W: (u16, u16) = (22, 36);
/// From Huge's column count up, the panel may grow to this so the widest
/// faces fit (blocks XL 51, blocks L with seconds 54); it only grows as
/// far as the widgets in it need.
const PANEL_W_WIDE: u16 = 56;
/// Rows between stacked widgets: in the panel, and on the lava.
const PANEL_GAP: u16 = 2;
const LAVA_GAP: u16 = 1;

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
/// The stack on the lava keeps at least this far from the lamp's edges,
/// `(cols, rows)`: past its soft backing ([`HALO`]), the toast row and
/// the chip's row.
const LAVA_INSET: (u16, u16) = (5, 3);
/// How far the soft backing behind widgets on the lava reaches past them.
pub const HALO: (u16, u16) = (4, 2);
/// The smallest lamp widgets go on: below it they'd crowd the wax, and the
/// corner chip says the same more quietly.
const LAVA_MIN: (u16, u16) = (28, 10);
/// The most of the lamp's area, in percent, the widgets on the lava and
/// their backing may cover.
const LAVA_MAX_AREA: u32 = 35;

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
    let room = Room {
        huge: is_huge(cols, rows),
        tall: rows >= DATE_ROWS,
    };
    let side = placed_at(input, Place::Side, room);
    let with_panel = (!input.minimal && !micro && !side.is_empty())
        .then(|| with_panel(area, content, input, &side))
        .flatten();
    let lamp = match with_panel {
        Some((lamp, panel)) => {
            out.panel = Some(panel);
            lamp
        }
        None => content,
    };
    out.lamp = Some(lamp);
    let lava = placed_at(input, Place::Overlay, room);
    if !micro && !lava.is_empty() {
        out.on_lava = on_lava(lamp, input.anchor, &lava);
    }
    out.chip = corner_chip(area, lamp, input, &out);
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

/// The widgets placed at `place`, with the forms this terminal has room
/// for (a widget left with none can't be placed there at all).
fn placed_at(input: &LayoutInput, place: Place, room: Room) -> Vec<(usize, Vec<WidgetForm>)> {
    input
        .dock
        .iter()
        .enumerate()
        .filter(|(_, d)| d.place == place)
        .map(|(i, d)| {
            let forms = d.forms.iter().copied().filter(|f| f.needs.met(room));
            (i, forms.collect())
        })
        .collect()
}

/// The first combination of one form per widget that `fits`, trying them
/// in the §1.3 hide order: the last widget's forms change fastest, so it
/// shrinks first and the first widget (the clock) keeps its size longest.
fn first_fit<T>(
    widgets: &[(usize, Vec<WidgetForm>)],
    mut fits: impl FnMut(&[WidgetForm]) -> Option<T>,
) -> Option<T> {
    if widgets.iter().any(|(_, forms)| forms.is_empty()) {
        return None;
    }
    let mut at = vec![0; widgets.len()];
    loop {
        let combo: Vec<WidgetForm> = at.iter().zip(widgets).map(|(&i, w)| w.1[i]).collect();
        if let Some(found) = fits(&combo) {
            return Some(found);
        }
        let mut k = widgets.len();
        loop {
            if k == 0 {
                return None;
            }
            k -= 1;
            at[k] += 1;
            if at[k] < widgets[k].1.len() {
                break;
            }
            at[k] = 0;
        }
    }
}

/// The widest of `combo`'s forms (a fill form's least width counts).
fn widest(combo: &[WidgetForm]) -> u16 {
    combo.iter().map(|f| f.size.width).max().unwrap_or(0)
}

/// `combo` stacked with `gap` rows between widgets.
fn stack_height(combo: &[WidgetForm], gap: u16) -> u16 {
    let rows: u16 = combo.iter().map(|f| f.size.height).sum();
    rows + gap * (combo.len() as u16).saturating_sub(1)
}

/// Stack `combo` (forms of `widgets`, in order) from (`x`, `y`) in a
/// column `w` wide: fill forms take the width, the rest line up by
/// `align`.
fn stack(
    widgets: &[(usize, Vec<WidgetForm>)],
    combo: &[WidgetForm],
    (x, y, w): (u16, u16, u16),
    gap: u16,
    align: Alignment,
) -> Vec<Placed> {
    let mut y = y;
    let mut out = Vec::with_capacity(combo.len());
    for (&(widget, _), &form) in widgets.iter().zip(combo) {
        let fw = if form.fill { w } else { form.size.width };
        let rect = Rect::new(x + align_x(align, w, fw), y, fw, form.size.height);
        out.push(Placed { widget, rect, form });
        y += form.size.height + gap;
    }
    out
}

/// The panel's width before its widgets are chosen (§1.4).
fn panel_width(cols: u16) -> u16 {
    ((f64::from(cols) * 0.30).round() as u16).clamp(PANEL_W.0, PANEL_W.1)
}

/// The widest a panel `base` wide may grow for its widgets at `cols`:
/// only from Huge's column count up.
fn panel_max(cols: u16, base: u16) -> u16 {
    if cols >= HUGE.0 {
        base.max(PANEL_W_WIDE)
    } else {
        base
    }
}

fn is_huge(cols: u16, rows: u16) -> bool {
    reaches(cols, rows, HUGE)
}

/// The panel at (`x`, `y`), `w` wide, holding `combo`.
fn place_panel(
    widgets: &[(usize, Vec<WidgetForm>)],
    combo: &[WidgetForm],
    (x, y, w): (u16, u16, u16),
) -> Stack {
    let h = stack_height(combo, PANEL_GAP);
    Stack {
        rect: Rect::new(x, y, w, h),
        items: stack(
            widgets,
            combo,
            (x + 1, y, w - 2),
            PANEL_GAP,
            Alignment::Left,
        ),
        align: Alignment::Left,
    }
}

/// The lamp with the panel of `side` widgets beside it (wider than tall)
/// or under it, or `None` if no combination of their forms fits (hide
/// order: date, face size).
fn with_panel(
    area: Rect,
    content: Rect,
    input: &LayoutInput,
    side: &[(usize, Vec<WidgetForm>)],
) -> Option<(Rect, Stack)> {
    let cols = area.width;
    if visual_aspect(content.width, content.height, input.cell_aspect) >= 1.0 {
        // Right panel if the lamp keeps ≥ 60 % of the width and ≥ 24 cols.
        let base_w = panel_width(cols);
        let max_w = panel_max(cols, base_w);
        first_fit(side, |combo| {
            let need = widest(combo);
            if need > max_w - 2 {
                return None;
            }
            let pw = base_w.max(need + 2).min(max_w);
            let lamp_w = content.width.checked_sub(pw)?;
            if lamp_w < 24 || u32::from(lamp_w) * 10 < u32::from(content.width) * 6 {
                return None;
            }
            let ph = stack_height(combo, PANEL_GAP);
            if ph > content.height {
                return None;
            }
            let lamp = Rect {
                width: lamp_w,
                ..content
            };
            let py = content.y + (content.height - ph) / 2;
            let at = (content.x + lamp_w, py, pw);
            Some((lamp, place_panel(side, combo, at)))
        })
    } else {
        // Bottom panel if the lamp keeps ≥ 60 % of the rows and ≥ 10 rows.
        let base_w = PANEL_W.1.min(content.width);
        let max_w = panel_max(cols, base_w).min(content.width);
        if base_w < PANEL_W.0 {
            return None;
        }
        first_fit(side, |combo| {
            let need = widest(combo);
            if need > max_w - 2 {
                return None;
            }
            let bw = base_w.max(need + 2).min(max_w);
            // One blank row between the lamp and the panel.
            let ph = stack_height(combo, PANEL_GAP) + 1;
            let lamp_h = content.height.checked_sub(ph)?;
            if lamp_h < 10 || u32::from(lamp_h) * 10 < u32::from(content.height) * 6 {
                return None;
            }
            let lamp = Rect {
                height: lamp_h,
                ..content
            };
            let px = content.x + (content.width - bw) / 2;
            let at = (px, content.y + lamp_h + 1, bw);
            Some((lamp, place_panel(side, combo, at)))
        })
    }
}

/// The widgets on the lava, stacked at `anchor` inside `lamp`: at most
/// 60 % of its width and half its height, with their backing at most 35 %
/// of its area, inset past their backing so it
/// never reaches the toast row or the chip's row. `None` when not even
/// their smallest forms fit: they go to the chip instead.
fn on_lava(lamp: Rect, anchor: Anchor, widgets: &[(usize, Vec<WidgetForm>)]) -> Option<Stack> {
    if !reaches(lamp.width, lamp.height, LAVA_MIN) {
        return None;
    }
    let mx = (lamp.width / 20).max(LAVA_INSET.0);
    let my = (lamp.height / 12).max(LAVA_INSET.1);
    let free = Rect::new(
        lamp.x + mx,
        lamp.y + my,
        lamp.width.checked_sub(2 * mx)?,
        lamp.height.checked_sub(2 * my)?,
    );
    let max_w = free.width.min(lamp.width * 3 / 5);
    let max_h = free.height.min(lamp.height / 2);
    let align = anchor.align();
    let (gx, gy) = anchor.grid();
    let at = |start: u16, avail: u16, len: u16, g: u8| {
        start
            + match g {
                0 => 0,
                1 => (avail - len) / 2,
                _ => avail - len,
            }
    };
    first_fit(widgets, |combo| {
        let (w, h) = (widest(combo), stack_height(combo, LAVA_GAP));
        if w == 0 || w > max_w || h > max_h {
            return None;
        }
        // With its backing, at most a third or so of the lamp.
        let back = halo(Rect::new(free.x, free.y, w, h));
        let area = |r: Rect| u32::from(r.width) * u32::from(r.height);
        if area(back) * 100 > area(lamp) * LAVA_MAX_AREA {
            return None;
        }
        let x = at(free.x, free.width, w, gx);
        let y = at(free.y, free.height, h, gy);
        Some(Stack {
            rect: Rect::new(x, y, w, h),
            items: stack(widgets, combo, (x, y, w), LAVA_GAP, align),
            align,
        })
    })
}

/// `r` grown by [`HALO`]: what the backing behind the lava stack covers.
pub fn halo(r: Rect) -> Rect {
    let (hx, hy) = HALO;
    let x = r.x.saturating_sub(hx);
    let y = r.y.saturating_sub(hy);
    Rect::new(x, y, r.right() + hx - x, r.bottom() + hy - y)
}

/// The chip in the lamp's bottom-right corner, for the widget with the
/// highest-ranked chip among those whose place has no room: side widgets
/// without a panel (always, in minimal mode), widgets on the lava when
/// their stack doesn't fit.
fn corner_chip(area: Rect, lamp: Rect, input: &LayoutInput, out: &Layout) -> Option<Chip> {
    if !reaches(area.width, area.height, TINY) {
        return None;
    }
    let homeless = |place: Place| match place {
        Place::Side => out.panel.is_none(),
        Place::Overlay => out.on_lava.is_none(),
        Place::Off => false,
    };
    let mut best: Option<(usize, u16, u8)> = None;
    for (i, d) in input.dock.iter().enumerate() {
        if let Some((w, rank)) = d.chip.filter(|_| homeless(d.place))
            && best.is_none_or(|b| rank > b.2)
        {
            best = Some((i, w, rank));
        }
    }
    let (widget, text_w, _) = best?;
    let w = (text_w + 2).min(area.width);
    Some(Chip {
        rect: Rect::new(lamp.right() - w, lamp.bottom() - 1, w, 1),
        widget,
    })
}

/// The toast row: the lamp's top row, spanning symmetrically around its
/// centre, clear of the panel.
fn toast_row(area: Rect, lamp: Rect, panel: Option<&Stack>) -> Rect {
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
