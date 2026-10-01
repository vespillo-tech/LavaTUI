//! Everything that touches the terminal: layout, modes (full / minimal),
//! keymap and overlays.

mod input;
mod placeholder;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;

pub use input::{Action, action_for};

use crate::sim::{Field, Sample};

/// Terminal cells are about twice as tall as wide (docs/design.md §2.3).
const CELL_ASPECT: f64 = 2.0;

/// What one frame needs to know. Built by the app loop each frame.
#[derive(Debug, Clone, Copy)]
pub struct Scene<'a> {
    /// The wax, interpolated to the moment of drawing.
    pub field: &'a Field,
    /// Measured render fps.
    pub fps: f64,
    pub minimal: bool,
}

/// Visual aspect (on-screen width ÷ height) of a `cols × rows` lamp.
pub fn lamp_aspect(cols: u16, rows: u16) -> f64 {
    if cols == 0 || rows == 0 {
        return 1.0;
    }
    f64::from(cols) / (f64::from(rows) * CELL_ASPECT)
}

/// `samples` is a scratch buffer the caller keeps across frames.
pub fn draw(frame: &mut Frame, scene: &Scene, samples: &mut Vec<Sample>) {
    let area = frame.area();
    let wax = placeholder::WaxView {
        field: scene.field,
        samples,
    };
    frame.render_widget(wax, area);
    if !scene.minimal {
        draw_status(frame, area, scene);
    }
}

/// One-line hint in the bottom-right corner; dropped when it would not fit.
fn draw_status(frame: &mut Frame, area: Rect, scene: &Scene) {
    let text = format!(" {:>3.0} fps · q quit ", scene.fps);
    let width = text.chars().count() as u16;
    if area.width < width || area.height < 2 {
        return;
    }
    let row = Rect::new(area.right() - width, area.bottom() - 1, width, 1);
    let style = Style::new()
        .fg(Color::Rgb(255, 214, 170))
        .bg(Color::Rgb(24, 8, 20));
    frame.render_widget(Line::styled(text, style), row);
}
