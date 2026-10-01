//! `segment`: seven-segment digits. M is the classic 3-row `_|` look; L and
//! XL use heavy box-drawing strokes with the unlit segments drawn dim, like
//! a real LCD.
//!
//! ```text
//!              ━━   ━━
//!    ┃ ┃  ┃ •    ┃    ┃
//!       ━━     ━━   ━━
//!    ┃    ┃ •    ┃ ┃
//!              ━━   ━━     (L at 14:32, ghost segments not shown)
//! ```

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::draw::{Glyph, MERIDIEM_W, Pen, glyph_row_width, glyphs, meridiem};
use super::{ClockTime, Face, FaceOptions, FaceStyle, Form, Tier, readout_forms};

pub struct Segment;

// Segment bits: a top, b top-right, c bottom-right, d bottom, e bottom-left,
// f top-left, g middle.
const A: u8 = 1;
const B: u8 = 1 << 1;
const C: u8 = 1 << 2;
const D: u8 = 1 << 3;
const E: u8 = 1 << 4;
const F: u8 = 1 << 5;
const G: u8 = 1 << 6;

const DIGITS: [u8; 10] = [
    A | B | C | D | E | F,
    B | C,
    A | B | D | E | G,
    A | B | C | D | G,
    B | C | F | G,
    A | C | D | F | G,
    A | C | D | E | F | G,
    A | B | C,
    A | B | C | D | E | F | G,
    A | B | C | D | F | G,
];

/// Digit geometry per tier: (digit width, inner half-height, gap).
/// M uses its own 3×3 ASCII layout; the others are box-drawn with
/// `rows = 2·half + 3` and `cols = inner + 2`.
fn geometry(tier: Tier) -> (usize, usize, usize) {
    match tier {
        Tier::XL => (6, 2, 2),
        Tier::L => (4, 1, 1),
        _ => (3, 0, 1),
    }
}

fn size(tier: Tier, seconds: bool, meridiem: bool) -> (usize, usize) {
    let (dw, half, gap) = geometry(tier);
    let h = if tier == Tier::M { 3 } else { 2 * half + 3 };
    let w = glyph_row_width(seconds, dw, 1, gap) + if meridiem { MERIDIEM_W } else { 0 };
    (w, h)
}

/// Classic 3×3: `_` sits at the bottom of its cell, so a top bar lives on
/// row 0 and the middle/bottom bars share rows with the verticals.
fn draw_ascii(pen: &mut Pen, x: usize, segs: u8, style: Style) {
    let cells = [
        (A, 1, 0, '_'),
        (F, 0, 1, '|'),
        (G, 1, 1, '_'),
        (B, 2, 1, '|'),
        (E, 0, 2, '|'),
        (D, 1, 2, '_'),
        (C, 2, 2, '|'),
    ];
    for (seg, dx, dy, ch) in cells {
        if segs & seg != 0 {
            pen.put(x + dx, dy, ch, style);
        }
    }
}

/// Box-drawn digit; unlit segments drawn as dim ghosts.
fn draw_box(pen: &mut Pen, x: usize, segs: u8, tier: Tier, style: FaceStyle) {
    let (dw, half, _) = geometry(tier);
    let ink = |seg: u8| {
        if segs & seg != 0 {
            style.main
        } else {
            style.dim
        }
    };
    for i in 1..dw - 1 {
        pen.put(x + i, 0, '━', ink(A));
        pen.put(x + i, half + 1, '━', ink(G));
        pen.put(x + i, 2 * half + 2, '━', ink(D));
    }
    for j in 1..=half {
        pen.put(x, j, '┃', ink(F));
        pen.put(x + dw - 1, j, '┃', ink(B));
        pen.put(x, half + 1 + j, '┃', ink(E));
        pen.put(x + dw - 1, half + 1 + j, '┃', ink(C));
    }
}

impl Face for Segment {
    fn name(&self) -> &'static str {
        "segment"
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
        let (dw, half, gap) = geometry(form.tier);
        let mut pen = Pen::new(buf, area);
        let mut x = 0;
        for glyph in glyphs(time, opts.hour24, form.seconds) {
            let segs = match glyph {
                Glyph::Digit(d) => DIGITS[usize::from(d)],
                Glyph::Blank => 0,
                Glyph::Colon => {
                    let (dot, rows) = if form.tier == Tier::M {
                        ('·', [1, 2])
                    } else {
                        ('•', [half, half + 2])
                    };
                    for y in rows {
                        pen.put(x, y, dot, style.main);
                    }
                    x += 1 + gap;
                    continue;
                }
            };
            if form.tier == Tier::M {
                draw_ascii(&mut pen, x, segs, style.main);
            } else {
                draw_box(&mut pen, x, segs, form.tier, style);
            }
            x += dw + gap;
        }
        if form.meridiem {
            let (w, h) = size(form.tier, form.seconds, false);
            pen.text(w, h - 1, meridiem(time), style.dim);
        }
    }
}
