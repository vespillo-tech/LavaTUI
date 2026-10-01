//! Drawing helpers shared by the faces: a clip-safe [`Pen`], a 1-bit
//! [`Bitmap`] that renders as half-blocks or braille, and the glyph sequence
//! for an `HH:MM[:SS]` readout.
//!
//! Nothing here can write outside the rect it was given, which is what lets
//! every face promise "never overflows".

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::ClockTime;

/// Writes cells relative to a rect, silently dropping anything outside it
/// (or outside the buffer). Spaces are transparent: they are skipped, so a
/// face never blanks out what is underneath it.
pub struct Pen<'b> {
    buf: &'b mut Buffer,
    area: Rect,
}

impl<'b> Pen<'b> {
    pub fn new(buf: &'b mut Buffer, area: Rect) -> Self {
        let area = area.intersection(buf.area);
        Self { buf, area }
    }

    pub fn put(&mut self, x: usize, y: usize, ch: char, style: Style) {
        if ch == ' ' || x >= usize::from(self.area.width) || y >= usize::from(self.area.height) {
            return;
        }
        // In range of a u16 width/height, so the casts are lossless.
        let pos = (self.area.x + x as u16, self.area.y + y as u16);
        if let Some(cell) = self.buf.cell_mut(pos) {
            cell.set_char(ch).set_style(style);
        }
    }

    pub fn text(&mut self, x: usize, y: usize, s: &str, style: Style) {
        for (i, ch) in s.chars().enumerate() {
            self.put(x + i, y, ch, style);
        }
    }
}

/// A 1-bit pixel canvas. Out-of-range `set`s are ignored.
pub struct Bitmap {
    w: usize,
    h: usize,
    px: Vec<bool>,
}

impl Bitmap {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![false; w * h],
        }
    }

    pub fn set(&mut self, x: i32, y: i32) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = true;
        }
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        x < self.w && y < self.h && self.px[y * self.w + x]
    }

    /// Bresenham line, endpoints included.
    pub fn line(&mut self, (x0, y0): (i32, i32), (x1, y1): (i32, i32)) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = ((x1 - x0).signum(), (y1 - y0).signum());
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            self.set(x, y);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Two pixel rows per cell (`▀ ▄ █`): square-ish pixels on a 1:2 cell.
    pub fn draw_halfblocks(&self, pen: &mut Pen, style: Style) {
        for cy in 0..self.h.div_ceil(2) {
            for cx in 0..self.w {
                let ch = match (self.get(cx, cy * 2), self.get(cx, cy * 2 + 1)) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => continue,
                };
                pen.put(cx, cy, ch, style);
            }
        }
    }

    /// Braille dot pattern (bits per Unicode's dot numbering) for one cell.
    pub fn braille_bits(&self, cx: usize, cy: usize) -> u8 {
        const DOTS: [(usize, usize, u8); 8] = [
            (0, 0, 0x01),
            (0, 1, 0x02),
            (0, 2, 0x04),
            (1, 0, 0x08),
            (1, 1, 0x10),
            (1, 2, 0x20),
            (0, 3, 0x40),
            (1, 3, 0x80),
        ];
        DOTS.iter()
            .filter(|(dx, dy, _)| self.get(cx * 2 + dx, cy * 4 + dy))
            .fold(0, |bits, (_, _, bit)| bits | bit)
    }
}

pub fn braille_char(bits: u8) -> char {
    char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' ')
}

/// One glyph of a digital readout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Digit(u8),
    /// An unlit digit slot (12h hours below 10), so the colon never moves.
    Blank,
    Colon,
}

/// `HH:MM` or `HH:MM:SS`. In 12h mode the hour runs 1–12 with a blank tens
/// slot rather than a leading zero.
pub fn glyphs(time: ClockTime, hour24: bool, seconds: bool) -> Vec<Glyph> {
    let hour = time.display_hour(hour24);
    let tens = if hour24 || hour >= 10 {
        Glyph::Digit(hour / 10)
    } else {
        Glyph::Blank
    };
    let mut out = vec![
        tens,
        Glyph::Digit(hour % 10),
        Glyph::Colon,
        Glyph::Digit(time.minute / 10),
        Glyph::Digit(time.minute % 10),
    ];
    if seconds {
        out.extend([
            Glyph::Colon,
            Glyph::Digit(time.second / 10),
            Glyph::Digit(time.second % 10),
        ]);
    }
    out
}

/// Width of a glyph row: `digit_w` per digit/blank, `colon_w` per colon,
/// `gap` between neighbours.
pub fn glyph_row_width(seconds: bool, digit_w: usize, colon_w: usize, gap: usize) -> usize {
    let (digits, colons) = if seconds { (6, 2) } else { (4, 1) };
    digits * digit_w + colons * colon_w + (digits + colons - 1) * gap
}

/// The `am`/`pm` suffix, drawn dim after the digits.
pub const MERIDIEM_W: usize = 3;

pub fn meridiem(time: ClockTime) -> &'static str {
    if time.hour < 12 { " am" } else { " pm" }
}
