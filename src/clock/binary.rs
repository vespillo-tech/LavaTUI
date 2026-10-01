//! `binary`: a BCD binary clock, one column per digit, bits 8·4·2·1 from
//! the top. Impossible bits (e.g. 8 and 4 for the hour's tens) are left
//! out. S is the bare dots; M spreads them out and labels each column.
//!
//! ```text
//!    ○       ○
//!    ●    ○  ○
//! ○  ○    ●  ●
//! ●  ○    ●  ○
//!
//! 1  4    3  2      (M at 14:32)
//! ```

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::draw::{Glyph, Pen, glyphs};
use super::{ClockTime, Face, FaceOptions, FaceStyle, Form, Tier, readout_forms};

pub struct Binary;

/// (in-pair gap, between-pair gap) per tier.
fn spacing(tier: Tier) -> (usize, usize) {
    if tier == Tier::M { (2, 4) } else { (1, 3) }
}

fn size(tier: Tier, seconds: bool, _meridiem: bool) -> (usize, usize) {
    let (inner, outer) = spacing(tier);
    let pairs = if seconds { 3 } else { 2 };
    let w = pairs * (2 + inner) + (pairs - 1) * outer;
    (w, if tier == Tier::M { 6 } else { 4 })
}

impl Face for Binary {
    fn name(&self) -> &'static str {
        "binary"
    }

    fn forms(&self, opts: FaceOptions) -> Vec<Form> {
        // No room for am/pm in a bit grid; 12h just changes the hour digits.
        let grid = FaceOptions {
            hour24: true,
            ..opts
        };
        readout_forms(grid, &[Tier::M, Tier::S], size)
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
        let (inner, outer) = spacing(form.tier);
        let digits: Vec<u8> = glyphs(time, opts.hour24, form.seconds)
            .into_iter()
            .filter_map(|g| match g {
                Glyph::Digit(d) => Some(d),
                Glyph::Blank => Some(0),
                Glyph::Colon => None,
            })
            .collect();
        // Largest value each column can show: hour tens, then 0-9 / 0-5 pairs.
        let hour_tens_max = if opts.hour24 { 2 } else { 1 };
        let mut pen = Pen::new(buf, area);
        for (i, &d) in digits.iter().enumerate() {
            let max = match i {
                0 => hour_tens_max,
                _ if i % 2 == 0 => 5,
                _ => 9,
            };
            let x = (i / 2) * (2 + inner + outer) + (i % 2) * (1 + inner);
            for (row, bit) in [8u8, 4, 2, 1].into_iter().enumerate() {
                if bit > max {
                    continue;
                }
                if d & bit != 0 {
                    pen.put(x, row, '●', style.main);
                } else {
                    pen.put(x, row, '○', style.dim);
                }
            }
            if form.tier == Tier::M {
                pen.put(x, 5, char::from(b'0' + d), style.dim);
            }
        }
    }
}
