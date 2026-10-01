//! Responsive layout (docs/design.md §1): a pure function from the
//! terminal size and a few settings to non-overlapping placed elements.
//! No terminal access and no state, so it is exhaustively testable (see
//! `ui/tests.rs`).
//!
//! The rules, in brief:
//!
//! * The lamp is always placed (unless the screen is under 4×2). It has no
//!   frame: it fills whatever the status bar and the panel leave.
//! * The panel holds the dock widgets placed `side`. It goes right of the
//!   lamp when the screen is wider than tall, else below it; in a
//!   wide-short screen it may instead be a strip under the lamp (widgets
//!   side by side), in a portrait one a wrap of rows under it, and in a
//!   very large one two columns: whichever gives the widgets, by rank,
//!   their largest forms (ties: the plain column, so nothing changes where
//!   it already fits).
//! * Widgets placed `overlay` go on the lava at their own anchor: those
//!   sharing one stack there, different anchors spread across the lamp
//!   without touching. Each stack ≤ 60 % of the lamp's width and half its
//!   height; all their soft backings ≤ 35 % of its area, clear of the
//!   toast row and the chip row.
//! * Things shrink and drop out whole by rank (§1.3 and §4.6): the
//!   lowest-ranked widget shrinks first (date line, then face size), and
//!   when even the smallest forms don't fit it leaves for the chip row: one
//!   line of mini-chips over the lamp's bottom-right corner (`14:32 ·
//!   ▸ 18:24`), the lowest-ranked dropped first when they don't all fit.
//!
//! Odd leftover cells always go right/bottom so nothing jitters by a cell
//! between neighbouring sizes.

use ratatui::layout::{Alignment, Rect};

use crate::dock::{Anchor, Place, Room, WidgetForm, align_x};

/// One dock widget as the layout sees it (in [`crate::dock::WIDGETS`]
/// order): where it wants to be, the forms it offers there (most
/// preferred first), its chip's text width (if it has one) and its rank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockItem {
    pub place: Place,
    /// Where it sits when on the lava.
    pub anchor: Anchor,
    pub forms: Vec<WidgetForm>,
    pub chip: Option<u16>,
    /// [`DockWidget::rank`](crate::dock::DockWidget::rank): higher keeps
    /// its size longer and leaves for the chip row later.
    pub rank: u8,
}

/// Everything the layout depends on besides the terminal size.
#[derive(Clone, Copy)]
pub struct LayoutInput<'a> {
    pub minimal: bool,
    pub status_bar: bool,
    /// The dock's widgets, in registry order.
    pub dock: &'a [DockItem],
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

/// Placed widgets: the side panel (`rect` includes 1 col of padding each
/// side, however the widgets are arranged in it) or one stack on the lava
/// (`rect` is exactly the widgets; their soft backing reaches [`HALO`]
/// further).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    pub rect: Rect,
    pub items: Vec<Placed>,
    /// How narrower widgets line up in it.
    pub align: Alignment,
}

/// The one-line row of mini-chips for widgets with no room where they
/// were put, over the lamp's bottom-right corner: ` 14:32 · ▸ 18:24 `.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChipRow {
    /// The whole row, its 1-cell pads included.
    pub rect: Rect,
    /// In registry order, ` · ` between them.
    pub items: Vec<Chip>,
}

/// One chip's text in the row.
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
    /// The widgets on the lava: one stack per anchor in use, inside the
    /// lamp, their backings apart.
    pub on_lava: Vec<Stack>,
    pub chips: Option<ChipRow>,
    /// The row toasts are centred in.
    pub toast: Option<Rect>,
}

impl Layout {
    /// Where widget `index` is drawn, if anywhere but the chip row.
    pub fn placed(&self, index: usize) -> Option<&Placed> {
        self.panel
            .iter()
            .chain(&self.on_lava)
            .flat_map(|s| &s.items)
            .find(|p| p.widget == index)
    }

    /// The cells the stacks on the lava take, their soft backings
    /// included (one rect around all of them).
    pub fn lava_footprint(&self) -> Option<Rect> {
        self.on_lava
            .iter()
            .map(|s| halo(s.rect))
            .reduce(|a, b| a.union(b))
    }

    /// Whether widget `index` is in the chip row.
    #[cfg(test)]
    pub fn chipped(&self, index: usize) -> bool {
        self.chips
            .as_ref()
            .is_some_and(|c| c.items.iter().any(|i| i.widget == index))
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
    let mut homeless = Vec::new();
    let side = placed_at(input, Place::Side, room, &mut homeless);
    let lamp = if input.minimal || micro || side.is_empty() {
        homeless.extend(side.iter().map(|w| w.index));
        content
    } else {
        match arrange_side(area, content, input, &side) {
            Some(fit) => {
                homeless.extend(fit.dropped);
                let (lamp, panel) = fit.found;
                out.panel = Some(panel);
                lamp
            }
            None => {
                homeless.extend(side.iter().map(|w| w.index));
                content
            }
        }
    };
    out.lamp = Some(lamp);
    let lava = placed_at(input, Place::Overlay, room, &mut homeless);
    if micro {
        homeless.extend(lava.iter().map(|w| w.index));
    } else if !lava.is_empty() {
        match on_lava(lamp, &lava) {
            Some(fit) => {
                homeless.extend(fit.dropped);
                out.on_lava = fit.found;
            }
            None => homeless.extend(lava.iter().map(|w| w.index)),
        }
    }
    out.chips = chip_row(area, lamp, input, &homeless);
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

/// A widget being placed: its index, the forms this terminal has room for,
/// its anchor and rank.
#[derive(Debug, Clone)]
struct W {
    index: usize,
    forms: Vec<WidgetForm>,
    anchor: Anchor,
}

/// The widgets placed at `place` that have a form this terminal has room
/// for, most important first (rank, then registry order). Those with none
/// go to `homeless`.
fn placed_at(input: &LayoutInput, place: Place, room: Room, homeless: &mut Vec<usize>) -> Vec<W> {
    let mut out: Vec<(u8, W)> = Vec::new();
    for (index, d) in input.dock.iter().enumerate() {
        if d.place != place {
            continue;
        }
        let forms: Vec<WidgetForm> = d
            .forms
            .iter()
            .copied()
            .filter(|f| f.needs.met(room))
            .collect();
        if forms.is_empty() {
            homeless.push(index);
        } else {
            let w = W {
                index,
                forms,
                anchor: d.anchor,
            };
            out.push((d.rank, w));
        }
    }
    // Stable: equal ranks keep registry order.
    out.sort_by_key(|w| std::cmp::Reverse(w.0));
    out.into_iter().map(|(_, w)| w).collect()
}

/// One form per widget, as `(widget, form)` in registry order.
type Combo = Vec<(usize, WidgetForm)>;

/// What fitting found: the arrangement, how good it is and who had to go.
#[derive(Debug)]
struct Fit<T> {
    found: T,
    /// Per widget, most important first: the index of its form (lower is
    /// larger), `usize::MAX` if it was dropped.
    quality: Vec<usize>,
    dropped: Vec<usize>,
}

impl<T> Fit<T> {
    /// Lower is better: fewer widgets dropped, then larger forms for the
    /// more important ones.
    fn score(&self) -> (usize, &[usize]) {
        (self.dropped.len(), &self.quality)
    }
}

/// The first combination of one form per widget (`ws` most important
/// first) that `fits`, trying them in the §1.3 hide order: the least
/// important widget's forms change fastest, so it shrinks first and the
/// most important keeps its size longest. When none fits, the least
/// important widget is dropped and the rest tried again.
fn fit_dropping<T>(ws: &[W], mut fits: impl FnMut(&Combo) -> Option<T>) -> Option<Fit<T>> {
    for n in (1..=ws.len()).rev() {
        let kept = &ws[..n];
        let mut at = vec![0; n];
        'combos: loop {
            let mut combo: Combo = at
                .iter()
                .zip(kept)
                .map(|(&i, w)| (w.index, w.forms[i]))
                .collect();
            combo.sort_by_key(|c| c.0);
            if let Some(found) = fits(&combo) {
                let mut quality = at;
                quality.resize(ws.len(), usize::MAX);
                return Some(Fit {
                    found,
                    quality,
                    dropped: ws[n..].iter().map(|w| w.index).collect(),
                });
            }
            // The odometer: the last (least important) digit turns fastest.
            let mut k = n;
            loop {
                if k == 0 {
                    break 'combos;
                }
                k -= 1;
                at[k] += 1;
                if at[k] < kept[k].forms.len() {
                    break;
                }
                at[k] = 0;
            }
        }
    }
    None
}

/// The widest of `combo`'s forms (a fill form's least width counts).
fn widest(combo: &[(usize, WidgetForm)]) -> u16 {
    combo.iter().map(|f| f.1.size.width).max().unwrap_or(0)
}

/// `combo` stacked with `gap` rows between widgets.
fn stack_height(combo: &[(usize, WidgetForm)], gap: u16) -> u16 {
    let rows: u16 = combo.iter().map(|f| f.1.size.height).sum();
    rows + gap * (combo.len() as u16).saturating_sub(1)
}

/// Stack `combo` from (`x`, `y`) in a column `w` wide: fill forms take
/// the width, the rest line up by `align`.
fn stack(
    combo: &[(usize, WidgetForm)],
    (x, y, w): (u16, u16, u16),
    gap: u16,
    align: Alignment,
) -> Vec<Placed> {
    let mut y = y;
    let mut out = Vec::with_capacity(combo.len());
    for &(widget, form) in combo {
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

/// The panel at (`x`, `y`), `w` wide, holding `combo` in one column.
fn place_panel(combo: &[(usize, WidgetForm)], (x, y, w): (u16, u16, u16)) -> Stack {
    let h = stack_height(combo, PANEL_GAP);
    Stack {
        rect: Rect::new(x, y, w, h),
        items: stack(combo, (x + 1, y, w - 2), PANEL_GAP, Alignment::Left),
        align: Alignment::Left,
    }
}

/// Whether a lamp `lamp` long (cols or rows) out of `total` is still the
/// hero: at least `min` and 60 %.
fn lamp_keeps(lamp: u16, total: u16, min: u16) -> bool {
    lamp >= min && u32::from(lamp) * 10 >= u32::from(total) * 6
}

/// A screen this much wider than tall (visually) may put the panel in a
/// strip under the lamp.
const STRIP_ASPECT: f64 = 3.0;
/// Columns between widgets side by side (strip, wrap, two columns).
const FLOW_GAP: u16 = 3;

/// The side widgets laid out, by whichever arrangement suits them best
/// (see the module docs), or `None` if not even one of them fits.
fn arrange_side(
    area: Rect,
    content: Rect,
    input: &LayoutInput,
    side: &[W],
) -> Option<Fit<(Rect, Stack)>> {
    let cols = area.width;
    let aspect = visual_aspect(content.width, content.height, input.cell_aspect);
    let mut best: Option<Fit<(Rect, Stack)>> = None;
    let mut offer = |fit: Option<Fit<(Rect, Stack)>>, wins_ties: bool| {
        let Some(fit) = fit else { return };
        let better = best.as_ref().is_none_or(|b| {
            let (new, old) = (fit.score(), b.score());
            new < old || (new == old && wins_ties)
        });
        if better {
            best = Some(fit);
        }
    };
    if aspect >= 1.0 {
        let column = fit_dropping(side, |c| right_column(cols, content, c));
        let tall = column
            .as_ref()
            .is_some_and(|f| f.found.1.rect.height * 2 > content.height);
        offer(column, true);
        if cols >= HUGE.0 {
            offer(fit_dropping(side, |c| two_columns(content, c)), tall);
        }
        if aspect >= STRIP_ASPECT {
            offer(
                fit_dropping(side, |c| under(content, c, Under::Strip)),
                false,
            );
        }
    } else {
        offer(
            fit_dropping(side, |c| bottom_column(cols, content, c)),
            true,
        );
        offer(
            fit_dropping(side, |c| under(content, c, Under::Wrap)),
            false,
        );
    }
    best
}

/// One column right of the lamp, if the lamp keeps ≥ 60 % of the width
/// and ≥ 24 cols, vertically centred.
fn right_column(cols: u16, content: Rect, combo: &Combo) -> Option<(Rect, Stack)> {
    let base_w = panel_width(cols);
    let max_w = panel_max(cols, base_w);
    let need = widest(combo);
    if need > max_w - 2 {
        return None;
    }
    let pw = base_w.max(need + 2).min(max_w);
    let lamp_w = content.width.checked_sub(pw)?;
    if !lamp_keeps(lamp_w, content.width, 24) {
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
    Some((lamp, place_panel(combo, (content.x + lamp_w, py, pw))))
}

/// One column under the lamp (portrait), centred, if the lamp keeps
/// ≥ 60 % of the rows and ≥ 10 rows.
fn bottom_column(cols: u16, content: Rect, combo: &Combo) -> Option<(Rect, Stack)> {
    let base_w = PANEL_W.1.min(content.width);
    let max_w = panel_max(cols, base_w).min(content.width);
    if base_w < PANEL_W.0 {
        return None;
    }
    let need = widest(combo);
    if need > max_w - 2 {
        return None;
    }
    let bw = base_w.max(need + 2).min(max_w);
    // One blank row between the lamp and the panel.
    let ph = stack_height(combo, PANEL_GAP) + 1;
    let lamp_h = content.height.checked_sub(ph)?;
    if !lamp_keeps(lamp_h, content.height, 10) {
        return None;
    }
    let lamp = Rect {
        height: lamp_h,
        ..content
    };
    let px = content.x + (content.width - bw) / 2;
    Some((lamp, place_panel(combo, (px, content.y + lamp_h + 1, bw))))
}

/// Two columns right of the lamp (very large screens): the widgets in
/// registry order, split where the taller column is shortest.
fn two_columns(content: Rect, combo: &Combo) -> Option<(Rect, Stack)> {
    if combo.len() < 2 {
        return None;
    }
    let split = (1..combo.len()).min_by_key(|&k| {
        stack_height(&combo[..k], PANEL_GAP).max(stack_height(&combo[k..], PANEL_GAP))
    })?;
    let (a, b) = combo.split_at(split);
    let (wa, wb) = (widest(a), widest(b));
    let pw = wa + FLOW_GAP + wb + 2;
    let lamp_w = content.width.checked_sub(pw)?;
    if !lamp_keeps(lamp_w, content.width, 24) {
        return None;
    }
    let ph = stack_height(a, PANEL_GAP).max(stack_height(b, PANEL_GAP));
    if ph > content.height {
        return None;
    }
    let (x, y) = (content.x + lamp_w, content.y + (content.height - ph) / 2);
    let mut items = stack(a, (x + 1, y, wa), PANEL_GAP, Alignment::Left);
    items.extend(stack(
        b,
        (x + 1 + wa + FLOW_GAP, y, wb),
        PANEL_GAP,
        Alignment::Left,
    ));
    let lamp = Rect {
        width: lamp_w,
        ..content
    };
    Some((
        lamp,
        Stack {
            rect: Rect::new(x, y, pw, ph),
            items,
            align: Alignment::Left,
        },
    ))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Under {
    /// Wide and short: the widgets side by side in a strip, centred,
    /// rows 1 apart.
    Strip,
    /// Portrait: rows across the whole width, rows 2 apart like the panel.
    Wrap,
}

/// The widgets in rows under the lamp, left to right in registry order,
/// a new row when the next doesn't fit, items centred in their row's
/// height. A fill form alone in its row takes the row (up to the panel's
/// usual width); beside others it takes its least width.
fn under(content: Rect, combo: &Combo, how: Under) -> Option<(Rect, Stack)> {
    let inner = content.width.checked_sub(2)?;
    let mut rows: Vec<Vec<(usize, WidgetForm)>> = Vec::new();
    let mut row_w = 0;
    for &(widget, form) in combo {
        let w = form.size.width;
        if w > inner {
            return None;
        }
        match rows.last_mut() {
            Some(row) if row_w + FLOW_GAP + w <= inner => {
                row.push((widget, form));
                row_w += FLOW_GAP + w;
            }
            _ => {
                rows.push(vec![(widget, form)]);
                row_w = w;
            }
        }
    }
    let row_gap = match how {
        Under::Strip => 1,
        Under::Wrap => PANEL_GAP,
    };
    let width_of = |row: &[(usize, WidgetForm)]| -> u16 {
        match row {
            [(_, f)] if f.fill => inner.min(PANEL_W.1 - 2).max(f.size.width),
            _ => {
                row.iter().map(|r| r.1.size.width).sum::<u16>() + FLOW_GAP * (row.len() as u16 - 1)
            }
        }
    };
    let height_of =
        |row: &[(usize, WidgetForm)]| row.iter().map(|r| r.1.size.height).max().unwrap_or(0);
    let ph: u16 =
        rows.iter().map(|r| height_of(r)).sum::<u16>() + row_gap * (rows.len() as u16 - 1);
    // One blank row between the lamp and the widgets.
    let lamp_h = content.height.checked_sub(ph + 1)?;
    if !lamp_keeps(lamp_h, content.height, 10) {
        return None;
    }
    let pw = match how {
        Under::Strip => rows.iter().map(|r| width_of(r)).max().unwrap_or(0) + 2,
        Under::Wrap => content.width,
    };
    let px = content.x + (content.width - pw) / 2;
    let mut y = content.y + lamp_h + 1;
    let mut items = Vec::with_capacity(combo.len());
    for row in &rows {
        let h = height_of(row);
        let mut x = px + 1;
        for &(widget, form) in row {
            let w = if row.len() == 1 {
                width_of(row)
            } else {
                form.size.width
            };
            let w = if form.fill { w } else { form.size.width };
            let dy = (h - form.size.height) / 2;
            items.push(Placed {
                widget,
                rect: Rect::new(x, y + dy, w, form.size.height),
                form,
            });
            x += w + FLOW_GAP;
        }
        y += h + row_gap;
    }
    let lamp = Rect {
        height: lamp_h,
        ..content
    };
    Some((
        lamp,
        Stack {
            rect: Rect::new(px, content.y + lamp_h + 1, pw, ph),
            items,
            align: Alignment::Left,
        },
    ))
}

/// The widgets on the lava: a stack per anchor, each at most 60 % of the
/// lamp's width and half its height, backings apart and together at most
/// 35 % of its area, inset past their backing so it never reaches the
/// toast row or the chip row. `None` when not even the most important
/// one's smallest form fits.
fn on_lava(lamp: Rect, ws: &[W]) -> Option<Fit<Vec<Stack>>> {
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
    let at = |start: u16, avail: u16, len: u16, g: u8| {
        start
            + match g {
                0 => 0,
                1 => (avail - len) / 2,
                _ => avail - len,
            }
    };
    let area = |r: Rect| u32::from(r.width) * u32::from(r.height);
    let anchor_of = |widget: usize| ws.iter().find(|w| w.index == widget).map(|w| w.anchor);
    fit_dropping(ws, |combo| {
        let mut stacks: Vec<Stack> = Vec::new();
        let mut covered = 0;
        for anchor in Anchor::ALL {
            let group: Combo = combo
                .iter()
                .copied()
                .filter(|c| anchor_of(c.0) == Some(anchor))
                .collect();
            if group.is_empty() {
                continue;
            }
            let (w, h) = (widest(&group), stack_height(&group, LAVA_GAP));
            if w == 0 || w > max_w || h > max_h {
                return None;
            }
            let (gx, gy) = anchor.grid();
            let (x, y) = (
                at(free.x, free.width, w, gx),
                at(free.y, free.height, h, gy),
            );
            let rect = Rect::new(x, y, w, h);
            let back = halo(rect);
            if stacks.iter().any(|s| halo(s.rect).intersects(back)) {
                return None;
            }
            covered += area(back);
            let align = anchor.align();
            stacks.push(Stack {
                rect,
                items: stack(&group, (x, y, w), LAVA_GAP, align),
                align,
            });
        }
        (covered * 100 <= area(lamp) * LAVA_MAX_AREA).then_some(stacks)
    })
}

/// `r` grown by [`HALO`]: what the backing behind a lava stack covers.
pub fn halo(r: Rect) -> Rect {
    let (hx, hy) = HALO;
    let x = r.x.saturating_sub(hx);
    let y = r.y.saturating_sub(hy);
    Rect::new(x, y, r.right() + hx - x, r.bottom() + hy - y)
}

/// Columns between chips in the row: ` · `.
pub const CHIP_SEP: u16 = 3;

/// The chip row over the lamp's bottom-right corner: the chips of the
/// `homeless` widgets (no room where they were put: side widgets without
/// a panel, always in minimal mode; widgets dropped from the panel or the
/// lava) in registry order, the lowest-ranked left out while they don't
/// fit the lamp's width (and any too wide for it on its own).
fn chip_row(area: Rect, lamp: Rect, input: &LayoutInput, homeless: &[usize]) -> Option<ChipRow> {
    if !reaches(area.width, area.height, TINY) {
        return None;
    }
    // A chip too wide for the lamp on its own never shows.
    let mut chips: Vec<(usize, u16, u8)> = homeless
        .iter()
        .filter_map(|&i| Some((i, input.dock[i].chip?, input.dock[i].rank)))
        .filter(|c| c.1 + 2 <= lamp.width)
        .collect();
    chips.sort_by_key(|c| c.0);
    let width = |chips: &[(usize, u16, u8)]| {
        chips.iter().map(|c| c.1).sum::<u16>()
            + CHIP_SEP * (chips.len() as u16).saturating_sub(1)
            + 2
    };
    while !chips.is_empty() && width(&chips) > lamp.width {
        // The least important: lowest rank, later in the registry.
        let (at, _) = chips
            .iter()
            .enumerate()
            .min_by_key(|(_, c)| (c.2, std::cmp::Reverse(c.0)))?;
        chips.remove(at);
    }
    if chips.is_empty() {
        return None;
    }
    let w = width(&chips);
    let rect = Rect::new(lamp.right() - w, lamp.bottom() - 1, w, 1);
    let mut x = rect.x + 1;
    let items = chips
        .iter()
        .map(|&(widget, cw, _)| {
            let chip = Chip {
                rect: Rect::new(x, rect.y, cw, 1),
                widget,
            };
            x += cw + CHIP_SEP;
            chip
        })
        .collect();
    Some(ChipRow { rect, items })
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
