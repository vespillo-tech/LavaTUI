//! Live-preview pickers for style / face / palette (§4.4). Moving the
//! cursor applies the item at once (the model does that); this draws, and
//! answers where things are ([`placement`], [`hit`]) for the mouse and for
//! keeping the rest of the chrome out from under the sheet.
//!
//! * Roomy: a 26-wide sheet. The style and palette pickers sit on the
//!   right (over the panel; the lamp stays un-dimmed so you can watch it
//!   change); the face picker sits on the left when that keeps the panel
//!   clear, so the face it previews stays in view.
//! * Small: a bottom sheet, up to half the height, across the lamp's
//!   columns (clear of a right panel) or the full width.
//! * Tiny / micro: an inline `‹ braille ›` selector in the top row, which
//!   sheds its pads, then its arrows, then letters, so it always shows.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Widget};

use crate::app::{Model, Picker, PickerKind};
use crate::theme::Role;
use crate::ui::chrome::PICKER_HINTS;
use crate::ui::layout::{Layout, SizeTier, margins};

const SHEET_W: u16 = 26;
/// Narrowest bottom sheet that still reads (border + `▸ ` + a name).
const MIN_BOTTOM_W: u16 = 16;

/// Where an open picker goes at a given size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// The one-line selector: its text and the rect it's drawn in.
    Inline { rect: Rect, text: InlineText },
    Sheet {
        sheet: Rect,
        /// The item rows.
        list: Rect,
        /// The `⏎ keep  esc revert` row, roomy sheets only.
        hint: Option<u16>,
    },
}

impl Placement {
    /// Every cell the picker draws on.
    pub fn footprint(&self) -> Rect {
        match *self {
            Placement::Inline { rect, .. } => rect,
            Placement::Sheet { sheet, .. } => sheet,
        }
    }
}

/// How much of ` ‹ name › ` the inline selector shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineText {
    /// ` ‹ name › `
    Padded,
    /// `‹ name ›`
    Arrows,
    /// `name`, cut to this many chars (the last one `…` when cut).
    Name(u16),
}

impl InlineText {
    /// Fit `name` in `width` columns: drop the pads, then the arrows, then
    /// letters. `None` only when there are no columns at all.
    pub fn fit(name: &str, width: u16) -> Option<Self> {
        let n = name.chars().count() as u16;
        if n + 6 <= width {
            Some(InlineText::Padded)
        } else if n + 4 <= width {
            Some(InlineText::Arrows)
        } else if width > 0 {
            Some(InlineText::Name(n.min(width)))
        } else {
            None
        }
    }

    pub fn render(self, name: &str) -> String {
        match self {
            InlineText::Padded => format!(" ‹ {name} › "),
            InlineText::Arrows => format!("‹ {name} ›"),
            InlineText::Name(w) => {
                let w = usize::from(w);
                if name.chars().count() <= w {
                    name.to_owned()
                } else {
                    let cut: String = name.chars().take(w.saturating_sub(1)).collect();
                    format!("{cut}…")
                }
            }
        }
    }
}

/// Where `picker` goes in `area`, which `layout` was made for.
pub fn placement(area: Rect, layout: &Layout, picker: &Picker) -> Option<Placement> {
    if area.is_empty() {
        return None;
    }
    let items = picker.kind.items();
    // From the drawn area, not the model's: they differ mid-resize.
    if SizeTier::of(area) <= SizeTier::Tiny {
        let name = items[picker.cursor];
        let text = InlineText::fit(name, area.width)?;
        let w = text.render(name).chars().count() as u16;
        let rect = Rect::new(area.x + (area.width - w) / 2, area.y, w, 1);
        return Some(Placement::Inline { rect, text });
    }

    let n = items.len() as u16;
    let status = u16::from(layout.status.is_some());
    if area.width >= 80 && area.height >= 16 {
        let h = (n + 6).min(area.height - 2);
        let (_, hm) = margins(area.width, area.height);
        let y = area.y + (area.height - status - h) / 2;
        let right = Rect::new(area.right().saturating_sub(hm + SHEET_W), y, SHEET_W, h);
        let left = Rect::new(area.x + hm, y, SHEET_W, h);
        // The face picker previews the panel: keep the sheet off it when
        // the other side is free.
        let clear_of_panel = |r: Rect| layout.panel.is_none_or(|p| !grow(p.rect, 1).intersects(r));
        let sheet = if picker.kind == PickerKind::Face && clear_of_panel(left) {
            left
        } else {
            right
        };
        let list = Rect::new(sheet.x + 1, y + 2, SHEET_W - 2, h - 6);
        return Some(Placement::Sheet {
            sheet,
            list,
            hint: Some(y + h - 3),
        });
    }

    let h = (n + 3).min((area.height - status) / 2).max(3);
    let y = area.bottom() - status - h;
    // Across the lamp only, when a panel sits to its right (its own
    // padding column keeps them apart).
    let w = match layout.panel {
        Some(p) if p.rect.x >= area.x + MIN_BOTTOM_W && p.rect.bottom() > y => p.rect.x - area.x,
        _ => area.width,
    };
    let sheet = Rect::new(area.x, y, w, h);
    Some(Placement::Sheet {
        sheet,
        list: Rect::new(area.x + 1, y + 1, w.saturating_sub(2), h - 2),
        hint: None,
    })
}

/// `r` with `n` more cells each side (clipped at 0).
pub fn grow(r: Rect, n: u16) -> Rect {
    let x = r.x.saturating_sub(n);
    let y = r.y.saturating_sub(n);
    Rect::new(x, y, r.right() + n - x, r.bottom() + n - y)
}

/// The first item row shown, keeping `cursor` in view of `rows` rows. Start
/// from the last frame's `top`, so clicks and short moves don't scroll.
pub fn visible_top(top: usize, cursor: usize, rows: usize, n: usize) -> usize {
    let rows = rows.max(1);
    let top = if cursor < top {
        cursor
    } else if cursor >= top + rows {
        cursor + 1 - rows
    } else {
        top
    };
    top.min(n.saturating_sub(rows))
}

/// What a click at (`col`, `row`) lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Item(usize),
    Prev,
    Next,
}

pub fn hit(area: Rect, layout: &Layout, picker: &Picker, col: u16, row: u16) -> Option<Hit> {
    let at = (col, row).into();
    match placement(area, layout, picker)? {
        Placement::Inline { rect, text } => {
            if !rect.contains(at) {
                return None;
            }
            // The arrows sit in the outer 3 (padded) / 2 (bare) columns.
            let edge = match text {
                InlineText::Padded => 3,
                InlineText::Arrows => 2,
                InlineText::Name(_) => 0,
            };
            Some(if col < rect.x + edge {
                Hit::Prev
            } else if col >= rect.right() - edge {
                Hit::Next
            } else {
                Hit::Item(picker.cursor)
            })
        }
        Placement::Sheet { list, .. } => {
            if !list.contains(at) {
                return None;
            }
            let n = picker.kind.items().len();
            let top = visible_top(picker.top, picker.cursor, usize::from(list.height), n);
            let i = top + usize::from(row - list.y);
            (i < n).then_some(Hit::Item(i))
        }
    }
}

/// Draw `picker` over `area`, which `layout` was made for.
pub fn draw(buf: &mut Buffer, area: Rect, layout: &Layout, picker: &Picker, model: &Model) {
    let Some(place) = placement(area, layout, picker) else {
        return;
    };
    let theme = &model.theme;
    let items = picker.kind.items();
    let bg = Style::new().bg(theme.role(Role::Bg));
    let (text, dim, accent) = (
        theme.text(Role::Text),
        theme.text(Role::Dim),
        theme.text(Role::Accent),
    );

    let (sheet, list, hint) = match place {
        Placement::Inline { rect, text } => {
            let s = text.render(items[picker.cursor]);
            buf.set_string(rect.x, rect.y, s, accent.patch(bg));
            return;
        }
        Placement::Sheet { sheet, list, hint } => (sheet, list, hint),
    };

    Clear.render(sheet, buf);
    buf.set_style(sheet, bg);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.role(Role::Metal)))
        .title(Line::styled(format!(" {} ", picker.kind.title()), accent))
        .render(sheet, buf);

    let rows = usize::from(list.height);
    let first = visible_top(picker.top, picker.cursor, rows, items.len());
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
    if let Some(y) = hint {
        // `⏎ keep   esc revert`, from the same table as the status bar.
        let mut x = list.x + 1;
        for (key, label, _) in PICKER_HINTS.iter().filter(|h| h.0 != "↑↓") {
            if x + (key.chars().count() + 1 + label.chars().count()) as u16 > list.right() {
                break;
            }
            buf.set_string(x, y, key, dim);
            x += key.chars().count() as u16 + 1;
            buf.set_string(x, y, label, dim);
            x += label.chars().count() as u16 + 3;
        }
    }
}
