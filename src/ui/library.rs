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
//!
//! A song the chosen playlist has already asks first (lava-75z.24): the
//! sheet's rows give way to `already in Lamplight Mix` / `add it again?`
//! (one line, `… · add it again?`, where there's one row), `⏎ add again
//! esc cancel` in its hint row, bottom border or under the inline
//! selector. While Spotify is asked, `checking Lamplight Mix…`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Widget};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{Adding, ListView, Model, Stage};
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

/// Under the inline selector and on the bottom sheet's border while a song
/// waits to be added, longest first.
const AGAIN_GUIDE: &[&str] = &[
    "Enter add again · Esc cancel",
    "Enter add · Esc cancel",
    "Enter · Esc",
];
const CHECKING_GUIDE: &[&str] = &[
    "Enter add anyway · Esc cancel",
    "Enter add · Esc cancel",
    "Enter · Esc",
];

/// A playlist this long shows how far the check got.
const LONG: u32 = 100;

/// What the sheet says while `adding` waits: its first line (the
/// question's subject) and its second (the question, accent).
fn adding_lines(adding: &Adding) -> (String, String) {
    match adding.stage {
        Stage::Confirm => (
            format!("already in {}", adding.name),
            "add it again?".into(),
        ),
        Stage::Checking { read, total, .. } => (
            format!("checking {}…", adding.name),
            if total >= LONG && read > 0 {
                format!("{read} of {total} songs")
            } else {
                String::new()
            },
        ),
    }
}

/// [`adding_lines`] in one line of `w` columns: the longest wording that
/// fits (the question kept whole), else the shortest, cut.
fn adding_line(adding: &Adding, w: u16) -> String {
    let name = &adding.name;
    let lines = match adding.stage {
        Stage::Confirm => vec![
            format!("already in {name} · add it again?"),
            format!("in {name} · add again?"),
            "already there · add again?".into(),
            "add it again?".into(),
            "add again?".into(),
        ],
        Stage::Checking { .. } => vec![format!("checking {name}…"), "checking…".into()],
    };
    let fits = lines.iter().find(|l| l.width() <= usize::from(w));
    fit(fits.unwrap_or(&lines[lines.len() - 1]), w)
}

/// The inline selector's text in `w` columns: the cursor's row, or the
/// list's message; after what's typed while filtering; what a waiting add
/// asks.
fn current(model: &Model, view: &ListView, w: u16) -> String {
    if let Some(adding) = &model.library.adding {
        return adding_line(adding, w);
    }
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
    let current = current(model, view, area.width);
    let adding = model.library.adding.as_ref().map(|a| a.stage);
    // All of them, so the sheet keeps its size while filtering; two rows
    // at least for a waiting add's question.
    let n = model.list_total(view.kind);
    let spec = Spec {
        n: if adding.is_some() { n.max(2) } else { n },
        width: SHEET_W,
        left: false,
        current: &current,
        guide: match adding {
            Some(Stage::Confirm) => AGAIN_GUIDE,
            Some(Stage::Checking { .. }) => CHECKING_GUIDE,
            None => &[],
        },
    };
    picker::place(area, layout, spec)
}

pub fn draw(buf: &mut Buffer, area: Rect, layout: &Layout, view: &ListView, model: &Model) {
    let Some(place) = placement(area, layout, view, model) else {
        return;
    };
    let theme = &model.theme;
    let bg = Style::new().bg(super::background(model));
    let (dim, accent) = (theme.text(Role::Dim), theme.text(Role::Accent));
    let adding = model.library.adding.as_ref();
    let guide_of = |i: usize| match adding.map(|a| a.stage) {
        Some(Stage::Confirm) => AGAIN_GUIDE[i],
        _ => CHECKING_GUIDE[i],
    };
    let (sheet, list, hint, guide) = match place {
        Placement::Inline { rect, text, guide } => {
            let current = current(model, view, area.width);
            // A question, not a list to step through: no ‹ ›.
            let s = match adding {
                Some(_) => format!("{current:^w$}", w = usize::from(rect.width)),
                None => text.render(&current),
            };
            buf.set_string(rect.x, rect.y, s, accent.patch(bg));
            if let Some((g, i)) = guide {
                buf.set_string(g.x, g.y, guide_of(i), dim.patch(bg));
            }
            return;
        }
        Placement::Sheet {
            sheet,
            list,
            hint,
            guide,
        } => (sheet, list, hint, guide),
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
        _ if adding.is_some() => {
            if let Some(i) = guide {
                block = block.title_bottom(Line::styled(format!(" {} ", guide_of(i)), dim));
            }
        }
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

    if let Some(adding) = adding {
        draw_adding(buf, list, adding, model);
    } else {
        draw_rows(buf, list, view, model);
    }
    if let Some(y) = hint {
        let mut x = list.x + 1;
        let stage = adding.map(|a| a.stage);
        for (key, label, _) in library_hints(view.kind, view.typing, stage)
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

/// A waiting add's message in the list's rows: two lines (a blank row
/// above them when there's room to spare), else one.
fn draw_adding(buf: &mut Buffer, list: Rect, adding: &Adding, model: &Model) {
    let theme = &model.theme;
    let (text, dim, accent) = (
        theme.text(Role::Text),
        theme.text(Role::Dim),
        theme.text(Role::Accent),
    );
    let w = list.width.saturating_sub(4);
    let (x, mut y) = (list.x + 3, list.y);
    let (first, second) = adding_lines(adding);
    let checking = matches!(adding.stage, Stage::Checking { .. });
    if list.height < 2 {
        let line = adding_line(adding, w);
        buf.set_string(x, y, line, if checking { dim } else { accent });
        return;
    }
    if list.height >= 4 {
        y += 1;
    }
    buf.set_string(x, y, fit(&first, w), if checking { dim } else { text });
    buf.set_string(
        x,
        y + 1,
        fit(&second, w),
        if checking { dim } else { accent },
    );
}

/// The list's rows (or why there are none).
fn draw_rows(buf: &mut Buffer, list: Rect, view: &ListView, model: &Model) {
    let theme = &model.theme;
    let (text, dim, accent) = (
        theme.text(Role::Text),
        theme.text(Role::Dim),
        theme.text(Role::Accent),
    );
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
}
