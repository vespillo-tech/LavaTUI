//! `analog`: a braille clock dial. Braille dots are square on a 1:2 cell
//! (2×4 dots per cell), so the dial is a true circle. Ticks are dim, hands
//! are `main`; a cell holding any hand dot is drawn in `main`.

use std::f64::consts::TAU;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::draw::{Bitmap, Pen, braille_char};
use super::{ClockTime, Face, FaceOptions, FaceStyle, Form, Tier, readout_forms};

pub struct Analog;

fn size(tier: Tier, _seconds: bool, _meridiem: bool) -> (usize, usize) {
    match tier {
        Tier::XL => (31, 16),
        Tier::L => (23, 12),
        _ => (15, 8),
    }
}

/// Point at `frac` of a turn clockwise from 12, `len` dots from `c`.
fn polar(c: (f64, f64), frac: f64, len: f64) -> (i32, i32) {
    let a = frac * TAU;
    (
        (c.0 + len * a.sin()).round() as i32,
        (c.1 - len * a.cos()).round() as i32,
    )
}

impl Face for Analog {
    fn name(&self) -> &'static str {
        "analog"
    }

    fn forms(&self, opts: FaceOptions) -> Vec<Form> {
        // A dial has no am/pm, so only seconds (the second hand) varies.
        let opts = FaceOptions {
            hour24: true,
            ..opts
        };
        readout_forms(opts, &[Tier::XL, Tier::L, Tier::M], size)
    }

    fn draw(
        &self,
        form: Form,
        time: ClockTime,
        _opts: FaceOptions,
        area: Rect,
        buf: &mut Buffer,
        style: FaceStyle,
    ) {
        let (cols, rows) = size(form.tier, false, false);
        let (w, h) = (cols * 2, rows * 4);
        let c = ((w - 1) as f64 / 2.0, (h - 1) as f64 / 2.0);
        let r = (w.min(h) as f64 / 2.0) - 1.0;

        let mut ticks = Bitmap::new(w, h);
        for i in 0..12 {
            let frac = f64::from(i) / 12.0;
            let inner = if i % 3 == 0 { 0.8 } else { 0.92 };
            ticks.line(polar(c, frac, r * inner), polar(c, frac, r));
        }

        let (h12, m, s) = (
            f64::from(time.hour % 12),
            f64::from(time.minute),
            f64::from(time.second),
        );
        let centre = (c.0.round() as i32, c.1.round() as i32);
        let mut hands = Bitmap::new(w, h);
        let hour_tip = polar(c, (h12 + m / 60.0) / 12.0, r * 0.5);
        // The hour hand is two dots thick so it reads apart from the minute hand.
        for (dx, dy) in [(0, 0), (1, 0), (0, 1)] {
            hands.line(
                (centre.0 + dx, centre.1 + dy),
                (hour_tip.0 + dx, hour_tip.1 + dy),
            );
        }
        hands.line(centre, polar(c, (m + s / 60.0) / 60.0, r * 0.8));
        if form.seconds {
            hands.line(centre, polar(c, s / 60.0, r * 0.9));
        }

        let mut pen = Pen::new(buf, area);
        for cy in 0..rows {
            for cx in 0..cols {
                let (hb, tb) = (hands.braille_bits(cx, cy), ticks.braille_bits(cx, cy));
                if hb != 0 {
                    pen.put(cx, cy, braille_char(hb | tb), style.main);
                } else if tb != 0 {
                    pen.put(cx, cy, braille_char(tb), style.dim);
                }
            }
        }
    }
}
