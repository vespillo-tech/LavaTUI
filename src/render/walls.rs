//! The container's walls at sub-cell precision (docs/design.md §2.1).
//!
//! Styles see a per-cell mask: a cell the wall passes through counts as
//! inside, so it's drawn like any other liquid / wax cell. Then, when the
//! theme blends, [`smooth`] reshapes each of those edge cells into a
//! quadrant glyph (`▐ ▌ ▗ ▟ …`): the inside quadrants in the colour the
//! style drew just inside the wall, the outside ones in `bg`. Walls land
//! on half columns and half rows, so the bottle's taper is smooth instead
//! of stepping a whole cell at a time, and odd / even widths centre
//! exactly.

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::Grid;
use crate::sim::Shape;
use crate::theme::{Role, Theme};

/// Half-width of `shape` at height `world_y` (0 bottom … 1 top) in a view
/// `cols` wide, in half columns either side of the view's centre. The same
/// rounding as the glass cap and base (`ui::glass`), so they meet flush.
pub fn wall(shape: Shape, cols: u16, world_y: f64) -> u32 {
    let n = (shape.width_fraction(world_y) * f64::from(cols)).round();
    n.clamp(1.0, f64::from(cols)) as u32
}

/// Inside span `[lo, hi)`, in half columns from the view's left edge, for
/// each half row of cell row `row`: `[top, bottom]`.
fn spans(shape: Shape, cols: u16, rows: u16, row: u16) -> [(u32, u32); 2] {
    let half_row = |h: u32| {
        let world_y = 1.0 - (f64::from(h) + 0.5) / (2.0 * f64::from(rows));
        let n = wall(shape, cols, world_y);
        (u32::from(cols) - n, u32::from(cols) + n)
    };
    let h = 2 * u32::from(row);
    [half_row(h), half_row(h + 1)]
}

/// The cells `[lo, hi)` of a row that the container touches at all.
fn cells(spans: [(u32, u32); 2]) -> (usize, usize) {
    let [(lo_t, hi_t), (lo_b, hi_b)] = spans;
    (
        (lo_t.min(lo_b) / 2) as usize,
        hi_t.max(hi_b).div_ceil(2) as usize,
    )
}

/// The container mask for a `grid` canvas over `area`: per sample row,
/// the `[lo, hi)` sample columns inside. Per cell row, so a cell the wall
/// cuts through is wholly inside for the style; [`smooth`] shapes it
/// afterwards.
pub fn mask(shape: Shape, area: Rect, grid: Grid, mask: &mut Vec<(usize, usize)>) {
    mask.clear();
    for row in 0..area.height {
        let (lo, hi) = cells(spans(shape, area.width, area.height, row));
        let span = (lo * usize::from(grid.x), hi * usize::from(grid.x));
        mask.extend(std::iter::repeat_n(span, usize::from(grid.y)));
    }
}

/// Reshape the cells of `area` that the walls cut through into quadrant
/// glyphs. Only for blending themes: without a liquid tint there's nothing
/// to shape, and the glass draws a `▕ │ ▏` edge instead.
pub fn smooth(shape: Shape, theme: &Theme, area: Rect, buf: &mut Buffer) {
    let outside = theme.role(Role::Bg);
    for row in 0..area.height {
        let spans = spans(shape, area.width, area.height, row);
        let (lo, hi) = cells(spans);
        // The cut cells are the runs either side of the whole ones.
        let full = |x: &usize| quadrants(spans, *x) == FULL;
        let first = (lo..hi).find(full).unwrap_or(hi);
        let last = (first..hi).rev().find(full).unwrap_or(first);
        // An edge cell's own samples sit on the wall, half in the glass
        // and half out, so they're washed out; take the colour from the
        // nearest whole cell inside instead (or its own, if there's none).
        let at = |x: usize| (area.x + x as u16, area.y + row);
        let (into_left, into_right) = if first < hi {
            (
                halves(theme, &buf[at(first)]),
                halves(theme, &buf[at(last)]),
            )
        } else {
            let own = halves(theme, &buf[at(lo)]);
            (own, own)
        };
        let left = (lo..first).map(|x| (x, into_left));
        let right = (last + 1..hi).map(|x| (x, into_right));
        for (x, (top, bottom)) in left.chain(right) {
            let bits = quadrants(spans, x);
            let inside = match (bits & 0b1100 != 0, bits & 0b0011 != 0) {
                (true, false) => top,
                (false, true) => bottom,
                _ => theme.blend(top, bottom, 0.5),
            };
            buf[at(x)]
                .set_char(QUADRANT[usize::from(bits)])
                .set_fg(inside)
                .set_bg(outside);
        }
    }
}

/// The colours a cell shows in its top and bottom halves.
fn halves(theme: &Theme, cell: &Cell) -> (Color, Color) {
    match cell.symbol() {
        "█" => (cell.fg, cell.fg),
        "▀" => (cell.fg, cell.bg),
        "▄" => (cell.bg, cell.fg),
        symbol => {
            let c = theme.blend(cell.bg, cell.fg, ink(symbol));
            (c, c)
        }
    }
}

const FULL: u8 = 0b1111;

/// Which quadrants of cell `x` are inside: bit 3 top-left, 2 top-right,
/// 1 bottom-left, 0 bottom-right.
fn quadrants(spans: [(u32, u32); 2], x: usize) -> u8 {
    let mut bits = 0;
    for (i, (lo, hi)) in spans.into_iter().enumerate() {
        for half in 0..2 {
            if (lo..hi).contains(&(2 * x as u32 + half)) {
                bits |= 1 << (3 - 2 * i as u32 - half);
            }
        }
    }
    bits
}

/// Quadrant glyph for each inside-bits value (see [`quadrants`]).
const QUADRANT: [char; 16] = [
    ' ', '▗', '▖', '▄', '▝', '▐', '▞', '▟', '▘', '▚', '▌', '▙', '▀', '▜', '▛', '█',
];

/// How much of a cell a block glyph fills in its foreground colour, so an
/// edge cell gets the average colour the style drew next to it. Line,
/// dot and letter glyphs (braille, ascii, …) count as none: a half-cell
/// solid tint of them would read as a block in a style made of marks, so
/// the wall shows the liquid behind them instead.
fn ink(symbol: &str) -> f32 {
    match symbol.chars().next() {
        Some('█') => 1.0,
        Some('▓' | '▛' | '▜' | '▙' | '▟') => 0.75,
        Some('▀' | '▄' | '▌' | '▐' | '▒' | '▚' | '▞') => 0.5,
        Some('░' | '▘' | '▝' | '▖' | '▗') => 0.25,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The glyph seen in a mirror (left ↔ right).
    fn mirror(c: char) -> char {
        let i = QUADRANT.iter().position(|&q| q == c).unwrap() as u8;
        // Swap left / right in each half row: bits 3 ↔ 2, 1 ↔ 0.
        let m = (i & 0b1010) >> 1 | (i & 0b0101) << 1;
        QUADRANT[usize::from(m)]
    }

    #[test]
    fn walls_are_symmetric_at_odd_and_even_widths() {
        for cols in 1..=60 {
            for rows in [1, 2, 7, 20, 33] {
                for row in 0..rows {
                    let spans = spans(Shape::Bottle, cols, rows, row);
                    let (lo, hi) = cells(spans);
                    assert!(
                        hi <= usize::from(cols) && lo < hi,
                        "{cols}x{rows} row {row}"
                    );
                    for (l, h) in spans {
                        assert_eq!(l + h, 2 * u32::from(cols), "centred");
                    }
                    for x in lo..hi {
                        let (a, b) = (
                            quadrants(spans, x),
                            quadrants(spans, usize::from(cols) - 1 - x),
                        );
                        assert_ne!(a, 0, "{cols}x{rows} row {row}: cell {x} in the span");
                        assert_eq!(QUADRANT[usize::from(b)], mirror(QUADRANT[usize::from(a)]));
                    }
                }
            }
        }
    }

    #[test]
    fn walls_land_on_half_columns() {
        // 10 cols, bottle bulge = full width: a fraction of 0.55 is 5.5
        // cols → 11 half columns; the wall cuts a cell in half.
        let rows = 40;
        let cuts = (0..rows)
            .flat_map(|row| spans(Shape::Bottle, 11, rows, row))
            .filter(|(lo, _)| lo % 2 == 1)
            .count();
        assert!(cuts > 0, "some rows end mid-cell");
        // A tank fills every cell: nothing to reshape.
        for row in 0..rows {
            let spans = spans(Shape::Tank, 11, rows, row);
            assert_eq!(spans, [(0, 22); 2]);
            assert!((0..11).all(|x| quadrants(spans, x) == FULL));
        }
    }

    #[test]
    fn block_glyphs_count_as_ink_and_marks_do_not() {
        assert_eq!(ink("█"), 1.0);
        assert_eq!(ink("▀"), 0.5);
        assert_eq!(ink(" "), 0.0);
        assert_eq!(ink("⣿"), 0.0);
        assert_eq!(ink("#"), 0.0);
    }
}
