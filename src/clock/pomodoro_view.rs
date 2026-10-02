//! The compact pomodoro readout (design.md §4.5), degrading with size:
//!
//! ```text
//! focus             ●●○○    3 rows, width ≥ 18 (dots need ≥ 22)
//! 18:24
//! ━━━━━━━━──────────────
//!
//! 18:24                     2 rows: time + bar
//! ━━━━━━━━──────
//!
//! ▸ 18:24                   1 row: the chip (‖ when paused)
//! ```
//!
//! Below the width of the time itself it draws nothing.

use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::{Rect, Size};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::draw::Pen;
use super::pomodoro::{Pomodoro, Status, format_remaining};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PomodoroStyle {
    pub text: Style,
    pub dim: Style,
    /// Accent while a focus phase runs (palette `accent`).
    pub focus: Style,
    /// Accent while a break runs (palette `wax_hot`).
    pub rest: Style,
}

impl Default for PomodoroStyle {
    fn default() -> Self {
        Self {
            text: Style::new(),
            dim: Style::new().add_modifier(Modifier::DIM),
            focus: Style::new().add_modifier(Modifier::BOLD),
            rest: Style::new().add_modifier(Modifier::BOLD),
        }
    }
}

/// Width at which the phase label appears, and the cycle dots.
const LABEL_MIN_W: u16 = 18;
const DOTS_MIN_W: u16 = 22;

pub struct PomodoroWidget<'a> {
    pomodoro: &'a Pomodoro,
    now: Instant,
    style: PomodoroStyle,
    /// The one-row form's mark while running and while paused.
    marks: (&'a str, &'a str),
}

impl<'a> PomodoroWidget<'a> {
    pub fn new(pomodoro: &'a Pomodoro, now: Instant) -> Self {
        Self {
            pomodoro,
            now,
            style: PomodoroStyle::default(),
            marks: ("▸", "‖"),
        }
    }

    /// The marks for running and paused (a terminal that can't draw `▸`
    /// and `‖` gets ASCII ones).
    pub fn marks(mut self, running: &'a str, paused: &'a str) -> Self {
        self.marks = (running, paused);
        self
    }

    pub fn style(mut self, style: PomodoroStyle) -> Self {
        self.style = style;
        self
    }

    /// The full three-row layout with dots.
    pub fn preferred_size() -> Size {
        Size::new(DOTS_MIN_W, 3)
    }

    /// Smallest area that shows anything (the bare time, `25:00`).
    pub fn min_size() -> Size {
        Size::new(5, 1)
    }

    /// Ink for the live parts: phase accent while running, else text/dim.
    fn live(&self) -> Style {
        match self.pomodoro.status() {
            Status::Running if self.pomodoro.phase().is_break() => self.style.rest,
            Status::Running => self.style.focus,
            Status::Paused => self.style.text,
            Status::Idle => self.style.dim,
        }
    }

    fn bar(&self, pen: &mut Pen, y: usize, width: usize) {
        let filled = if self.pomodoro.status() == Status::Idle {
            0
        } else {
            (self.pomodoro.progress(self.now) * width as f64).round() as usize
        };
        for x in 0..width {
            if x < filled {
                pen.put(x, y, '━', self.live());
            } else {
                pen.put(x, y, '─', self.style.dim);
            }
        }
    }
}

impl Widget for PomodoroWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

impl Widget for &PomodoroWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let p = self.pomodoro;
        let time = format_remaining(p.remaining(self.now));
        let time_w = time.chars().count();
        let (w, h) = (usize::from(area.width), area.height);
        if area.is_empty() || w < time_w {
            return;
        }
        let mut pen = Pen::new(buf, area);

        if h == 1 {
            let glyph = match p.status() {
                Status::Running => Some(self.marks.0),
                Status::Paused => Some(self.marks.1),
                Status::Idle => None,
            };
            match glyph.map(|g| (g, g.width())) {
                Some((g, gw)) if gw > 0 && w > time_w + gw => {
                    pen.text(0, 0, g, self.live());
                    pen.text(gw + 1, 0, &time, self.live());
                }
                _ => pen.text(0, 0, &time, self.live()),
            }
            return;
        }

        let mut y = 0;
        if h >= 3 && area.width >= LABEL_MIN_W {
            pen.text(0, 0, p.phase().label(), self.style.dim);
            let (done, of) = p.set_progress();
            let dots = of as usize;
            let label_w = p.phase().label().len();
            if area.width >= DOTS_MIN_W && label_w + 2 + dots <= w {
                for i in 0..dots {
                    let (ch, style) = if (i as u32) < done {
                        ('●', self.style.focus)
                    } else {
                        ('○', self.style.dim)
                    };
                    pen.put(w - dots + i, 0, ch, style);
                }
            }
            y = 1;
        }
        pen.text(0, y, &time, self.live());
        if p.status() == Status::Paused && time_w + 2 + "paused".len() <= w {
            pen.text(time_w + 2, y, "paused", self.style.dim);
        }
        self.bar(&mut pen, y + 1, w);
    }
}
