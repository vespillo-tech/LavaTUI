//! The clock: the chosen face (§4.5), with the date line under it in tall
//! terminals, and `14:32` as its chip.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};

use super::{Backdrop, ChipText, DockWidget, Look, Needs, Place, WidgetForm, align_x};
use crate::app::Model;
use crate::clock::{self, Face, FaceOptions, FaceStyle, Form, Text, Tier};
use crate::config::MinimalClock;
use crate::theme::Role;

pub struct Clock;

/// Widest date line (`wed 30 sep`).
const DATE_W: u16 = 10;
/// Rows the date line adds under the face: a blank one, then the date.
const DATE_ROWS: u16 = 2;

/// The clock's variants in `place`, most preferred first: each face form
/// with the date line, then without (the date goes before the face
/// shrinks, §1.3). Seconds only in L/XL, and never on the lava, where
/// they'd be the one thing ticking over the wax; never with
/// `opts.seconds` off (`clock.seconds = false`).
fn variants(face: &dyn Face, opts: FaceOptions, place: Place) -> Vec<(Form, bool)> {
    face.all_forms(opts)
        .into_iter()
        .filter(|f| !f.seconds || (f.tier >= Tier::L && place == Place::Side))
        .flat_map(|f| [(f, true), (f, false)])
        .collect()
}

/// The clock's forms for `face` in `place` (pure, for the layout tests).
pub fn clock_forms(face: &dyn Face, opts: FaceOptions, place: Place) -> Vec<WidgetForm> {
    variants(face, opts, place)
        .into_iter()
        .enumerate()
        .map(|(i, (f, date))| {
            let (w, h) = (f.size.width, f.size.height);
            let mut form = if date {
                WidgetForm::fill(w.max(DATE_W), h + DATE_ROWS, i as u16)
            } else {
                WidgetForm::fixed(w, h, i as u16)
            };
            form.needs = Needs {
                huge: f.tier == Tier::XL,
                tall: date,
            };
            form.seconds = f.seconds;
            form
        })
        .collect()
}

/// Where the parts of clock form `form` go in `area`: the face form and
/// its rect, and the date line's rect, if the form has one.
pub fn clock_parts(
    face: &dyn Face,
    opts: FaceOptions,
    place: Place,
    form: WidgetForm,
    area: Rect,
    align: Alignment,
) -> Option<(Form, Rect, Option<Rect>)> {
    let &(f, date) = variants(face, opts, place).get(usize::from(form.variant))?;
    let (w, h) = (f.size.width, f.size.height);
    if w > area.width || h > area.height {
        return None;
    }
    let face_rect = Rect::new(area.x + align_x(align, area.width, w), area.y, w, h);
    let date = (date && h + DATE_ROWS <= area.height)
        .then(|| Rect::new(area.x, area.y + h + 1, area.width, 1));
    Some((f, face_rect, date))
}

fn place_of(look: Look) -> Place {
    match look.backdrop {
        Backdrop::Panel => Place::Side,
        Backdrop::Lava => Place::Overlay,
    }
}

impl DockWidget for Clock {
    fn name(&self) -> &'static str {
        "clock"
    }

    fn forms(&self, model: &Model, place: Place) -> Vec<WidgetForm> {
        clock_forms(model.face, model.clock_options(), place)
    }

    fn draw(&self, model: &Model, form: WidgetForm, area: Rect, look: Look, buf: &mut Buffer) {
        let opts = model.clock_options();
        let place = place_of(look);
        let Some((f, rect, date)) = clock_parts(model.face, opts, place, form, area, look.align)
        else {
            return;
        };
        let theme = &model.theme;
        let face = if f.tier == Tier::Text {
            &Text
        } else {
            model.face
        };
        let style = FaceStyle {
            main: theme.text(Role::Text),
            dim: theme.text(Role::Dim),
        };
        face.draw(f, model.local.time, opts, rect, buf, style);
        if let Some(r) = date {
            let text = &model.local.date;
            let w = (text.chars().count() as u16).min(r.width);
            let x = r.x + align_x(look.align, r.width, w);
            buf.set_stringn(x, r.y, text, usize::from(w), theme.text(Role::Dim));
        }
    }

    /// `14:32` (` 2:32 pm`); none in minimal mode with `minimal.clock = "off"`.
    fn chip(&self, model: &Model) -> Option<ChipText> {
        let s = &model.settings;
        if model.minimal() && s.minimal.clock == MinimalClock::Off {
            return None;
        }
        let hour24 = s.clock.hour24;
        Some(ChipText {
            text: clock::readout(model.local.time, hour24, false, !hour24),
            ink: Role::Text,
        })
    }
}
