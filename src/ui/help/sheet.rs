//! The help overlay's geometry (§4.3), pure: which form it takes at a
//! size, what lines it shows and where, how far it scrolls. Drawing is in
//! the parent module; the model reads [`max_scroll`] from here.

use ratatui::layout::Rect;

use crate::ui::keymap::{KEYMAP, Section};
use crate::ui::layout::{HELP_SHEET, SizeTier, reaches};

/// The sheet's outer size cap (§4.3): 24 rows show every key at 80x24.
const SHEET: (u16, u16) = (66, 24);
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
    /// A dim line of its own: the narrow help's note that keys whose
    /// labels don't fit were left out.
    Note(&'static str),
}

/// The narrow help's last line when it left keys out.
pub const WIDEN: &str = "widen for all keys";

/// Blank columns between the two columns of the sheet.
pub const GUTTER: u16 = 2;

/// A section's header and rows: as the keymap has them, or `split` one
/// action a line ([`crate::ui::keymap::Row::split`]).
fn section(s: Section, split: bool) -> Vec<Line> {
    let mut lines = vec![Line::Header(s)];
    for r in KEYMAP.iter().filter(|r| r.section == s) {
        let rows = if split {
            r.split()
        } else {
            vec![(r.keys, r.label)]
        };
        lines.extend(rows.into_iter().map(|(keys, label)| Line::Key {
            keys,
            label,
            key_w: 0,
        }));
    }
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

/// Two columns (lamp + clock & pomodoro + mouse | widgets + music + app:
/// at most 22 rows, so 80x24 shows them whole), labels lined up per
/// column; or one `width` wide, app first so `m ? q` are on screen at the
/// smallest sizes, one action a line, labels lined up per section (room
/// is short there). A label there shows whole or not at all (cut short,
/// it could say something else): rows without room are left out and the
/// last line says to widen the window.
fn columns(two: bool, width: u16) -> Vec<Vec<Line>> {
    if two {
        let column = |sections: &[Section]| {
            let mut col = Vec::new();
            for (i, &s) in sections.iter().enumerate() {
                if i > 0 {
                    col.push(Line::Blank);
                }
                col.extend(section(s, false));
            }
            align(col)
        };
        return vec![
            column(&[Section::Lamp, Section::Clock, Section::Mouse]),
            column(&[Section::Widgets, Section::Music, Section::App]),
        ];
    }
    let mut one = align(section(Section::App, true));
    for s in [
        Section::Lamp,
        Section::Clock,
        Section::Widgets,
        Section::Music,
        Section::Mouse,
    ] {
        one.push(Line::Blank);
        one.extend(align(section(s, true)));
    }
    let fits = |l: &Line| match *l {
        Line::Key { label, key_w, .. } => key_w + 2 + label.chars().count() <= usize::from(width),
        _ => true,
    };
    if !one.iter().all(fits) {
        one.retain(fits);
        one.extend([Line::Blank, Line::Note(WIDEN)]);
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
            Line::Note(s) => s.chars().count(),
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
        let (w, h) = (SHEET.0.min(area.width - 4), SHEET.1.min(area.height));
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
            Some((columns(false, inner.width), inner))
        }
        Mode::Sheet(sheet) => {
            let inner = Rect::new(sheet.x + 3, sheet.y + 1, sheet.width - 6, sheet.height - 2);
            Some((columns(inner.width >= TWO_COLUMNS, inner.width), inner))
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
