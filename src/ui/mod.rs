//! Everything that touches the terminal: layout, the keymap, and drawing
//! the [`Model`] — lamp, glass, panel, chip, status bar, toasts, help and
//! pickers. Colours only ever come from the model's `Theme`.
//!
//! Draw order, back to front: background → lamp (+ glass) → panel / chip →
//! status bar → toast → overlay.

mod chrome;
mod glass;
pub mod help;
pub mod keymap;
pub mod layout;
mod panel;
mod picker;
#[cfg(test)]
mod tests;

use ratatui::Frame;
use ratatui::style::{Color, Style};

use crate::app::{Model, Overlay};
use crate::light::{Lamplight, Lighting};
use crate::render::{LampState, LampView};
use crate::theme::Role;

/// Draw one frame. `lamp` is the lamp's scratch state, kept by the caller.
pub fn draw(frame: &mut Frame, model: &Model, lamp: &mut LampState) {
    let area = frame.area();
    let layout = &model.layout;
    let theme = &model.theme;
    let buf = frame.buffer_mut();
    let bg = if model.settings.theme.transparent {
        Color::Reset
    } else {
        theme.role(Role::Bg)
    };
    buf.set_style(area, Style::new().bg(bg).fg(theme.role(Role::Text)));

    if let Some(l) = layout.lamp {
        let view = LampView {
            field: &model.field,
            style: model.style.style(),
            theme,
            time: model.time(),
            lighting: model
                .settings
                .lamp
                .lighting
                .then_some(&Lamplight as &dyn Lighting),
        };
        frame.render_stateful_widget(view, l.view, lamp);
        if let Some(g) = l.glass {
            glass::draw(frame.buffer_mut(), &l, g, model);
        }
    }

    let buf = frame.buffer_mut();
    if let Some(p) = &layout.panel {
        panel::draw_panel(buf, p, model);
    }
    if let Some(c) = &layout.chip {
        panel::draw_chip(buf, c, model);
    }
    if let Some(s) = layout.status {
        chrome::draw_status(buf, s, model);
    } else if model.hud {
        chrome::draw_hud_corner(buf, area, model);
    }
    if let (Some(t), Some(toast)) = (layout.toast, &model.toast) {
        chrome::draw_toast(buf, t, toast, model);
    }
    match model.overlay {
        Overlay::None => {}
        Overlay::Help { scroll } => help::draw(buf, area, scroll, model),
        Overlay::Picker(p) => picker::draw(buf, area, &p, model),
    }
}
