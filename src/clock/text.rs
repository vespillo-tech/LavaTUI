//! `text`: the plain readout (`14:32`, `14:32:07`, ` 2:32 pm`). Also the
//! fallback every other face degrades to, so its smallest form (5×1) is the
//! floor for all of them.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::draw::{Pen, meridiem};
use super::{ClockTime, Face, FaceOptions, FaceStyle, Form, Tier, readout_forms};

pub struct Text;

/// The readout string for a form. Fixed width per form: 12h hours are
/// space-padded so the colon never moves.
pub fn readout(time: ClockTime, hour24: bool, seconds: bool, meridiem_on: bool) -> String {
    let hour = time.display_hour(hour24);
    let mut s = if hour24 {
        format!("{hour:02}:{:02}", time.minute)
    } else {
        format!("{hour:>2}:{:02}", time.minute)
    };
    if seconds {
        s += &format!(":{:02}", time.second);
    }
    if meridiem_on {
        s += meridiem(time);
    }
    s
}

impl Face for Text {
    fn name(&self) -> &'static str {
        "text"
    }

    fn forms(&self, opts: FaceOptions) -> Vec<Form> {
        readout_forms(opts, &[Tier::Text], |_, seconds, meridiem| {
            let probe = ClockTime::from_secs_of_day(0);
            (
                readout(probe, opts.hour24, seconds, meridiem)
                    .chars()
                    .count(),
                1,
            )
        })
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
        let mut pen = Pen::new(buf, area);
        let s = readout(time, opts.hour24, form.seconds, false);
        pen.text(0, 0, &s, style.main);
        if form.meridiem {
            pen.text(s.chars().count(), 0, meridiem(time), style.dim);
        }
    }
}
