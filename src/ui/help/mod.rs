//! The help overlay (§4.3), generated from the keymap table so it can't
//! drift from dispatch.
//!
//! * Roomy terminals: a centred sheet with a rounded `metal` border, two
//!   columns (lamp | clock & pomodoro + app); the lamp keeps animating
//!   behind it, dimmed in truecolor.
//! * Smaller: a full-screen, one-column sheet, scrollable with `j k`.
//! * Micro: the single line `? close · too small for keys`.

pub mod sheet;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, BorderType, Clear, Widget};

use crate::app::Model;
use crate::theme::{ColorDepth, Role, Theme};
use crate::ui::chrome::fit_words;
use crate::ui::keymap::SELECT_DRAG;
pub use sheet::footprint;
use sheet::{Line, Mode, body, column_spans, max_scroll, mode};

/// How far the lamp behind the sheet fades toward `bg` (truecolor).
const BEHIND_FADE: f32 = 0.65;

pub fn draw(buf: &mut Buffer, area: Rect, scroll: u16, model: &Model) {
    if area.is_empty() {
        return;
    }
    let ink = Inks::new(model);
    match mode(area) {
        Mode::Line => return draw_line(buf, area, &ink),
        Mode::Full => {
            Clear.render(area, buf);
            buf.set_style(area, ink.bg);
            buf.set_string(area.x + 1, area.y, "keys", ink.accent);
            let close = "Esc close";
            let mut end = area.right() - 1;
            if area.width as usize > 6 + close.len() {
                end -= close.len() as u16;
                buf.set_string(end, area.y, close, ink.dim);
                end -= 2;
            }
            // Cut-off keys say so: `keys  ↓ j/k more   Esc close`.
            let x = area.x + 7;
            let room = usize::from(end.saturating_sub(x));
            if let Some(hint) = scroll_hint(scroll, max_scroll(area), room) {
                buf.set_string(x, area.y, hint, ink.dim);
            }
        }
        Mode::Sheet(sheet) => {
            dim_outside(buf, sheet, &model.theme);
            Clear.render(sheet, buf);
            buf.set_style(sheet, ink.bg);
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(model.theme.role(Role::Metal)))
                .title(ratatui::text::Line::styled(" keys ", ink.accent))
                // The sheet shows `S`, `C`, `A`: say what a capital means
                // (the full-screen help spells out `Shift+S`).
                .title(ratatui::text::Line::styled(
                    " capital = hold Shift ",
                    ink.dim,
                ))
                .title_bottom(ratatui::text::Line::styled(" Esc close ", ink.dim).right_aligned())
                .title_bottom(
                    scroll_hint(scroll, max_scroll(area), usize::from(sheet.width / 2))
                        .map(|hint| ratatui::text::Line::styled(format!(" {hint} "), ink.dim))
                        .unwrap_or_default(),
                )
                .render(sheet, buf);
        }
    }
    if let Some((cols, inner)) = body(area) {
        draw_body(buf, &cols, inner, scroll, &ink, model.option_drag);
    }
}

/// The text styles the help draws in.
struct Inks {
    text: Style,
    dim: Style,
    accent: Style,
    bg: Style,
}

impl Inks {
    fn new(model: &Model) -> Self {
        let theme = &model.theme;
        Inks {
            text: theme.text(Role::Text),
            dim: theme.text(Role::Dim),
            accent: theme.text(Role::Accent),
            bg: Style::new().bg(super::background(model)),
        }
    }
}

/// Micro: as much of `? close · too small for keys` as fits, on the top
/// row. Only help's own keys act under it, so it names no others.
fn draw_line(buf: &mut Buffer, area: Rect, ink: &Inks) {
    let items = ["? close", "too small for keys"];
    for n in (1..=items.len()).rev() {
        let s = items[..n].join(" · ");
        if s.chars().count() <= usize::from(area.width) {
            let line = Rect::new(area.x, area.y, area.width, 1);
            Clear.render(line, buf);
            buf.set_style(line, ink.bg);
            buf.set_string(area.x, area.y, s, ink.dim.patch(ink.bg));
            return;
        }
    }
}

/// The key columns in `inner`, scrolled down `scroll` lines.
fn draw_body(
    buf: &mut Buffer,
    cols: &[Vec<Line>],
    inner: Rect,
    scroll: u16,
    ink: &Inks,
    option_drag: bool,
) {
    for (lines, (x, col_w)) in cols.iter().zip(column_spans(cols, inner)) {
        let shown = lines
            .iter()
            .skip(usize::from(scroll))
            .take(usize::from(inner.height));
        for (row, line) in shown.enumerate() {
            let y = inner.y + row as u16;
            match *line {
                Line::Header(s) => {
                    buf.set_stringn(x, y, s.title(), usize::from(col_w), ink.dim);
                }
                Line::Key { keys, label, key_w } => {
                    // Never a key without its label: a long label sheds
                    // trailing words (`show/hide clock` → `show/hide`),
                    // and a key with no room for any is left out.
                    let lx = x + key_w as u16 + 2;
                    let room = usize::from((x + col_w).saturating_sub(lx));
                    if let Some(label) = fit_words(label, room) {
                        // Where the terminal selects with option held.
                        let keys = match keys {
                            SELECT_DRAG if option_drag => "⌥ drag",
                            keys => keys,
                        };
                        buf.set_string(x, y, keys, ink.accent);
                        buf.set_string(lx, y, label, ink.text);
                    }
                }
                Line::Note(note) => {
                    buf.set_stringn(x, y, note, usize::from(col_w), ink.dim);
                }
                Line::Blank => {}
            }
        }
    }
}

/// Which way the help can scroll from `scroll`: `↓`, `↑`, `↕`, or nothing
/// when it all fits.
fn scroll_arrows(scroll: u16, max: u16) -> Option<&'static str> {
    match (scroll > 0, scroll < max) {
        (false, false) => None,
        (false, true) => Some("↓"),
        (true, false) => Some("↑"),
        (true, true) => Some("↕"),
    }
}

/// The scroll hint (`↓ j/k more`, shortened to fit `room`), or nothing.
fn scroll_hint(scroll: u16, max: u16, room: usize) -> Option<String> {
    let arrows = scroll_arrows(scroll, max)?;
    [
        format!("{arrows} j/k more"),
        format!("{arrows} more"),
        arrows.to_owned(),
    ]
    .into_iter()
    .find(|s| s.chars().count() <= room)
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
            // A kitty placeholder's ink is its image id: leave it be.
            if let Some(cell) = buf
                .cell_mut((x, y))
                .filter(|c| !crate::graphics::is_placeholder(c.symbol()))
            {
                cell.fg = theme.fade_to_bg(cell.fg, BEHIND_FADE);
                cell.bg = theme.fade_to_bg(cell.bg, BEHIND_FADE);
            }
        }
    }
}
