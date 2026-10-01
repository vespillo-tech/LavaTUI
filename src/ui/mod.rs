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
use crate::theme::{Ink, Role};

/// How far the bleed liquid goes toward `accent` at the flash's peak. Under
/// ½, so depths that can't blend (no liquid tint) never flip the whole tank.
const BLEED_FLASH: f32 = 0.35;

/// Draw one frame. `lamp` is the lamp's scratch state, kept by the caller.
///
/// Everything is placed by a layout for `frame.area()`, the size actually
/// being drawn. The loop ticks the model at that size, so this is normally
/// `model.layout`; if the two ever disagree (the terminal resized between
/// tick and draw) the layout is recomputed rather than drawing stale rects
/// outside the buffer.
pub fn draw(frame: &mut Frame, model: &Model, lamp: &mut LampState) {
    let area = frame.area();
    let fresh;
    let layout = if model.layout.area == area {
        &model.layout
    } else {
        fresh = model.layout_for(area);
        &fresh
    };
    let theme = &model.theme;
    let buf = frame.buffer_mut();
    let bg = if model.settings.theme.transparent {
        Color::Reset
    } else {
        theme.role(Role::Bg)
    };
    buf.set_style(area, Style::new().bg(bg).fg(theme.role(Role::Text)));

    if let Some(l) = layout.lamp {
        // Phase-change flash (§4.5): the glass flashes its metal; in bleed
        // there's no metal, so the liquid pulses toward `accent` instead.
        let flash = model.flash_level();
        let flashed;
        let lamp_theme = if l.glass.is_none() && flash > 0.0 {
            let liquid = theme.paint(Ink::Role(Role::Liquid));
            flashed = theme.with_role(
                Role::Liquid,
                liquid.mix(Ink::Role(Role::Accent), BLEED_FLASH * flash),
            );
            &flashed
        } else {
            theme
        };
        let view = LampView {
            field: &model.field,
            style: model.style.style(),
            theme: lamp_theme,
            time: model.time(),
            lighting: model
                .settings
                .lamp
                .lighting
                .then_some(&Lamplight as &dyn Lighting),
        };
        frame.render_stateful_widget(view, l.view, lamp);
        if let Some(g) = l.glass {
            glass::draw(
                frame.buffer_mut(),
                &l,
                g,
                theme,
                flash,
                model.settings.lamp.lighting,
            );
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
        Overlay::Picker(p) => picker::draw(buf, area, layout, &p, model),
    }
}
