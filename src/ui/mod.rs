//! Everything that touches the terminal: layout, modes (full / minimal),
//! keymap and overlays.

mod input;
mod placeholder;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;

pub use input::{Action, action_for};

/// What one frame needs to know. Built by the app loop each frame.
#[derive(Debug, Clone, Copy)]
pub struct Scene {
    /// Simulated seconds, interpolated to the moment of drawing.
    pub time: f64,
    /// Measured render fps.
    pub fps: f64,
    pub minimal: bool,
}

pub fn draw(frame: &mut Frame, scene: &Scene) {
    let area = frame.area();
    frame.render_widget(placeholder::Gradient { time: scene.time }, area);
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
