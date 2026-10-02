//! The settings screen (`,`): where it goes at each size and how it's
//! drawn. What's on it, and what keys and clicks do, is the model's
//! (`app/model/settings_screen.rs`); the geometry here is shared by
//! drawing and the mouse ([`hit`]), like the pickers'.
//!
//! * Roomy (≥ [`SETTINGS_SHEET`]): a centred sheet, the list of pages on
//!   the left, the picked page's rows on the right, and what the row under
//!   the cursor does in plain words along the bottom. The lamp keeps going
//!   around it, un-dimmed, so changes show as they're made.
//! * Smaller: a full-screen page showing one list at a time (the pages,
//!   or one page's rows), the explanation below.
//! * Micro: one line saying the window is too small.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Widget};
use unicode_width::UnicodeWidthStr;

use crate::app::{Item, Kind, Model, Page, Row, SettingsView};
use crate::dock::fit;
use crate::theme::Role;
use crate::ui::layout::{SETTINGS_SHEET, SizeTier, reaches};
use crate::ui::picker::visible_top;

/// The sheet's largest size: room for ten rows (a longer page scrolls),
/// the same on every page so switching pages never moves it.
const SHEET: (u16, u16) = (64, 19);
/// The page list's width: ` ▸ music & lyrics`.
const PAGES_W: u16 = 17;
/// Columns between the page list and the rows.
const GUTTER: u16 = 2;
/// Lines the explanation gets at most.
const ABOUT_LINES: u16 = 3;

/// The form the screen takes at a size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// One line: too small to show settings.
    Line,
    /// Full screen, one list at a time.
    Full,
    /// The centred sheet.
    Sheet(Rect),
}

pub fn mode(area: Rect) -> Mode {
    if SizeTier::of(area) == SizeTier::Micro {
        Mode::Line
    } else if reaches(area.width, area.height, SETTINGS_SHEET) {
        let (w, h) = (SHEET.0.min(area.width - 4), SHEET.1.min(area.height - 2));
        Mode::Sheet(Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        ))
    } else {
        Mode::Full
    }
}

/// Where everything goes for `view` in `area`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    /// Every cell the screen takes.
    pub footprint: Rect,
    /// The list of pages, if shown.
    pub pages: Option<Rect>,
    /// The page's title line, over its rows.
    pub header: Option<Rect>,
    /// The page's rows, if shown.
    pub rows: Option<Rect>,
    /// The explanation of the row (or page) under the cursor.
    pub about: Option<Rect>,
    /// `esc close` / `esc back`: a click there does it.
    pub back: Option<Rect>,
}

/// `esc back` in a page's rows, `esc close` in the list of pages.
fn back_text(view: &SettingsView) -> &'static str {
    if view.in_rows {
        "esc back"
    } else {
        "esc close"
    }
}

pub fn geometry(area: Rect, view: &SettingsView) -> Option<Geometry> {
    if area.is_empty() {
        return None;
    }
    let back_w = back_text(view).width() as u16;
    match mode(area) {
        Mode::Line => Some(Geometry {
            footprint: Rect::new(area.x, area.y, area.width, 1),
            pages: None,
            header: None,
            rows: None,
            about: None,
            back: None,
        }),
        Mode::Sheet(sheet) => {
            let inner = Rect::new(sheet.x + 2, sheet.y + 1, sheet.width - 4, sheet.height - 2);
            let about_h = ABOUT_LINES.min(inner.height.saturating_sub(6));
            let about = Rect::new(inner.x, inner.bottom() - about_h, inner.width, about_h);
            // A blank row above the lists and one above the explanation.
            let body_h = inner.height.saturating_sub(about_h + 2);
            let body = Rect::new(inner.x, inner.y + 1, inner.width, body_h);
            let right_x = body.x + PAGES_W + GUTTER;
            let right_w = body.right().saturating_sub(right_x);
            let back_x = sheet.right().saturating_sub(back_w + 3);
            Some(Geometry {
                footprint: sheet,
                pages: Some(Rect::new(
                    body.x,
                    body.y + 2,
                    PAGES_W,
                    body_h.saturating_sub(2),
                )),
                header: Some(Rect::new(right_x, body.y, right_w, 1)),
                rows: Some(Rect::new(
                    right_x,
                    body.y + 2,
                    right_w,
                    body_h.saturating_sub(2),
                )),
                about: (about_h > 0).then_some(about),
                back: Some(Rect::new(back_x, sheet.bottom() - 1, back_w + 2, 1)),
            })
        }
        Mode::Full => {
            let about_h = ((area.height.saturating_sub(4)) / 3).clamp(1, ABOUT_LINES);
            let body_y = area.y + 2;
            let body_h = area.height.saturating_sub(2 + about_h + 1);
            let body = Rect::new(area.x + 1, body_y, area.width.saturating_sub(2), body_h);
            let about = Rect::new(body.x, area.bottom() - about_h, body.width, about_h);
            let back = (area.width > back_w + 10)
                .then(|| Rect::new(area.right() - 1 - back_w, area.y, back_w, 1));
            Some(Geometry {
                footprint: area,
                pages: (!view.in_rows).then_some(body),
                header: None,
                rows: view.in_rows.then_some(body),
                about: (body_h > 0).then_some(about),
                back,
            })
        }
    }
}

/// Every cell the settings screen draws on: chrome there hides.
pub fn footprint(area: Rect, view: &SettingsView) -> Option<Rect> {
    geometry(area, view).map(|g| g.footprint)
}

/// How many rows of a page show at once.
pub fn visible_rows(area: Rect, _model: &Model, view: &SettingsView) -> usize {
    let view = SettingsView {
        in_rows: true,
        ..*view
    };
    geometry(area, &view)
        .and_then(|g| g.rows)
        .map_or(1, |r| usize::from(r.height).max(1))
}

/// What a click lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Page(Page),
    Row(usize),
    Back,
}

pub fn hit(area: Rect, model: &Model, view: &SettingsView, col: u16, row: u16) -> Option<Hit> {
    let g = geometry(area, view)?;
    let at = (col, row).into();
    if g.back.is_some_and(|b| b.contains(at)) {
        return Some(Hit::Back);
    }
    if let Some(pages) = g.pages.filter(|p| p.contains(at)) {
        let top = pages_top(view, pages);
        let i = top + usize::from(row - pages.y);
        return Page::LIST.get(i).map(|&p| Hit::Page(p));
    }
    if let Some(rows) = g.rows.filter(|r| r.contains(at)) {
        let n = model.settings_rows(view.page).len();
        let top = visible_top(view.top, view.cursor, usize::from(rows.height), n);
        let i = top + usize::from(row - rows.y);
        return (i < n).then_some(Hit::Row(i));
    }
    None
}

/// The first page shown in `pages` (it scrolls only in tiny windows).
fn pages_top(view: &SettingsView, pages: Rect) -> usize {
    visible_top(
        0,
        view.page.index(),
        usize::from(pages.height),
        Page::LIST.len(),
    )
}

/// The text styles the screen draws in.
struct Inks {
    text: Style,
    dim: Style,
    accent: Style,
    bg: Style,
}

pub fn draw(buf: &mut Buffer, area: Rect, view: &SettingsView, model: &Model) {
    let Some(g) = geometry(area, view) else {
        return;
    };
    let theme = &model.theme;
    let ink = Inks {
        text: theme.text(Role::Text),
        dim: theme.text(Role::Dim),
        accent: theme.text(Role::Accent),
        bg: Style::new().bg(super::background(model)),
    };
    Clear.render(g.footprint, buf);
    buf.set_style(g.footprint, ink.bg);
    let rows = model.settings_rows(view.page);
    match mode(area) {
        Mode::Line => {
            let items = ["settings", "window too small", "esc close"];
            for n in (1..=items.len()).rev() {
                let s = items[..n].join(" · ");
                if s.width() <= usize::from(area.width) {
                    buf.set_string(area.x, area.y, s, ink.dim);
                    break;
                }
            }
            return;
        }
        Mode::Sheet(sheet) => {
            let note = match &model.save_problem {
                Some(_) => " couldn't save ",
                None => " changes save automatically ",
            };
            let mut block = Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(theme.role(Role::Metal)))
                .title(Line::styled(" settings ", ink.accent))
                .title_bottom(
                    Line::styled(format!(" {} ", back_text(view)), ink.dim).right_aligned(),
                );
            if note.width() as u16 + back_text(view).width() as u16 + 8 <= sheet.width {
                block = block.title_bottom(Line::styled(note, ink.dim));
            }
            block.render(sheet, buf);
        }
        Mode::Full => {
            let title = if view.in_rows {
                format!("settings · {}", view.page.title())
            } else {
                "settings".into()
            };
            let room = g
                .back
                .map_or(area.width, |b| b.x.saturating_sub(area.x + 3));
            // Short of room, the page's own name says enough.
            let title = if title.width() as u16 > room && view.in_rows {
                view.page.title().to_owned()
            } else {
                title
            };
            buf.set_string(area.x + 1, area.y, fit(&title, room), ink.accent);
            if let Some(b) = g.back {
                buf.set_string(b.x, b.y, back_text(view), ink.dim);
            }
        }
    }
    if let Some(pages) = g.pages {
        draw_pages(buf, pages, view, &ink);
    }
    if let Some(header) = g.header {
        buf.set_stringn(
            header.x,
            header.y,
            view.page.title(),
            usize::from(header.width),
            ink.dim,
        );
    }
    let mut spilled = None;
    if let Some(r) = g.rows {
        spilled = draw_rows(buf, r, view, &rows, model, &ink);
    }
    if let Some(about) = g.about {
        // The explanation moves up under a short list (one blank row
        // between), so it gets the lines the list doesn't need.
        let list = match (g.rows, g.pages) {
            (Some(r), _) if view.in_rows || g.pages.is_some() => Some((r, rows.len())),
            (None, Some(p)) => Some((p, Page::LIST.len())),
            _ => None,
        };
        let about = list.map_or(about, |(r, n)| {
            let used = r.y + (n as u16).min(r.height);
            let top = (used + 1).clamp(r.y, about.y);
            Rect::new(about.x, top, about.width, about.bottom() - top)
        });
        draw_about(buf, about, view, &rows, spilled, model, &ink);
    }
}

fn draw_pages(buf: &mut Buffer, r: Rect, view: &SettingsView, ink: &Inks) {
    let top = pages_top(view, r);
    for (row, (i, page)) in Page::LIST
        .iter()
        .enumerate()
        .skip(top)
        .take(usize::from(r.height))
        .enumerate()
    {
        let y = r.y + row as u16;
        let current = i == view.page.index();
        let (marker, style) = match (current, view.in_rows) {
            (true, false) => ("▸ ", ink.accent),
            (true, true) => ("▸ ", ink.text),
            (false, _) => ("  ", ink.dim),
        };
        let line = fit(&format!(" {marker}{}", page.title()), r.width);
        buf.set_string(r.x, y, line, style);
    }
}

/// The page's rows. Returns the cursor row's value when it had no room
/// beside its label (the explanation shows it instead).
fn draw_rows(
    buf: &mut Buffer,
    r: Rect,
    view: &SettingsView,
    rows: &[Row],
    model: &Model,
    ink: &Inks,
) -> Option<String> {
    let n = rows.len();
    let top = visible_top(view.top, view.cursor, usize::from(r.height), n);
    let mut spilled = None;
    for (line, (i, row)) in rows
        .iter()
        .enumerate()
        .skip(top)
        .take(usize::from(r.height))
        .enumerate()
    {
        let y = r.y + line as u16;
        let focused = view.in_rows && i == view.cursor;
        let marker = if focused { "▸ " } else { "  " };
        let indent = if row.sub { "  " } else { "" };
        let label = format!(" {marker}{indent}{}", row.label);
        let label_style = if focused { ink.accent } else { ink.text };
        let label_w = label.width() as u16;
        buf.set_string(r.x, y, fit(&label, r.width), label_style);

        // The value, right-aligned: `‹ value ›` on the cursor's choice.
        // Two blank columns between label and value, one at the end.
        let room = r.width.saturating_sub(label_w + 3);
        let editing = row.item == Item::ClientId && model.settings_screen.editing;
        let value = match row.kind {
            Kind::Text => squeeze(&row.value, room, editing),
            _ => row.value.clone(),
        };
        let shown = if focused && row.kind == Kind::Choice {
            format!("‹ {value} ›")
        } else {
            value.clone()
        };
        let shown = if shown.width() as u16 <= room {
            shown
        } else if value.width() as u16 <= room {
            value
        } else {
            if focused {
                spilled = Some(row.value.clone());
            }
            continue;
        };
        let style = match (focused, row.kind) {
            (_, Kind::Info) => ink.dim,
            (true, _) => ink.accent,
            (false, Kind::Button) => ink.dim,
            (false, _) => ink.text,
        };
        let w = shown.width() as u16;
        buf.set_string(r.right() - 1 - w, y, shown, style);
    }
    spilled
}

/// `value` within `room` columns: a field being typed in keeps its end
/// (where the typing is), a saved one its start and end.
fn squeeze(value: &str, room: u16, editing: bool) -> String {
    let room = usize::from(room);
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= room || room < 5 {
        return value.to_owned();
    }
    if editing {
        let tail: String = chars[chars.len() - (room - 1)..].iter().collect();
        format!("…{tail}")
    } else {
        let head = (room - 1) / 2;
        let tail = room - 1 - head;
        let a: String = chars[..head].iter().collect();
        let b: String = chars[chars.len() - tail..].iter().collect();
        format!("{a}…{b}")
    }
}

/// The explanation: the cursor row's (its value first, if that had no
/// room in the row), or the picked page's.
fn draw_about(
    buf: &mut Buffer,
    r: Rect,
    view: &SettingsView,
    rows: &[Row],
    spilled: Option<String>,
    model: &Model,
    ink: &Inks,
) {
    let row = rows.get(view.cursor).filter(|_| view.in_rows);
    let mut text = match row {
        Some(row) => row.about.clone(),
        None => view.page.about().to_owned(),
    };
    let problem = row.is_some_and(|row| {
        row.item == Item::ClientId && matches!(model.settings_screen.note, Some(Err(_)))
    });
    if let Some(p) = &model.save_problem {
        text = format!("Couldn't save your settings: {p}");
    }
    let mut lines = Vec::new();
    if let Some(value) = spilled {
        lines.push((format!("‹ {value} ›"), ink.accent));
    }
    let room = usize::from(r.height).saturating_sub(lines.len());
    let style = if problem { ink.accent } else { ink.dim };
    lines.extend(
        wrap_sentences(&text, usize::from(r.width), room)
            .into_iter()
            .map(|l| (l, style)),
    );
    for (i, (line, style)) in lines.into_iter().take(usize::from(r.height)).enumerate() {
        buf.set_string(r.x, r.y + i as u16, fit(&line, r.width), style);
    }
}

/// `text` word-wrapped to `width`, in at most `lines` lines: whole
/// sentences are dropped from the end until it fits; none, if not even
/// the first one does (half a sentence says the wrong thing).
pub fn wrap_sentences(text: &str, width: usize, lines: usize) -> Vec<String> {
    let sentences: Vec<&str> = text.split_inclusive(". ").collect();
    for n in (1..=sentences.len()).rev() {
        let wrapped = wrap(sentences[..n].concat().trim_end(), width);
        if wrapped.len() <= lines {
            return wrapped;
        }
    }
    Vec::new()
}

/// Greedy word wrap; a word longer than `width` gets a line of its own.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.width() + 1 + word.width() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_drops_whole_sentences() {
        let text = "One two three. Four five six.";
        assert_eq!(
            wrap_sentences(text, 14, 2),
            ["One two three.", "Four five six."]
        );
        assert_eq!(wrap_sentences(text, 14, 1), ["One two three."]);
        assert_eq!(
            wrap_sentences(text, 40, 1),
            ["One two three. Four five six."]
        );
        assert!(wrap_sentences(text, 7, 1).is_empty());
    }

    #[test]
    fn a_long_field_keeps_where_the_typing_is() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(squeeze(id, 40, false), id);
        assert_eq!(squeeze(id, 11, true), "…6789abcdef");
        assert_eq!(squeeze(id, 11, false), "01234…bcdef");
    }
}
