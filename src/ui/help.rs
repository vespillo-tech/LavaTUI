//! The help overlay (§4.3), generated from the keymap table so it can't
//! drift from dispatch.
//!
//! * Roomy terminals: a centred sheet with a rounded `metal` border, two
//!   columns (lamp | clock & pomodoro + app); the lamp keeps animating
//!   behind it, dimmed in truecolor.
//! * Smaller: a full-screen, one-column sheet, scrollable with `j k`.
//! * Micro: the single line `? help · q quit · m mode`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType, Clear, Widget};

use crate::app::Model;
use crate::theme::{ColorDepth, Role, Theme};
use crate::ui::chrome::fit_words;
use crate::ui::keymap::{KEYMAP, Section};
use crate::ui::layout::SizeTier;

/// The sheet's outer size cap (§4.3).
const SHEET: (u16, u16) = (64, 18);
/// Columns needed inside the sheet for two columns.
const TWO_COLUMNS: u16 = 56;
/// How far the lamp behind the sheet fades toward `bg` (truecolor).
const BEHIND_FADE: f32 = 0.65;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    Header(Section),
    Key {
        keys: &'static str,
        label: &'static str,
        key_w: usize,
    },
    Blank,
}

fn section(s: Section) -> Vec<Line> {
    let rows: Vec<_> = KEYMAP.iter().filter(|r| r.section == s).collect();
    let key_w = rows
        .iter()
        .map(|r| r.keys.chars().count())
        .max()
        .unwrap_or(1);
    let mut lines = vec![Line::Header(s)];
    lines.extend(rows.iter().map(|r| Line::Key {
        keys: r.keys,
        label: r.label,
        key_w,
    }));
    lines
}

fn columns(two: bool) -> Vec<Vec<Line>> {
    let mut right = section(Section::Clock);
    right.push(Line::Blank);
    right.extend(section(Section::App));
    if two {
        vec![section(Section::Lamp), right]
    } else {
        let mut one = section(Section::Lamp);
        one.push(Line::Blank);
        one.extend(right);
        vec![one]
    }
}

enum Mode {
    Line,
    Full,
    Sheet(Rect),
}

fn mode(area: Rect) -> Mode {
    if SizeTier::of(area) == SizeTier::Micro {
        Mode::Line
    } else if area.width >= 68 && area.height >= 20 {
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

/// Rows of content and the rect they scroll in.
fn body(area: Rect) -> Option<(Vec<Vec<Line>>, Rect)> {
    match mode(area) {
        Mode::Line => None,
        Mode::Full => {
            let inner = Rect::new(
                area.x + 1,
                area.y + 2,
                area.width.saturating_sub(2),
                area.height.saturating_sub(2),
            );
            Some((columns(false), inner))
        }
        Mode::Sheet(sheet) => {
            let inner = Rect::new(sheet.x + 3, sheet.y + 1, sheet.width - 6, sheet.height - 2);
            Some((columns(inner.width >= TWO_COLUMNS), inner))
        }
    }
}

/// Every cell the help draws on (or dims under): chrome there hides.
pub fn footprint(area: Rect) -> Option<Rect> {
    if area.is_empty() {
        return None;
    }
    Some(match mode(area) {
        Mode::Line => Rect::new(area.x, area.y, area.width, 1),
        Mode::Full => area,
        Mode::Sheet(sheet) => sheet,
    })
}

/// Furthest the help can scroll at this size.
pub fn max_scroll(area: Rect) -> u16 {
    body(area).map_or(0, |(cols, inner)| {
        let lines = cols.iter().map(Vec::len).max().unwrap_or(0) as u16;
        lines.saturating_sub(inner.height)
    })
}

pub fn draw(buf: &mut Buffer, area: Rect, scroll: u16, model: &Model) {
    if area.is_empty() {
        return;
    }
    let theme = &model.theme;
    let (text, dim, accent) = (
        theme.text(Role::Text),
        theme.text(Role::Dim),
        theme.text(Role::Accent),
    );
    let bg = Style::new().bg(super::background(model));

    match mode(area) {
        Mode::Line => {
            let items = ["? help", "q quit", "m mode"];
            for n in (1..=items.len()).rev() {
                let s = items[..n].join(" · ");
                if s.chars().count() <= usize::from(area.width) {
                    let line = Rect::new(area.x, area.y, area.width, 1);
                    Clear.render(line, buf);
                    buf.set_style(line, bg);
                    buf.set_string(area.x, area.y, s, dim.patch(bg));
                    break;
                }
            }
            return;
        }
        Mode::Full => {
            Clear.render(area, buf);
            buf.set_style(area, bg);
            buf.set_string(area.x + 1, area.y, "keys", accent);
            let close = "esc close";
            if area.width as usize > 6 + close.len() {
                buf.set_string(area.right() - 1 - close.len() as u16, area.y, close, dim);
            }
        }
        Mode::Sheet(sheet) => {
            dim_outside(buf, sheet, theme);
            Clear.render(sheet, buf);
            buf.set_style(sheet, bg);
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(theme.role(Role::Metal)))
                .title(ratatui::text::Line::styled(" keys ", accent))
                .title_bottom(ratatui::text::Line::styled(" esc close ", dim).right_aligned())
                .render(sheet, buf);
        }
    }

    let Some((cols, inner)) = body(area) else {
        return;
    };
    let col_w = inner.width / cols.len() as u16;
    for (c, lines) in cols.iter().enumerate() {
        let x = inner.x + c as u16 * col_w;
        for (row, line) in lines
            .iter()
            .skip(usize::from(scroll))
            .take(usize::from(inner.height))
            .enumerate()
        {
            let y = inner.y + row as u16;
            match *line {
                Line::Header(s) => {
                    buf.set_stringn(x, y, s.title(), usize::from(col_w), dim);
                }
                Line::Key { keys, label, key_w } => {
                    // Never a key without its label: a long label sheds
                    // trailing words (`frame: auto/glass/bleed` → `frame`),
                    // and a key with no room for any is left out.
                    let lx = x + key_w as u16 + 2;
                    let room = usize::from((x + col_w).saturating_sub(lx + 1));
                    if let Some(label) = fit_words(label, room) {
                        buf.set_string(x, y, keys, accent);
                        buf.set_string(lx, y, label.trim_end_matches(':'), text);
                    }
                }
                Line::Blank => {}
            }
        }
    }
}

/// Fade everything outside `keep` toward the background (truecolor only).
fn dim_outside(buf: &mut Buffer, keep: Rect, theme: &Theme) {
    if theme.depth() != ColorDepth::TrueColor {
        return;
    }
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if keep.contains((x, y).into()) {
                continue;
            }
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.fg = theme.fade_to_bg(cell.fg, BEHIND_FADE);
                cell.bg = theme.fade_to_bg(cell.bg, BEHIND_FADE);
            }
        }
    }
}
