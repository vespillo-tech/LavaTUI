//! `blocks` (the default face): a 3×5 pixel font drawn with half-blocks,
//! scaled ×1 (M, 3 rows), ×2 (L, 5 rows) or ×3 (XL, 8 rows).
//!
//! ```text
//! ▄█  █ █ ▄ ▀▀█ ▀▀█
//!  █  ▀▀█ ▄ ▀▀█ █▀▀
//! ▀▀▀   ▀   ▀▀▀ ▀▀▀
//! ```

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::draw::{Bitmap, Glyph, MERIDIEM_W, Pen, glyph_row_width, glyphs, meridiem};
use super::{ClockTime, Face, FaceOptions, FaceStyle, Form, Tier, readout_forms};

pub struct Blocks;

/// 3×5 digits, one `u8` per row, bit 2 = left column.
const FONT: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];
const COLON: [u8; 5] = [0, 1, 0, 1, 0];

fn scale(tier: Tier) -> usize {
    match tier {
        Tier::XL => 3,
        Tier::L => 2,
        _ => 1,
    }
}

/// Pixel width of the readout at scale 1.
fn px_width(seconds: bool) -> usize {
    glyph_row_width(seconds, 3, 1, 1)
}

fn size(tier: Tier, seconds: bool, meridiem: bool) -> (usize, usize) {
    let s = scale(tier);
    let w = px_width(seconds) * s + if meridiem { MERIDIEM_W } else { 0 };
    (w, (5 * s).div_ceil(2))
}

impl Face for Blocks {
    fn name(&self) -> &'static str {
        "blocks"
    }

    fn forms(&self, opts: FaceOptions) -> Vec<Form> {
        readout_forms(opts, &[Tier::XL, Tier::L, Tier::M], size)
    }

    fn draw(
        &self,
        form: Form,
        time: ClockTime,
        opts: FaceOptions,
        area: Rect,
        buf: &mut Buffer,
        style: FaceStyle,
    ) {
        let s = scale(form.tier);
        let mut bmp = Bitmap::new(px_width(form.seconds) * s, 5 * s);
        let mut x = 0;
        for glyph in glyphs(time, opts.hour24, form.seconds) {
            let (rows, w): ([u8; 5], usize) = match glyph {
                Glyph::Digit(d) => (FONT[usize::from(d)], 3),
                Glyph::Blank => ([0; 5], 3),
                Glyph::Colon => (COLON, 1),
            };
            for (py, row) in rows.iter().enumerate() {
                for px in 0..w {
                    if row >> (w - 1 - px) & 1 == 1 {
                        for (sx, sy) in (0..s).flat_map(|sx| (0..s).map(move |sy| (sx, sy))) {
                            bmp.set(((x + px) * s + sx) as i32, (py * s + sy) as i32);
                        }
                    }
                }
            }
            x += w + 1;
        }
        let mut pen = Pen::new(buf, area);
        bmp.draw_halfblocks(&mut pen, style.main);
        if form.meridiem {
            let (w, h) = size(form.tier, form.seconds, false);
            pen.text(w, h - 1, meridiem(time), style.dim);
        }
    }
}
