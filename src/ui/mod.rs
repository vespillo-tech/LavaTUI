//! Everything that touches the terminal: layout, modes (full / minimal),
//! keymap and overlays.

mod input;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

pub use input::{Action, action_for};

use crate::render::{LampState, LampView, StyleId};
use crate::sim::Field;
use crate::theme::{Role, Theme};

/// Terminal cells are about twice as tall as wide (docs/design.md §2.3).
const CELL_ASPECT: f64 = 2.0;

/// What one frame needs to know. Built by the app loop each frame.
#[derive(Debug, Clone, Copy)]
pub struct Scene<'a> {
    /// The wax, interpolated to the moment of drawing.
    pub field: &'a Field,
    pub style: StyleId,
    pub theme: &'a Theme,
    /// Seconds since launch.
    pub time: f64,
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

/// `lamp` is the lamp's scratch state, kept by the caller across frames.
pub fn draw(frame: &mut Frame, scene: &Scene, lamp: &mut LampState) {
    let area = frame.area();
    let view = LampView {
        field: scene.field,
        style: scene.style.style(),
        theme: scene.theme,
        time: scene.time,
        lighting: None,
    };
    frame.render_stateful_widget(view, area, lamp);
    if !scene.minimal {
        draw_status(frame, area, scene);
    }
}

/// One-line hint in the bottom-right corner; dropped when it would not fit.
fn draw_status(frame: &mut Frame, area: Rect, scene: &Scene) {
    let text = format!(
        " {} · {}  {:>3.0} fps · s style · p palette · q quit ",
        scene.style.style().name(),
        scene.theme.palette().name,
        scene.fps
    );
    let width = text.chars().count() as u16;
    if area.width < width || area.height < 2 {
        return;
    }
    let row = Rect::new(area.right() - width, area.bottom() - 1, width, 1);
    let style = Style::new()
        .fg(scene.theme.role(Role::Dim))
        .bg(scene.theme.role(Role::Liquid));
    frame.render_widget(Line::styled(text, style), row);
}
