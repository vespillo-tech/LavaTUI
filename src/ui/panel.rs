//! The clock + pomodoro panel (§4.5) and its one-line fallback, the chip.

use ratatui::buffer::Buffer;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use crate::app::Model;
use crate::clock::{FaceStyle, PomodoroStyle, PomodoroWidget, Status, Text, Tier};
use crate::theme::Role;
use crate::ui::layout::{Chip, ChipKind, Panel};

pub fn draw_panel(buf: &mut Buffer, panel: &Panel, model: &Model) {
    let theme = &model.theme;
    if let Some((rect, form)) = panel.face {
        let face = if form.tier == Tier::Text {
            &Text
        } else {
            model.face
        };
        let style = FaceStyle {
            main: theme.text(Role::Text),
            dim: theme.text(Role::Dim),
        };
        face.draw(
            form,
            model.local.time,
            model.face_options(true),
            rect,
            buf,
            style,
        );
    }
    if let Some(rect) = panel.date {
        buf.set_stringn(
            rect.x,
            rect.y,
            &model.local.date,
            usize::from(rect.width),
            theme.text(Role::Dim),
        );
    }
    PomodoroWidget::new(&model.pomodoro, model.now)
        .style(pomodoro_style(model))
        .render(panel.pomodoro, buf);
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

/// ` 14:32 ` / ` ▸ 18:24 ` with a 1-cell `bg` pad, over the lamp.
pub fn draw_chip(buf: &mut Buffer, chip: &Chip, model: &Model) {
    let Some((kind, text)) = model.chip_text() else {
        return;
    };
    let theme = &model.theme;
    let ink = match kind {
        ChipKind::Pomodoro => match model.pomodoro.status() {
            Status::Running if model.pomodoro.phase().is_break() => Role::WaxHot,
            Status::Running => Role::Accent,
            _ => Role::Text,
        },
        ChipKind::Clock if chip.under => Role::Dim,
        ChipKind::Clock => Role::Text,
    };
    let bg = Style::new().bg(theme.role(Role::Bg));
    let r = chip.rect;
    let w = text.chars().count() as u16;
    if w > r.width {
        return;
    }
    if !chip.under {
        buf.set_string(r.x, r.y, " ".repeat(usize::from(r.width)), bg);
    }
    let x = r.x + (r.width - w) / 2;
    buf.set_string(x, r.y, &text, theme.text(ink).patch(bg));
}
