//! The help overlay's geometry (§4.3), pure: which form it takes at a
//! size, what lines it shows and where, how far it scrolls. Drawing is in
//! the parent module; the model reads [`max_scroll`] from here.

use ratatui::layout::Rect;

use crate::ui::keymap::{KEYMAP, Section};
use crate::ui::layout::{HELP_SHEET, SizeTier, reaches};

/// The sheet's outer size cap (§4.3).
const SHEET: (u16, u16) = (64, 18);
/// Columns needed inside the sheet for two columns.
const TWO_COLUMNS: u16 = 56;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line {
    Header(Section),
    Key {
        keys: &'static str,
        label: &'static str,
        key_w: usize,
    },
    Blank,
}

/// Blank columns between the two columns of the sheet.
pub const GUTTER: u16 = 2;

fn section(s: Section) -> Vec<Line> {
    let mut lines = vec![Line::Header(s)];
    lines.extend(KEYMAP.iter().filter(|r| r.section == s).map(|r| Line::Key {
        keys: r.keys,
        label: r.label,
        key_w: 0,
    }));
    lines
}

/// Line the labels in `lines` up at their widest key + 2.
fn align(mut lines: Vec<Line>) -> Vec<Line> {
    let widest = lines
        .iter()
        .filter_map(|l| match l {
            Line::Key { keys, .. } => Some(keys.chars().count()),
            _ => None,
        })
        .max()
        .unwrap_or(1);
    for line in &mut lines {
        if let Line::Key { key_w, .. } = line {
            *key_w = widest;
        }
    }
    lines
}

/// Two columns (lamp + widgets | clock & pomodoro + app), labels lined
/// up per column; or one, app first so `m ? q` are on screen at the
/// smallest sizes, labels lined up per section (room is short there).
fn columns(two: bool) -> Vec<Vec<Line>> {
    if two {
        let pair = |a, b| {
            let mut col = section(a);
            col.push(Line::Blank);
            col.extend(section(b));
            align(col)
        };
        return vec![
            pair(Section::Lamp, Section::Widgets),
            pair(Section::Clock, Section::App),
        ];
    }
    let mut one = align(section(Section::App));
    for s in [Section::Lamp, Section::Clock, Section::Widgets] {
        one.push(Line::Blank);
        one.extend(align(section(s)));
    }
    vec![one]
}

/// A column's natural width: its widest line.
fn natural_width(lines: &[Line]) -> u16 {
    lines
        .iter()
        .map(|l| match *l {
            Line::Header(s) => s.title().chars().count(),
            Line::Key { label, key_w, .. } => key_w + 2 + label.chars().count(),
            Line::Blank => 0,
        })
        .max()
        .unwrap_or(0) as u16
}

/// Where each column is drawn in `inner`: `(x, width)`, the width not
/// counting the gutter after it. The first of two columns gets its
/// natural width (at least half) plus [`GUTTER`]; the last gets the rest.
pub fn column_spans(cols: &[Vec<Line>], inner: Rect) -> Vec<(u16, u16)> {
    match cols {
        [left, _] => {
            let left_w = (natural_width(left) + GUTTER)
                .max(inner.width / 2)
                .min(inner.width);
            vec![
                (inner.x, left_w.saturating_sub(GUTTER)),
                (inner.x + left_w, inner.width - left_w),
            ]
        }
        _ => vec![(inner.x, inner.width)],
    }
}

pub enum Mode {
    Line,
    Full,
    Sheet(Rect),
}

pub fn mode(area: Rect) -> Mode {
    if SizeTier::of(area) == SizeTier::Micro {
        Mode::Line
    } else if reaches(area.width, area.height, HELP_SHEET) {
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
pub fn body(area: Rect) -> Option<(Vec<Vec<Line>>, Rect)> {
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
