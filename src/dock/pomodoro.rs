//! The pomodoro (§4.5): label and cycle dots, the time left, the progress
//! bar; `▸ 18:24` as its chip while one runs.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use super::{ChipText, DockWidget, Look, Place, WidgetForm, align_x};
use crate::app::Model;
use crate::clock::{PomodoroStyle, PomodoroWidget, Status, format_remaining};
use crate::theme::Role;

pub struct Pomodoro;

/// The full readout: label + dots, time, bar. In the panel it takes the
/// panel's width (at least 20); on the lava it's 22 (room for the dots) or
/// as wide as what's stacked with it.
const FULL: (u16, u16) = (22, 3);
const SIDE_MIN_W: u16 = 20;
/// Time and bar, no label.
const SHORT: (u16, u16) = (12, 2);

/// The pomodoro's forms in `place`; `time_w` is the width of the longest
/// time it can show (`25:00`, `120:00`), for the one-row form.
pub fn pomodoro_forms(place: Place, time_w: u16) -> Vec<WidgetForm> {
    match place {
        Place::Side => vec![WidgetForm::fill(SIDE_MIN_W, FULL.1, 0)],
        _ => vec![
            WidgetForm::fill(FULL.0, FULL.1, 0),
            WidgetForm::fill(SHORT.0, SHORT.1, 1),
            // `▸ 18:24`, the chip's look without its pads.
            WidgetForm::fixed(time_w + 2, 1, 2),
        ],
    }
}

pub fn pomodoro_style(model: &Model) -> PomodoroStyle {
    let theme = &model.theme;
    PomodoroStyle {
        text: theme.text(Role::Text),
        dim: theme.text(Role::Dim),
        focus: theme.text(Role::Accent),
        rest: theme.text(Role::WaxHot),
    }
}

impl DockWidget for Pomodoro {
    fn name(&self) -> &'static str {
        "pomodoro"
    }

    fn forms(&self, model: &Model, place: Place) -> Vec<WidgetForm> {
        let config = model.pomodoro.config();
        let longest = [config.focus, config.short_break, config.long_break]
            .into_iter()
            .max()
            .unwrap_or_default();
        let time_w = format_remaining(longest).chars().count() as u16;
        pomodoro_forms(place, time_w)
    }

    fn draw(&self, model: &Model, form: WidgetForm, area: Rect, look: Look, buf: &mut Buffer) {
        // The one-row form is fixed-width: line it up like the others.
        let w = if form.fill {
            area.width
        } else {
            form.size.width.min(area.width)
        };
        let rect = Rect {
            x: area.x + align_x(look.align, area.width, w),
            width: w,
            ..area
        };
        PomodoroWidget::new(&model.pomodoro, model.now)
            .style(pomodoro_style(model))
            .render(rect, buf);
    }

    /// Running 3 (above everything), paused 2, idle 0 (below the clock).
    fn rank(&self, model: &Model) -> u8 {
        match model.pomodoro.status() {
            Status::Running => 3,
            Status::Paused => 2,
            Status::Idle => 0,
        }
    }

    /// `▸ 24:58` (focus), `▸ break 4:58`, `‖` when paused; nothing idle.
    fn chip(&self, model: &Model) -> Option<ChipText> {
        let p = &model.pomodoro;
        let (glyph, ink) = match p.status() {
            Status::Running if p.phase().is_break() => ('▸', Role::WaxHot),
            Status::Running => ('▸', Role::Accent),
            Status::Paused => ('‖', Role::Text),
            Status::Idle => return None,
        };
        let remaining = format_remaining(p.remaining(model.now));
        // A break says so: phase colours alone can be near twins (or, in
        // 16 colours and none, the same).
        let phase = if p.phase().is_break() { "break " } else { "" };
        Some(ChipText {
            text: format!("{glyph} {phase}{remaining}"),
            ink,
        })
    }
}
