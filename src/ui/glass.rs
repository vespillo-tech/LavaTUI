//! The glass lamp silhouette (§2.1): cap and base in `metal`, shaded
//! top-light → bottom-dark, with half-column edges so the taper is smooth
//! and odd/even widths centre exactly. The bottle itself is the lamp view
//! (its liquid and wax come from the style); the glass has no outline,
//! except in 16-colour / NO_COLOR where there's no liquid tint to show it,
//! so a thin `▕ ▏` edge marks the walls instead.

use ratatui::buffer::Buffer;
use ratatui::style::Style;

use crate::app::{FLASH_TIME, Model};
use crate::sim::Shape;
use crate::theme::{Ink, Role};
use crate::ui::layout::{Glass, Lamp};

/// Cap: 0.18 → 0.40 of the lamp width; base: 0.56 → 1.0 (§2.1).
const CAP: (f64, f64) = (0.18, 0.40);
const BASE: (f64, f64) = (0.56, 1.0);
/// Metal brightness at the top of the cap and the bottom of the base.
const SHADE: (f32, f32) = (1.25, 0.7);

pub fn draw(buf: &mut Buffer, lamp: &Lamp, glass: Glass, model: &Model) {
    let r = lamp.region;
    let theme = &model.theme;
    let flash = model.flash.map_or(0.0, |at| {
        let t = model.now.duration_since(at).as_secs_f32() / FLASH_TIME.as_secs_f32();
        (t * std::f32::consts::PI).sin().max(0.0)
    });
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

/// `▕ … ▏` just outside the bottle walls (no-colour depths only).
fn edges(buf: &mut Buffer, lamp: &Lamp, style: Style) {
    let v = lamp.view;
    let w = f64::from(v.width);
    for j in 0..v.height {
        let world_y = 1.0 - (f64::from(j) + 0.5) / f64::from(v.height);
        let half = Shape::Bottle.width_fraction(world_y) * w / 2.0;
        let lo = (w / 2.0 - half).round().max(0.0) as u16;
        let hi = v.width - lo.min(v.width / 2);
        let y = v.y + j;
        if v.x + lo > lamp.region.x
            && let Some(cell) = buf.cell_mut((v.x + lo - 1, y))
        {
            cell.set_char('▕').set_style(style);
        }
        if v.x + hi < lamp.region.right()
            && let Some(cell) = buf.cell_mut((v.x + hi, y))
        {
            cell.set_char('▏').set_style(style);
        }
    }
}
