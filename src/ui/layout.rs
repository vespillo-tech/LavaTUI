//! Responsive layout (docs/design.md §1): a pure function from the
//! terminal size and a few settings to non-overlapping placed elements.
//! No terminal access and no history beyond the last frame's glass/bleed
//! choice (passed in), so it is exhaustively testable (see `ui/tests.rs`).
//!
//! The rules, in brief:
//!
//! * The lamp is always placed (unless the screen is under 4×2).
//! * Frame: `glass` (a lamp silhouette) when there are ≥ 20 content rows
//!   and the content isn't wider than 2.2:1, else `bleed` (edge to edge).
//!   Coming from bleed, glass needs ≥ 22 rows and ≤ 2.0:1, so a window
//!   edge dragged across the line doesn't flap between the two.
//! * The panel (clock + pomodoro) goes right of the lamp, else below it,
//!   else it collapses into a one-line chip over the lamp's corner.
//! * Things drop out whole in the §1.3 hide order: date line, then outer
//!   margins, then face size, before the panel goes.
//!
//! Odd leftover cells always go right/bottom so nothing jitters by a cell
//! between neighbouring sizes.

use ratatui::layout::Rect;

use crate::clock::{Face, FaceOptions, Form, Tier};
use crate::config::{FrameMode, MinimalClock};
use crate::silhouette;

/// Everything the layout depends on besides the terminal size.
#[derive(Clone, Copy)]
pub struct LayoutInput<'a> {
    pub minimal: bool,
    pub status_bar: bool,
    pub frame: FrameMode,
    pub show_clock: bool,
    pub face: &'a dyn Face,
    pub hour24: bool,
    /// What a chip would show, and its text width (without padding).
    pub chip: Option<(ChipKind, u16)>,
    pub minimal_clock: MinimalClock,
    /// Cell height ÷ width (§2.3).
    pub cell_aspect: f64,
    /// The frame the lamp had last time, for auto's hysteresis.
    pub prev_frame: Option<LampFrame>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LampFrame {
    Glass,
    Bleed,
}

/// Glass silhouette rows (§2.1): cap, bottle, base, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glass {
    pub cap: u16,
    pub bottle: u16,
    pub base: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lamp {
    /// Everything that belongs to the lamp (glass: the whole silhouette's
    /// bounding box; bleed: the tank).
    pub region: Rect,
    /// Where the wax is drawn ([`crate::render::LampView`]'s rect): the
    /// bottle's bounding box in glass, the region itself in bleed.
    pub view: Rect,
    pub frame: LampFrame,
    pub glass: Option<Glass>,
}

impl Lamp {
    /// Centre column, in half-columns from the left edge of the screen.
    pub fn centre2(&self) -> u32 {
        2 * u32::from(self.region.x) + u32::from(self.region.width)
    }
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

/// A one-line clock or pomodoro readout, drawn over the lamp's corner (or
/// centred under the lamp in minimal mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chip {
    pub rect: Rect,
    pub kind: ChipKind,
    /// Minimal mode's "under" clock: plain dim text, no padding fill.
    pub under: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Layout {
    pub area: Rect,
    pub lamp: Option<Lamp>,
    pub status: Option<Rect>,
    pub panel: Option<Panel>,
    pub chip: Option<Chip>,
    /// The row toasts are centred in.
    pub toast: Option<Rect>,
}

/// Auto picks glass with at least this many content rows and at most this
/// visual aspect (§2.1); coming from bleed it needs the stricter pair.
const AUTO_GLASS: (u16, f64) = (20, 2.2);
const AUTO_GLASS_ENTER: (u16, f64) = (22, 2.0);
/// Smallest glass lamp. Auto only picks glass with ≥ 20 content rows; a
/// forced glass below this becomes bleed (hide order 6).
const MIN_GLASS_ROWS: u16 = 12;
/// A panel beside or under a glass lamp needs the lamp at least this tall.
const PANEL_GLASS_ROWS: u16 = 20;
/// Panel width bounds (incl. 1-col padding each side).
const PANEL_W: (u16, u16) = (22, 36);
/// Rows of the pomodoro block, and the gap above it.
const POMODORO_ROWS: u16 = 3;
const FACE_GAP: u16 = 2;

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
        if c < 20 || r < 8 {
            SizeTier::Micro
        } else if c < 40 || r < 14 {
            SizeTier::Tiny
        } else if c < 80 || r < 24 {
            SizeTier::Small
        } else {
            SizeTier::Medium
        }
    }
}

/// Outer margins `(vertical, horizontal)` for glass mode (§1.3).
pub fn margins(cols: u16, rows: u16) -> (u16, u16) {
    let v = match rows {
        0..24 => 0,
        24..36 => 1,
        36..56 => 2,
        _ => (f64::from(rows) * 0.04).round() as u16,
    };
    let h = match cols {
        0..120 => 2,
        120..200 => 4,
        _ => (f64::from(cols) * 0.03).round() as u16,
    };
    (v, h)
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
        lamp: None,
        status: None,
        panel: None,
        chip: None,
        toast: None,
    };
    let (cols, rows) = (area.width, area.height);
    if cols < 4 || rows < 2 {
        return out;
    }
    let (vm, hm) = margins(cols, rows);

    let status_on = !input.minimal && input.status_bar && rows >= 14 && cols >= 30;
    if status_on {
        let inset = hm.max(1);
        out.status = Some(Rect::new(
            area.x + inset,
            area.bottom() - 1,
            cols - 2 * inset,
            1,
        ));
    }
    let content = Rect {
        height: rows - u16::from(status_on),
        ..area
    };

    let k = silhouette::LAMP_WIDTH * 2.0 / input.cell_aspect.clamp(1.0, 4.0);
    let micro = cols < 20 || rows < 8;
    let glass = match input.frame {
        _ if micro => false,
        FrameMode::Bleed => false,
        FrameMode::Glass => true,
        FrameMode::Auto => {
            // Stay glass down to the §2.1 line; re-enter it only well inside.
            let (min_rows, max_aspect) = match input.prev_frame {
                Some(LampFrame::Bleed) => (AUTO_GLASS_ENTER.0, AUTO_GLASS_ENTER.1),
                _ => (AUTO_GLASS.0, AUTO_GLASS.1),
            };
            content.height >= min_rows
                && visual_aspect(content.width, content.height, input.cell_aspect) <= max_aspect
        }
    };
    let margin_steps = [(vm, hm), (0, 0)];

    // Glass, with the panel if one fits (hide order: date, margins, face).
    if glass
        && !input.minimal
        && let Some((lamp, panel)) = glass_with_panel(area, content, &margin_steps, k, input)
    {
        out.lamp = Some(lamp);
        out.panel = Some(panel);
    }
    if glass && out.lamp.is_none() {
        out.lamp = glass_alone(content, &margin_steps, k, input, &mut out.chip);
    }
    if out.lamp.is_none()
        && !input.minimal
        && !micro
        && let Some((lamp, panel)) = bleed_with_panel(area, content, input)
    {
        out.lamp = Some(lamp);
        out.panel = Some(panel);
    }
    let lamp = *out.lamp.get_or_insert(Lamp {
        region: content,
        view: content,
        frame: LampFrame::Bleed,
        glass: None,
    });

    if out.panel.is_none() && out.chip.is_none() {
        out.chip = corner_chip(area, content, &lamp, &margin_steps, input);
    }
    if cols >= 16 && rows >= 4 {
        out.toast = Some(toast_row(area, &lamp, out.panel.as_ref()));
    }
    out
}

/// `margins` applied to `r`, or `None` if nothing is left.
fn shrink(r: Rect, (v, h): (u16, u16)) -> Option<Rect> {
    (r.width > 2 * h && r.height > 2 * v)
        .then(|| Rect::new(r.x + h, r.y + v, r.width - 2 * h, r.height - 2 * v))
}

/// A glass lamp `ht` rows tall at (`x`, `y`).
fn glass_lamp(x: u16, y: u16, ht: u16, k: f64) -> Lamp {
    let w = glass_width(ht, k);
    let (cap, bottle, base) = silhouette::part_rows(ht);
    // The bottle bulge spans the view; keep the parity of `w` so both
    // centre on the same column.
    let side = (f64::from(w) * silhouette::BOTTLE_INSET).round() as u16;
    let region = Rect::new(x, y, w, ht);
    Lamp {
        region,
        view: Rect::new(x + side, y + cap, w - 2 * side, bottle),
        frame: LampFrame::Glass,
        glass: Some(Glass { cap, bottle, base }),
    }
}

fn glass_width(ht: u16, k: f64) -> u16 {
    ((f64::from(ht) * k).round() as u16).max(3)
}

/// Tallest glass lamp whose width fits in `cols`.
fn glass_rows_for_width(cols: u16, k: f64) -> u16 {
    let mut ht = (f64::from(cols) / k).floor() as u16;
    while ht > 0 && glass_width(ht, k) > cols {
        ht -= 1;
    }
    ht
}

fn glass_alone(
    content: Rect,
    margin_steps: &[(u16, u16)],
    k: f64,
    input: &LayoutInput,
    chip: &mut Option<Chip>,
) -> Option<Lamp> {
    // Minimal mode's "under" clock sits on its own row below the base.
    let under = input.minimal
        && match input.chip {
            Some((ChipKind::Clock, _)) => input.minimal_clock == MinimalClock::Under,
            Some((ChipKind::Pomodoro, _)) => input.minimal_clock != MinimalClock::Corner,
            None => false,
        };
    let tries: &[bool] = if under { &[true, false] } else { &[false] };
    for &with_under in tries {
        for &m in margin_steps {
            let Some(c) = shrink(content, m) else {
                continue;
            };
            let reserve = if with_under { 2 } else { 0 };
            let ht = c
                .height
                .saturating_sub(reserve)
                .min(glass_rows_for_width(c.width, k));
            if ht < MIN_GLASS_ROWS {
                continue;
            }
            let w = glass_width(ht, k);
            let group_h = ht + reserve;
            let x = c.x + (c.width - w) / 2;
            let y = c.y + (c.height - group_h) / 2;
            let lamp = glass_lamp(x, y, ht, k);
            if with_under {
                let (kind, text_w) = input.chip.expect("under implies a chip");
                let cw = (text_w + 2).min(content.width);
                let cx = (lamp.centre2() / 2) as u16;
                let rx = cx
                    .saturating_sub(cw / 2)
                    .clamp(content.x, content.right() - cw);
                *chip = Some(Chip {
                    rect: Rect::new(rx, y + group_h - 1, cw, 1),
                    kind,
                    under: true,
                });
            }
            return Some(lamp);
        }
    }
    None
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

fn panel_width(cols: u16) -> u16 {
    ((f64::from(cols) * 0.30).round() as u16).clamp(PANEL_W.0, PANEL_W.1)
}

/// Date line candidates, preferred first: it's the first thing to go.
fn date_options(area_rows: u16, show_clock: bool) -> &'static [bool] {
    if show_clock && area_rows >= 36 {
        &[true, false]
    } else {
        &[false]
    }
}

fn is_huge(cols: u16, rows: u16) -> bool {
    cols >= 200 && rows >= 56
}

fn glass_with_panel(
    area: Rect,
    content: Rect,
    margin_steps: &[(u16, u16)],
    k: f64,
    input: &LayoutInput,
) -> Option<(Lamp, Panel)> {
    let (cols, rows) = (area.width, area.height);
    let huge = is_huge(cols, rows);
    let gutter = (cols / 16).clamp(4, 12);
    let pw = panel_width(cols);
    let dates = date_options(rows, input.show_clock);

    // 1. Right panel: lamp + gutter + panel centred as one group.
    for form in panel_faces(input, pw - 2, huge) {
        for &m in margin_steps {
            let Some(c) = shrink(content, m) else {
                continue;
            };
            for &date in dates {
                let ph = panel_height(form, date);
                let Some(avail) = c.width.checked_sub(gutter + pw) else {
                    continue;
                };
                let ht = c.height.min(glass_rows_for_width(avail, k));
                if ht < PANEL_GLASS_ROWS
                    || f64::from(ht) < 0.85 * f64::from(c.height)
                    || ph > c.height
                {
                    continue;
                }
                let w = glass_width(ht, k);
                let group = w + gutter + pw;
                let x = c.x + (c.width - group) / 2;
                let lamp = glass_lamp(x, c.y + (c.height - ht) / 2, ht, k);
                let py = lamp.region.y + ht.saturating_sub(ph) / 2;
                let py = py.min(c.bottom() - ph);
                return Some((lamp, place_panel(x + w + gutter, py, pw, form, date)));
            }
        }
    }

    // 2. Bottom panel, centred under the base.
    for form in panel_faces(input, PANEL_W.1.min(content.width).saturating_sub(2), huge) {
        for &m in margin_steps {
            let Some(c) = shrink(content, m) else {
                continue;
            };
            for &date in dates {
                let ph = panel_height(form, date);
                let Some(rows_left) = c.height.checked_sub(ph + 2) else {
                    continue;
                };
                // §1.4's 2-col side clearance is the horizontal margin
                // (the 34×56 mockup: a 30-col lamp in 34 cols).
                let ht = rows_left.min(glass_rows_for_width(c.width, k));
                if ht < PANEL_GLASS_ROWS {
                    continue;
                }
                let w = glass_width(ht, k);
                let bw = w.clamp(PANEL_W.0, PANEL_W.1).min(c.width);
                if bw < PANEL_W.0 || form.is_some_and(|f| f.size.width + 2 > bw) {
                    continue;
                }
                let y = c.y + (c.height - (ht + 2 + ph)) / 2;
                let lamp = glass_lamp(c.x + (c.width - w) / 2, y, ht, k);
                let px = ((lamp.centre2() / 2) as u16)
                    .saturating_sub(bw / 2)
                    .clamp(c.x, c.right() - bw);
                return Some((lamp, place_panel(px, y + ht + 2, bw, form, date)));
            }
        }
    }
    None
}

fn bleed_with_panel(area: Rect, content: Rect, input: &LayoutInput) -> Option<(Lamp, Panel)> {
    let (cols, rows) = (area.width, area.height);
    let huge = is_huge(cols, rows);
    let dates = date_options(rows, input.show_clock);
    let tank = |region: Rect| Lamp {
        region,
        view: region,
        frame: LampFrame::Bleed,
        glass: None,
    };

    if visual_aspect(content.width, content.height, input.cell_aspect) >= 1.0 {
        // Right panel if the lamp keeps ≥ 60 % of the width and ≥ 24 cols.
        let pw = panel_width(cols);
        let lamp_w = content.width.checked_sub(pw)?;
        if lamp_w < 24 || u32::from(lamp_w) * 10 < u32::from(content.width) * 6 {
            return None;
        }
        for form in panel_faces(input, pw - 2, huge) {
            for &date in dates {
                let ph = panel_height(form, date);
                if ph > content.height {
                    continue;
                }
                let lamp = tank(Rect {
                    width: lamp_w,
                    ..content
                });
                let py = content.y + (content.height - ph) / 2;
                return Some((lamp, place_panel(content.x + lamp_w, py, pw, form, date)));
            }
        }
    } else {
        // Bottom panel if the lamp keeps ≥ 60 % of the rows and ≥ 10 rows.
        let bw = PANEL_W.1.min(content.width);
        if bw < PANEL_W.0 {
            return None;
        }
        for form in panel_faces(input, bw - 2, huge) {
            for &date in dates {
                // One blank row between the tank and the panel.
                let ph = panel_height(form, date) + 1;
                let Some(lamp_h) = content.height.checked_sub(ph) else {
                    continue;
                };
                if lamp_h < 10 || u32::from(lamp_h) * 10 < u32::from(content.height) * 6 {
                    continue;
                }
                let lamp = tank(Rect {
                    height: lamp_h,
                    ..content
                });
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

/// The chip in a corner: bottom-right of the tank (bleed) or of the
/// glass's content area. Minimal mode puts it in the screen's corner.
fn corner_chip(
    area: Rect,
    content: Rect,
    lamp: &Lamp,
    margin_steps: &[(u16, u16)],
    input: &LayoutInput,
) -> Option<Chip> {
    let (kind, text_w) = input.chip?;
    if area.width < 20 || area.height < 8 {
        return None;
    }
    if input.minimal && kind == ChipKind::Clock && input.minimal_clock == MinimalClock::Off {
        return None;
    }
    let w = (text_w + 2).min(area.width);
    let corner = match lamp.frame {
        LampFrame::Bleed => lamp.region,
        LampFrame::Glass => {
            // The margin box the lamp was placed in, if the chip fits beside it.
            margin_steps
                .iter()
                .filter_map(|&m| shrink(content, m))
                .find(|c| c.contains(lamp.region.as_position()))
                .unwrap_or(content)
        }
    };
    Some(Chip {
        rect: Rect::new(corner.right() - w, corner.bottom() - 1, w, 1),
        kind,
        under: false,
    })
}

/// The toast row: just above the cap (glass) or the tank's top row,
/// spanning symmetrically around the lamp's centre, clear of the panel.
fn toast_row(area: Rect, lamp: &Lamp, panel: Option<&Panel>) -> Rect {
    let y = match lamp.frame {
        LampFrame::Glass if lamp.region.y > area.y => lamp.region.y - 1,
        _ => lamp.region.y,
    };
    let mut right = area.right();
    if let Some(p) = panel
        && p.rect.y <= y
        && y < p.rect.bottom()
        && p.rect.x > lamp.region.x
    {
        right = right.min(p.rect.x);
    }
    let centre2 = lamp.centre2();
    let half = (centre2 / 2 - u32::from(area.x)).min(u32::from(right) - centre2.div_ceil(2));
    let x = (centre2 / 2) as u16 - half as u16;
    let w = (2 * half + centre2 % 2) as u16;
    Rect::new(x, y, w.min(right - x), 1)
}
