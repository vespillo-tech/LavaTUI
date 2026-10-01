//! The glass lamp silhouette (§2.1): cap and base in `metal`, shaded
//! top-light → bottom-dark, with half-column edges so the taper is smooth
//! and odd/even widths centre exactly. The bottle itself is the lamp view
//! (its liquid and wax come from the style, its half-cell walls from
//! `render`); the glass has no outline, except in 16-colour / NO_COLOR
//! where there's no liquid tint to show it, so a thin `▕ │ ▏` edge marks
//! the walls instead. With lighting on, a soft highlight streak runs down
//! the bottle's left side.

use ratatui::buffer::Buffer;
use ratatui::style::Style;

use crate::app::Model;
use crate::render::{smoothstep, wall};
use crate::sim::Shape;
use crate::theme::{Ink, Role};
use crate::ui::layout::{Glass, Lamp};

/// Cap: 0.18 → 0.40 of the lamp width; base: 0.56 → 1.0 (§2.1).
const CAP: (f64, f64) = (0.18, 0.40);
const BASE: (f64, f64) = (0.56, 1.0);
/// Metal brightness at the top of the cap and the bottom of the base.
const SHADE: (f32, f32) = (1.25, 0.7);
/// Highlight streak: how far toward `text` at its brightest (§2.1: ~20 %),
/// and where it sits, as a share of the bottle's half-width in from the
/// left wall.
const STREAK: f32 = 0.18;
const STREAK_INSET: f64 = 0.32;

pub fn draw(buf: &mut Buffer, lamp: &Lamp, glass: Glass, model: &Model) {
    let r = lamp.region;
    let theme = &model.theme;
    let flash = model.flash_level();
    let metal = |row: u16| {
        let t = f32::from(row) / f32::from(r.height.max(2) - 1);
        theme
            .paint(Ink::Role(Role::Metal))
            .scale(SHADE.0 + (SHADE.1 - SHADE.0) * t)
            .mix(Ink::Role(Role::Accent), flash)
            .color()
    };

    let parts = [
        (0, glass.cap, CAP),
        (glass.cap + glass.bottle, glass.base, BASE),
    ];
    for (top, rows, (from, to)) in parts {
        for i in 0..rows {
            let t = (f64::from(i) + 0.5) / f64::from(rows);
            let frac = from + (to - from) * t;
            let row = top + i;
            let style = Style::new().fg(metal(row));
            span(buf, lamp, r.y + row, frac, style);
        }
    }

    if !theme.blends() {
        edges(buf, lamp, Style::new().fg(theme.role(Role::Metal)));
    } else if model.settings.lamp.lighting {
        streak(buf, lamp, model);
    }
}

/// One row of metal `frac` of the lamp width, centred, with half-cell
/// precision.
fn span(buf: &mut Buffer, lamp: &Lamp, y: u16, frac: f64, style: Style) {
    let r = lamp.region;
    // Half-column units: a span `n` cols wide reaches `n` half-columns
    // either side of the centre, so edges land on half cells.
    let centre2 = i64::from(lamp.centre2());
    let n = (frac * f64::from(r.width)).round().max(1.0) as i64;
    let left = (centre2 - n).max(2 * i64::from(r.x));
    let right = (centre2 + n).min(2 * i64::from(r.right()));
    for col in r.x..r.right() {
        let (a, b) = (2 * i64::from(col), 2 * i64::from(col) + 1);
        let in_a = (left..right).contains(&a);
        let in_b = (left..right).contains(&b);
        let ch = match (in_a, in_b) {
            (true, true) => '█',
            (true, false) => '▌',
            (false, true) => '▐',
            (false, false) => continue,
        };
        if let Some(cell) = buf.cell_mut((col, y)) {
            cell.set_char(ch).set_style(style);
        }
    }
}

/// The walls in `metal` (no-colour depths only), at half-column
/// precision: `▕` / `▏` just outside a wall on a cell boundary, `│` on a
/// wall through the middle of a cell.
fn edges(buf: &mut Buffer, lamp: &Lamp, style: Style) {
    let v = lamp.view;
    let r = lamp.region;
    for j in 0..v.height {
        let world_y = 1.0 - (f64::from(j) + 0.5) / f64::from(v.height);
        let n = wall(Shape::Bottle, v.width, world_y);
        let (lo, hi) = (u32::from(v.width) - n, u32::from(v.width) + n);
        // Half-column → (cell, glyph); `None` if it falls off the region.
        let left = match lo % 2 {
            0 => (v.x + (lo / 2) as u16).checked_sub(1).map(|x| (x, '▕')),
            _ => Some((v.x + (lo / 2) as u16, '│')),
        };
        let right = match hi % 2 {
            0 => (v.x + (hi / 2) as u16, '▏'),
            _ => (v.x + (hi / 2) as u16, '│'),
        };
        for (x, ch) in left.into_iter().chain([right]) {
            if (r.x..r.right()).contains(&x)
                && let Some(cell) = buf.cell_mut((x, v.y + j))
            {
                cell.set_char(ch).set_style(style);
            }
        }
    }
}

/// The glass highlight: a one-cell band a little in from the left wall,
/// following its curve, brightest down the upper body and fading out at
/// the shoulder and above the base light. It sits on the glass, so wax
/// passing behind it is lifted too.
fn streak(buf: &mut Buffer, lamp: &Lamp, model: &Model) {
    let v = lamp.view;
    let theme = &model.theme;
    let text = theme.role(Role::Text);
    let rows = f64::from(v.height);
    for j in 0..v.height {
        let world_y = |dy: f64| 1.0 - (f64::from(j) + dy) / rows;
        let y = world_y(0.5) as f32;
        let level = STREAK * smoothstep((y - 0.3) / 0.25) * smoothstep((0.97 - y) / 0.2);
        if level <= 0.0 {
            continue;
        }
        // Band centre in half columns from the view's left. Unrounded, so
        // it drifts across cells smoothly.
        let n = Shape::Bottle.width_fraction(world_y(0.5)) * f64::from(v.width);
        let centre = f64::from(v.width) - n * (1.0 - STREAK_INSET);
        // Never on a cell the wall cuts (those show `bg` on one side).
        let inner =
            [0.25, 0.75].map(|dy| u32::from(v.width) - wall(Shape::Bottle, v.width, world_y(dy)));
        let first = inner[0].max(inner[1]).div_ceil(2);
        // The nearest cell carries the full level and the next one fades
        // in as the band crosses over, so it never dims or splits.
        let near = (centre / 2.0).floor() as u32;
        let d = (centre - f64::from(2 * near + 1)).abs() / 2.0;
        let next = if centre >= f64::from(2 * near + 1) {
            near + 1
        } else {
            near.wrapping_sub(1)
        };
        for (c, w) in [(near, 1.0), (next, d / (1.0 - d))] {
            let amount = level * w as f32;
            if c < first || c >= u32::from(v.width) || amount <= 0.0 {
                continue;
            }
            if let Some(cell) = buf.cell_mut((v.x + c as u16, v.y + j)) {
                let (fg, bg) = (cell.fg, cell.bg);
                cell.set_fg(theme.blend(fg, text, amount))
                    .set_bg(theme.blend(bg, text, amount));
            }
        }
    }
}
