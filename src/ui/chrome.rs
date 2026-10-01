//! The quiet chrome: status bar (§4.1), debug HUD and toasts (§4.2).
//! "A whisper, not a bar": no fills, no separators but spacing.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::{Model, Overlay, TOAST_TIME, Toast};
use crate::theme::{ColorDepth, Ink, Role};

/// The status bar's key hints in display order, with their drop rank
/// (lower drops first). `? help` always goes last.
pub const HINTS: &[(&str, &str, u8)] = &[
    ("s", "style", 6),
    ("c", "clock", 5),
    ("p", "palette", 4),
    ("f", "frame", 2),
    ("l", "light", 1),
    ("m", "minimal", 0),
    ("␣", "pomo", 3),
    ("?", "help", 7),
];

/// Hints shown while a picker is open (also the sheet's own hint row).
pub const PICKER_HINTS: &[(&str, &str, u8)] =
    &[("↑↓", "preview", 0), ("⏎", "keep", 1), ("esc", "revert", 2)];

/// Width of hints laid out with two spaces between them.
fn hints_width(hints: &[(&str, &str, u8)]) -> usize {
    hints
        .iter()
        .map(|(k, l, _)| k.chars().count() + 1 + l.chars().count())
        .sum::<usize>()
        + 2 * hints.len().saturating_sub(1)
}

/// The hints that fit in `avail` columns, dropping by rank, in display
/// order. Pure, so the drop order is unit-tested.
pub fn fit_hints<'a>(all: &[(&'a str, &'a str, u8)], avail: usize) -> Vec<(&'a str, &'a str, u8)> {
    let mut shown = all.to_vec();
    while !shown.is_empty() && hints_width(&shown) > avail {
        let lowest = shown
            .iter()
            .enumerate()
            .min_by_key(|(_, h)| h.2)
            .map(|(i, _)| i)
            .unwrap_or(0);
        shown.remove(lowest);
    }
    shown
}

/// Least gap between the left segment and the hints.
const GAP: usize = 4;

pub fn draw_status(buf: &mut Buffer, r: Rect, model: &Model) {
    let theme = &model.theme;
    let (text, dim, accent) = (
        theme.text(Role::Text),
        theme.text(Role::Dim),
        theme.text(Role::Accent),
    );
    let w = usize::from(r.width);

    // Left: ● style · palette.
    let mut left: Vec<(String, Style)> = if model.frozen {
        vec![("‖ ".into(), accent), ("frozen".into(), text)]
    } else {
        vec![
            ("● ".into(), accent),
            (model.style.style().name().into(), text),
        ]
    };
    if buf.area.width >= 60 {
        left.push((format!(" · {}", theme.palette().name), dim));
    }
    let left_w: usize = left.iter().map(|(s, _)| s.chars().count()).sum();
    if left_w > w {
        return;
    }
    let mut x = r.x;
    for (s, style) in &left {
        buf.set_string(x, r.y, s, *style);
        x += s.chars().count() as u16;
    }

    // Right: hints.
    let all = if matches!(model.overlay, Overlay::Picker(_)) {
        PICKER_HINTS
    } else {
        HINTS
    };
    let hints = fit_hints(all, w.saturating_sub(left_w + GAP));
    let hw = hints_width(&hints);
    let mut x = r.right() - hw as u16;
    let right_start = x;
    for (i, (key, label, _)) in hints.iter().enumerate() {
        if i > 0 {
            x += 2;
        }
        buf.set_string(x, r.y, key, text);
        x += key.chars().count() as u16 + 1;
        buf.set_string(x, r.y, label, dim);
        x += label.chars().count() as u16;
    }

    // Centre: the debug HUD, if it fits between the two.
    if model.hud {
        let hud = hud_text(model);
        let hud_w = hud.chars().count() as u16;
        let lo = r.x + left_w as u16 + GAP as u16;
        let hi = right_start.saturating_sub(GAP as u16);
        if hi > lo && hi - lo >= hud_w {
            let cx = r.x + (r.width.saturating_sub(hud_w)) / 2;
            buf.set_string(cx.clamp(lo, hi - hud_w), r.y, &hud, hud_style(model));
        }
    }
}

/// Minimal mode / no status bar: the HUD sits in the top-left corner.
/// Where it would go, if it fits.
pub fn hud_corner_rect(area: Rect, model: &Model) -> Option<Rect> {
    let w = hud_text(model).chars().count() as u16 + 2;
    (w <= area.width && area.height > 2).then(|| Rect::new(area.x, area.y, w, 1))
}

pub fn draw_hud_corner(buf: &mut Buffer, r: Rect, model: &Model) {
    let style = hud_style(model).bg(super::background(model));
    buf.set_string(r.x, r.y, format!(" {} ", hud_text(model)), style);
}

/// `60 fps · 2.1 ms · 412k px`: the samples actually taken, so a reduced
/// grid (adaptive quality, §7) shows as fewer.
fn hud_text(model: &Model) -> String {
    let samples = model.layout.lamp.map_or(0, |l| {
        let grid = model.style.style().grid();
        let n = usize::from(l.view.width)
            * usize::from(grid.x)
            * usize::from(l.view.height)
            * usize::from(grid.y);
        crate::render::samples_taken(n, model.quality.reduced_grid())
    });
    format!(
        "{:.0} fps · {:.1} ms · {}k px",
        model.stats.fps,
        model.stats.frame_ms,
        samples.div_ceil(1000)
    )
}

/// `fps` turns `wax_hot` when frames run late or adaptive quality is
/// holding the lamp back (§7).
fn hud_style(model: &Model) -> Style {
    let budget = 1000.0 / f64::from(model.target_fps());
    let role = if model.quality.degraded() || model.stats.frame_ms > 0.8 * budget {
        Role::WaxHot
    } else {
        Role::Dim
    };
    model.theme.text(role)
}

/// Where ` braille  6/12 ` goes in the toast row `r`: centred, with
/// trailing words dropped to fit (never mid-word). The text includes its
/// 1-cell pads.
pub fn toast_place(r: Rect, toast: &Toast) -> Option<(Rect, String)> {
    let text = fit_words(&toast.text, usize::from(r.width).saturating_sub(2))?;
    let padded = format!(" {text} ");
    let w = padded.chars().count() as u16;
    Some((Rect::new(r.x + (r.width - w) / 2, r.y, w, 1), padded))
}

/// Draw a placed toast. The last 400 ms fade out in truecolor.
pub fn draw_toast(buf: &mut Buffer, r: Rect, text: &str, toast: &Toast, model: &Model) {
    let theme = &model.theme;
    let age = model.now.saturating_duration_since(toast.at);
    let fade_from = TOAST_TIME.saturating_sub(std::time::Duration::from_millis(400));
    let fade = if theme.depth() == ColorDepth::TrueColor && age > fade_from {
        (age - fade_from).as_secs_f32() / 0.4
    } else {
        0.0
    };
    let fg = theme
        .paint(Ink::Role(Role::Text))
        .mix(Ink::Role(Role::Bg), fade.min(1.0))
        .color();
    let style = Style::new().fg(fg).bg(super::background(model));
    buf.set_string(r.x, r.y, text, style);
}

/// `text` cut down to whole words (and no dangling `·`) within `width`.
pub fn fit_words(text: &str, width: usize) -> Option<String> {
    let mut words: Vec<&str> = text.split(' ').collect();
    loop {
        let s = words.join(" ");
        let s = s.trim_end_matches([' ', '·']).trim_end();
        if s.is_empty() {
            return None;
        }
        if s.chars().count() <= width {
            return Some(s.to_owned());
        }
        words.pop();
    }
}
