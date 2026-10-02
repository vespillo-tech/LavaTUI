//! Everything that touches the terminal: layout, the keymap, and drawing
//! the [`Model`] — lamp, dock (panel, widgets on the lava, chip), status
//! bar, toasts, help and pickers. Colours only ever come from the model's
//! `Theme`.
//!
//! Draw order, back to front: background → lamp → widgets on the lava →
//! panel / chip → status bar → toast → HUD → overlay.
//!
//! Chrome never shares a cell with other chrome (§8.2): anything an open
//! overlay would cover (or touch, for the panel, the chip and the widgets
//! on the lava) is left out whole rather than clipped, and a toast
//! outranks the corner HUD.

pub(crate) mod chrome;
mod dock;
pub mod help;
pub mod keymap;
pub mod layout;
pub mod library;
pub mod picker;
#[cfg(test)]
mod render_tests;
pub mod settings;
#[cfg(test)]
mod settings_tests;
#[cfg(test)]
mod tests;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::app::{Model, Overlay};
use crate::render::{LampOptions, LampState, LampView};
use crate::theme::{Ink, Role, Theme};
use crate::ui::layout::{Layout, halo};

/// How far the liquid goes toward `accent` at the flash's peak. Under ½,
/// so depths that can't blend (no liquid tint) never flip the whole lamp.
const FLASH: f32 = 0.35;

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
    buf.set_style(
        area,
        Style::new()
            .bg(background(model))
            .fg(theme.role(Role::Text)),
    );

    // Phase-change flash (§4.5): the liquid pulses toward `accent`.
    let flashed = flashed(model);
    let lamp_theme = flashed.as_ref().unwrap_or(theme);
    if let Some(l) = layout.lamp {
        draw_lamp(frame, l, model, lamp_theme, lamp);
    }

    let buf = frame.buffer_mut();
    let covered = overlay_footprint(area, layout, model);
    let free = |r: Rect, gap: u16| covered.is_none_or(|c| !picker::grow(c, gap).intersects(r));
    for s in layout.on_lava.iter().filter(|s| free(halo(s.rect), 1)) {
        dock::draw_on_lava(buf, s, model, lamp_theme);
    }
    if let Some(p) = layout.panel.as_ref().filter(|p| free(p.rect, 1)) {
        dock::draw_panel(buf, p, model);
    }
    if let Some(c) = layout.chips.as_ref().filter(|c| free(c.rect, 1)) {
        dock::draw_chips(buf, c, model);
    }
    if let Some(s) = layout.status.filter(|&s| free(s, 0)) {
        chrome::draw_status(buf, s, model);
    }
    let toast = layout
        .toast
        .zip(model.toast.as_ref())
        .and_then(|(row, toast)| Some((chrome::toast_place(row, toast)?, toast)))
        .filter(|((r, _), _)| free(*r, 0));
    if let Some(((r, text), toast)) = &toast {
        chrome::draw_toast(buf, *r, text, toast, model);
    }
    if layout.status.is_none()
        && model.hud
        && let Some(r) = chrome::hud_corner_rect(area, model)
        && free(r, 0)
        && toast.as_ref().is_none_or(|((t, _), _)| !t.intersects(r))
    {
        chrome::draw_hud_corner(buf, r, model);
    }
    match model.overlay {
        Overlay::None => {}
        Overlay::Help { scroll } => help::draw(buf, area, scroll, model),
        Overlay::Picker(p) => picker::draw(buf, area, layout, &p, model),
        Overlay::Library(v) => library::draw(buf, area, layout, &v, model),
        Overlay::Settings(v) => settings::draw(buf, area, &v, model),
    }
}

/// The theme with the liquid mid-flash, while a phase-change flash runs.
fn flashed(model: &Model) -> Option<Theme> {
    let flash = model.flash_level();
    (flash > 0.0).then(|| {
        let theme = &model.theme;
        let liquid = theme.paint(Ink::Role(Role::Liquid));
        theme.with_role(
            Role::Liquid,
            liquid.mix(Ink::Role(Role::Accent), FLASH * flash),
        )
    })
}

/// The lamp: the field in the current style and `lamp_theme`.
fn draw_lamp(
    frame: &mut Frame,
    area: Rect,
    model: &Model,
    lamp_theme: &Theme,
    lamp: &mut LampState,
) {
    let view = LampView {
        field: &model.field,
        style: model.style.style(),
        theme: lamp_theme,
        time: model.time(),
        options: LampOptions {
            reduced: model.quality.reduced_grid(),
            translucent: model.translucent_cells(),
        },
    };
    frame.render_stateful_widget(view, area, lamp);
}

/// The app background chrome paints on: `bg`, or nothing when the theme
/// is transparent (§9).
fn background(model: &Model) -> Color {
    model.theme.background(model.settings.theme.transparent)
}

/// The cells the open overlay takes, if any.
pub fn overlay_footprint(area: Rect, layout: &Layout, model: &Model) -> Option<Rect> {
    match model.overlay {
        Overlay::None => None,
        Overlay::Help { .. } => help::footprint(area),
        Overlay::Picker(p) => picker::placement(area, layout, &p).map(|p| p.footprint()),
        Overlay::Library(v) => library::placement(area, layout, &v, model).map(|p| p.footprint()),
        Overlay::Settings(v) => settings::footprint(area, &v),
    }
}
