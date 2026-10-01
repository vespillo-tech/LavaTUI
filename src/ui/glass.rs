//! The glass lamp silhouette (§2.1): cap and base in `metal`, shaded
//! top-light → bottom-dark, with half-column edges so the taper is smooth
//! and odd/even widths centre exactly. The bottle itself is the lamp view
//! (its liquid and wax come from the style, its half-cell walls from
//! `render`); the glass has no outline, except in 16-colour / NO_COLOR
//! where there's no liquid tint to show it, so a thin `▕ │ ▏` edge marks
//! the walls instead.

use ratatui::buffer::Buffer;
use ratatui::style::Style;

use crate::silhouette::{BASE, CAP, row_height, row_span};
use crate::sim::Shape;
use crate::theme::{Ink, Role, Theme};
use crate::ui::layout::{Glass, Lamp};

/// Metal brightness at the top of the cap and the bottom of the base.
const SHADE: (f32, f32) = (1.25, 0.7);

/// Draw the cap, base and (in 16 colours / NO_COLOR) the wall edges.
/// `flash` is the phase-change flash level (0..1).
pub fn draw(buf: &mut Buffer, lamp: &Lamp, glass: Glass, theme: &Theme, flash: f32) {
    let r = lamp.region;
    // NO_COLOR: metal and wax are both the terminal's foreground, so metal
    // takes a lighter texture to keep the pool apart from the base.
    let fill = if theme.has_color() { '█' } else { '▒' };
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
            span(buf, lamp, r.y + row, frac, fill, style);
        }
    }

    if !theme.blends() {
        edges(buf, lamp, Style::new().fg(theme.role(Role::Metal)));
    }
}

/// One row of metal `frac` of the lamp width, centred, with half-cell
/// precision; whole cells are `fill`.
fn span(buf: &mut Buffer, lamp: &Lamp, y: u16, frac: f64, fill: char, style: Style) {
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
            (true, true) => fill,
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
        let world_y = row_height(u32::from(j), u32::from(v.height));
        let (lo, hi) = row_span(Shape::Bottle, v.width, world_y);
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

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;
    use crate::theme::{ColorDepth, Palette};
    use crate::ui::layout::LampFrame;

    fn base_row(depth: ColorDepth) -> String {
        let region = Rect::new(0, 0, 16, 20);
        let glass = Glass {
            cap: 3,
            bottle: 13,
            base: 4,
        };
        let lamp = Lamp {
            region,
            view: Rect::new(2, 3, 12, 13),
            frame: LampFrame::Glass,
            glass: Some(glass),
        };
        let theme = Theme::new(&Palette::all()[0], depth);
        let mut buf = Buffer::empty(region);
        draw(&mut buf, &lamp, glass, &theme, 0.0);
        (0..16).map(|x| buf[(x, 17)].symbol()).collect()
    }

    /// NO_COLOR metal is textured, so the wax pool (solid in the terminal
    /// foreground) stays apart from the base (lava-ebq.13).
    #[test]
    fn no_color_metal_is_textured() {
        assert!(base_row(ColorDepth::None).contains('▒'));
        assert!(!base_row(ColorDepth::None).contains('█'));
        assert!(base_row(ColorDepth::Ansi16).contains('█'));
        assert!(base_row(ColorDepth::TrueColor).contains('█'));
    }
}
