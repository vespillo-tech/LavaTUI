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
