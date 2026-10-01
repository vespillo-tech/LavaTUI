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

/// Braille dot bit for sub-pixel (`x` 0..2, `y` 0..4) of a cell.
#[inline]
pub fn braille_bit(x: usize, y: usize) -> u8 {
    const BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    BITS[x][y]
}

/// The braille glyph with dots `bits` set (blank braille for 0).
#[inline]
pub fn braille(bits: u8) -> char {
    char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' ')
}
