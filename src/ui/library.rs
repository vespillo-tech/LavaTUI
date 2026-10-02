//! The Spotify library overlays: the playlist browser (`b` in the player
//! keys: my playlists → a playlist's tracks) and the add-to-playlist
//! picker (`a`). Same sheet as the pickers (§4.4, `picker::place`), wider
//! (names and artists need it), with the same small-size forms: a bottom
//! sheet, then the one-line `‹ name ›` selector. Not live: `⏎` chooses.
//!
//! `/` filters (lava-75z.17): the roomy sheet shows what's typed in its
//! spare row above the list (`/ chill▏      3 of 77`), the bottom sheet on
//! its bottom border, the inline selector before the name. The sheet keeps
//! its size while the rows thin out.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{ListView, Model};
use crate::dock::fit;
use crate::theme::Role;
use crate::ui::chrome::library_hints;
use crate::ui::layout::Layout;
use crate::ui::picker::{self, Placement, Spec};

/// The roomy sheet's width.
const SHEET_W: u16 = 44;
/// The detail column (count, artist) takes at most this share of a row.
const DETAIL_SHARE: u16 = 2;

/// The text cursor at the end of what's typed.
const CARET: &str = "▏";

/// The inline selector's text: the cursor's row, or the list's message;
/// after what's typed while filtering.
fn current(model: &Model, view: &ListView) -> String {
    let row = model
        .list_row(view.kind, view.cursor)
        .map_or_else(|| model.list_message(view.kind), |r| r.name);
    if view.typing {
        format!("/{}{CARET} {row}", model.library.find.text)
    } else {
        row
    }
}

/// How many rows match (`3 of 77`), once something is typed.
fn find_count(model: &Model, view: &ListView) -> String {
    if model.library.find.text.trim().is_empty() {
        return String::new();
    }
    let (shown, total) = (model.list_len(view.kind), model.list_total(view.kind));
    format!("{shown} of {total}")
}

/// `/ what's typed▏` in `w` columns: the end of it (where the typing
/// is), after `…`, when it doesn't fit.
fn find_text(model: &Model, w: u16) -> String {
    let text = &model.library.find.text;
    let room = usize::from(w).saturating_sub(3);
    if text.width() <= room {
        return format!("/ {text}{CARET}");
    }
    let mut tail = Vec::new();
    let mut used = 1;
    for c in text.chars().rev() {
        used += c.width().unwrap_or(0);
        if used > room {
            break;
        }
        tail.push(c);
    }
    let tail: String = tail.into_iter().rev().collect();
    format!("/ …{tail}{CARET}")
}

/// Where the library overlay goes in `area`, which `layout` was made for.
pub fn placement(area: Rect, layout: &Layout, view: &ListView, model: &Model) -> Option<Placement> {
    let current = current(model, view);
    let spec = Spec {
        // All of them, so the sheet keeps its size while filtering.
        n: model.list_total(view.kind),
        width: SHEET_W,
        left: false,
        current: &current,
        guide: &[],
    };
    picker::place(area, layout, spec)
}

pub fn draw(buf: &mut Buffer, area: Rect, layout: &Layout, view: &ListView, model: &Model) {
    let Some(place) = placement(area, layout, view, model) else {
        return;
    };
    let theme = &model.theme;
    let bg = Style::new().bg(super::background(model));
    let (text, dim, accent) = (
        theme.text(Role::Text),
        theme.text(Role::Dim),
        theme.text(Role::Accent),
    );
    let (sheet, list, hint) = match place {
        Placement::Inline { rect, text, .. } => {
            let s = text.render(&current(model, view));
            buf.set_string(rect.x, rect.y, s, accent.patch(bg));
            return;
        }
        Placement::Sheet {
            sheet, list, hint, ..
        } => (sheet, list, hint),
    };

    Clear.render(sheet, buf);
    buf.set_style(sheet, bg);
    let title = fit(&model.list_title(view), sheet.width.saturating_sub(6));
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.role(Role::Metal)))
        .title(Line::styled(format!(" {title} "), accent));
    let count = find_count(model, view);
    let count_w = count.width() as u16;
    match hint {
        // The roomy sheet's spare row above the list.
        Some(_) if view.typing && list.y > sheet.y + 1 => {
            let y = list.y - 1;
            let count_w = if count_w + 12 <= list.width {
                count_w
            } else {
                0
            };
            let typed = find_text(model, list.width.saturating_sub(3 + count_w));
            buf.set_string(list.x + 1, y, typed, accent);
            if count_w > 0 {
                buf.set_string(list.right() - 1 - count_w, y, &count, dim);
            }
        }
        // The bottom sheet's bottom border.
        None if view.typing => {
            let room = sheet.width.saturating_sub(4);
            let count_w = if count_w + 12 <= room { count_w + 2 } else { 0 };
            let mut line = find_text(model, room - count_w);
            if count_w > 0 {
                line = format!("{line}  {count}");
            }
            block = block.title_bottom(Line::styled(format!(" {line} "), accent));
        }
        _ => {}
    }
    block.render(sheet, buf);

    let n = model.list_len(view.kind);
    let rows = usize::from(list.height);
    if n == 0 {
        let line = fit(&format!("   {}", model.list_message(view.kind)), list.width);
        buf.set_string(list.x, list.y, line, dim);
    }
    let first = picker::visible_top(view.top, view.cursor, rows, n);
    for (row, i) in (first..n).take(rows).enumerate() {
        let Some(item) = model.list_row(view.kind, i) else {
            continue;
        };
        let y = list.y + row as u16;
        let selected = i == view.cursor;
        let (marker, name_style) = match (selected, item.quiet) {
            (true, _) => (model.glyphs().pointer, accent),
            (false, true) => ("  ", dim),
            (false, false) => ("  ", text),
        };
        // ` ▸ name …        detail `
        let inner = list.width.saturating_sub(4);
        let detail = fit(&item.detail, inner / DETAIL_SHARE);
        let detail_w = detail.width() as u16;
        let name_w = inner.saturating_sub(if detail_w > 0 { detail_w + 2 } else { 0 });
        let name = fit(&item.name, name_w);
        buf.set_string(list.x + 1, y, marker, name_style);
        buf.set_stringn(list.x + 3, y, &name, usize::from(name_w), name_style);
        if detail_w > 0 {
            buf.set_string(list.right() - 1 - detail_w, y, &detail, dim);
        }
    }
    if let Some(y) = hint {
        let mut x = list.x + 1;
        for (key, label, _) in library_hints(view.kind, view.typing)
            .iter()
            .filter(|h| h.0 != "↑↓")
        {
            let w = (key.chars().count() + 1 + label.chars().count()) as u16;
            if x + w > list.right() {
                break;
            }
            buf.set_string(x, y, key, dim);
            x += key.chars().count() as u16 + 1;
            buf.set_string(x, y, label, dim);
            x += label.chars().count() as u16 + 3;
        }
    }
}
