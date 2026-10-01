//! Glyph helpers shared by styles.

use ratatui::buffer::Cell;
use ratatui::style::Color;

/// Draw two vertically stacked pixels in one cell. `None` is an empty
/// pixel (shows `base`, the backdrop colour); `Some(c)` is ink in `c`,
/// which may be `Color::Reset` (terminal foreground) when there's no
/// colour. Picks `▀ ▄ █` or a space so empty pixels never need a colour
/// that can't be a foreground.
#[inline]
pub fn half_block(cell: &mut Cell, top: Option<Color>, bottom: Option<Color>, base: Color) {
    let (ch, fg, bg) = match (top, bottom) {
        (None, None) => (' ', Color::Reset, base),
        (Some(t), None) => ('▀', t, base),
        (None, Some(b)) => ('▄', b, base),
        (Some(t), Some(b)) if t == b => ('█', t, base),
        (Some(t), Some(b)) => ('▀', t, b),
    };
    cell.set_char(ch).set_fg(fg).set_bg(bg);
}

/// An empty cell: a space on `bg`. The foreground is left as it was.
#[inline]
pub fn blank(cell: &mut Cell, bg: Color) {
    cell.set_char(' ').set_bg(bg);
}

/// `ch` in `fg` on `bg`.
#[inline]
pub fn glyph(cell: &mut Cell, ch: char, fg: Color, bg: Color) {
    cell.set_char(ch).set_fg(fg).set_bg(bg);
}

/// `Some((ch, fg))` as a [`glyph`] on `bg`, `None` as a [`blank`].
#[inline]
pub fn mark(cell: &mut Cell, mark: Option<(char, Color)>, bg: Color) {
    match mark {
        Some((ch, fg)) => glyph(cell, ch, fg, bg),
        None => blank(cell, bg),
    }
}

/// The braille dots of cell (`cx`, `cy`) on a 2×4 grid: one per sample
/// pixel for which `dot(x, y)` holds. Pixels are visited row by row, left
/// to right, so anything `dot` accumulates adds up in a fixed order.
#[inline]
pub fn braille_dots(cx: usize, cy: usize, mut dot: impl FnMut(usize, usize) -> bool) -> u8 {
    /// Dot bit per sub-pixel, by row then column.
    const BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let mut bits = 0;
    for (dy, row) in BITS.iter().enumerate() {
        for (dx, bit) in row.iter().enumerate() {
            if dot(2 * cx + dx, 4 * cy + dy) {
                bits |= bit;
            }
        }
    }
    bits
}

/// The braille glyph with dots `bits` set (blank braille for 0).
#[inline]
pub fn braille(bits: u8) -> char {
    char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' ')
}
