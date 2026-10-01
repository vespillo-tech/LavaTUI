//! Live-preview pickers for style / face / palette (§4.4). Moving the
//! cursor applies the item at once (the model does that); this only draws.
//!
//! * Roomy: a right-anchored sheet, 26 wide; the lamp stays un-dimmed so
//!   you can watch it change.
//! * Small: a bottom sheet across the full width, up to half the height.
//! * Tiny / micro: an inline `‹ braille ›` selector in the top row.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Widget};

use crate::app::{Model, Picker};
use crate::theme::Role;
use crate::ui::layout::{Layout, SizeTier, margins};

const SHEET_W: u16 = 26;

/// Draw `picker` over `area`, which `layout` was made for.
pub fn draw(buf: &mut Buffer, area: Rect, layout: &Layout, picker: &Picker, model: &Model) {
    if area.is_empty() {
        return;
    }
    let theme = &model.theme;
    let items = picker.kind.items();
    let bg = Style::new().bg(theme.role(Role::Bg));
    let (text, dim, accent) = (
        theme.text(Role::Text),
        theme.text(Role::Dim),
        theme.text(Role::Accent),
    );

    // From the drawn area, not the model's: they differ mid-resize.
    if SizeTier::of(area) <= SizeTier::Tiny {
        let name = items[picker.cursor];
        let s = format!(" ‹ {name} › ");
        let w = s.chars().count() as u16;
        if w <= area.width {
            let x = area.x + (area.width - w) / 2;
            buf.set_string(x, area.y, s, accent.patch(bg));
        }
        return;
    }

    let n = items.len() as u16;
    let status = u16::from(layout.status.is_some());
    let (sheet, list, hint_y) = if area.width >= 80 && area.height >= 16 {
        let h = (n + 6).min(area.height - 2);
        let (_, hm) = margins(area.width, area.height);
        let x = area.right().saturating_sub(hm + SHEET_W);
        let y = area.y + (area.height - status - h) / 2;
        let sheet = Rect::new(x, y, SHEET_W, h);
        let list = Rect::new(x + 1, y + 2, SHEET_W - 2, h - 6);
        (sheet, list, Some(y + h - 3))
    } else {
        let h = (n + 3).min((area.height - status) / 2).max(3);
        let y = area.bottom() - status - h;
        let sheet = Rect::new(area.x, y, area.width, h);
        (
            sheet,
            Rect::new(area.x + 1, y + 1, area.width - 2, h - 2),
            None,
        )
    };

    Clear.render(sheet, buf);
    buf.set_style(sheet, bg);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.role(Role::Metal)))
        .title(Line::styled(format!(" {} ", picker.kind.title()), accent))
        .render(sheet, buf);

    // Scroll so the cursor stays in view.
    let rows = usize::from(list.height);
    let first = picker.cursor.saturating_sub(rows.saturating_sub(1));
    for (row, (i, name)) in items.iter().enumerate().skip(first).take(rows).enumerate() {
        let y = list.y + row as u16;
        let (marker, style) = if i == picker.cursor {
            ("▸ ", accent)
        } else {
            ("  ", text)
        };
        let line = format!(" {marker}{name}");
        let w = usize::from(list.width);
        buf.set_stringn(list.x, y, &line, w, style);
        if i == picker.original {
            let x = list.x + line.chars().count() as u16 + 1;
            if x < list.right() {
                buf.set_string(x, y, "·", dim);
            }
        }
    }
    if let Some(y) = hint_y {
        buf.set_string(list.x + 1, y, "⏎ keep   esc revert", dim);
    }
}
