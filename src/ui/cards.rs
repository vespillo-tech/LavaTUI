//! Cards: small bordered notes over the lamp that come with a moment, not
//! a key's overlay (§4.7): the welcome card on a first start, the music
//! note in the music controls when the music widget has no room to say
//! what's wrong, and the clock preview while choosing a face the clock
//! isn't showing. Plus the guide line: one quiet line in the toast row
//! while a mode needs saying (`music controls · Esc back`).
//!
//! All pure geometry first (the model asks [`welcome_card`] whether a key
//! should dismiss the welcome), drawing after. Chrome a card would touch
//! is left out whole, as under an overlay (§8.2).

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect, Size};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, Widget};

use crate::app::{Model, Overlay, PickerKind};
use crate::clock::{ClockWidget, FaceOptions, FaceStyle, Form, Tier};
use crate::dock::{self, Place};
use crate::theme::Role;
use crate::ui::layout::{Layout, halo};
use crate::ui::picker::{self, grow};

/// The welcome card's two forms, `(cols, rows)` with the border.
const WELCOME_FULL: (u16, u16) = (42, 11);
const WELCOME_SMALL: (u16, u16) = (28, 7);
/// The music note's widest.
const NOTE_W: u16 = 40;
/// `enlarge to preview`, in a card.
const ENLARGE: &str = "enlarge to preview";

/// The guide line's texts, longest first: the first that fits is shown.
const MUSIC_GUIDE: &[&str] = &["music controls · Esc back", "music · Esc back", "Esc back"];
const WELCOME_GUIDE: &[&str] = &[
    "? help · q quit · enlarge for tips",
    "? help · q quit",
    "? help",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WelcomeForm {
    Full,
    Small,
}

/// Where cards may go in `layout`: inside the lamp, clear of its top row
/// (toasts, the guide) and bottom row (chips), a column in from its sides.
fn room(layout: &Layout) -> Option<Rect> {
    let lamp = layout.lamp?;
    Some(Rect::new(
        lamp.x + 1,
        lamp.y + 1,
        lamp.width.checked_sub(2)?,
        lamp.height.checked_sub(2)?,
    ))
}

/// `w × h` centred in `r`, if it fits (odd cells go right/bottom).
fn centred(r: Rect, w: u16, h: u16) -> Option<Rect> {
    (w <= r.width && h <= r.height)
        .then(|| Rect::new(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2, w, h))
}

/// Whether the welcome is up (as a card or, too small for one, the guide).
pub fn welcome_shown(model: &Model) -> bool {
    model.welcome && model.overlay == Overlay::None && !model.music.keys
}

/// The welcome card's place and form in `layout`, the largest that fits;
/// `None` when even the small one doesn't (the guide line stands in).
pub fn welcome_card(layout: &Layout) -> Option<(Rect, WelcomeForm)> {
    let room = room(layout)?;
    [
        (WelcomeForm::Full, WELCOME_FULL),
        (WelcomeForm::Small, WELCOME_SMALL),
    ]
    .into_iter()
    .find_map(|(form, (w, h))| Some((centred(room, w, h)?, form)))
}

/// The music note: in the music controls, the player's problem in full
/// when the music widget isn't on screen to say it. Its rect and lines.
pub fn music_note(layout: &Layout, model: &Model) -> Option<(Rect, Vec<String>)> {
    if !model.music.keys || model.overlay != Overlay::None {
        return None;
    }
    let message = model.music.snapshot.as_ref()?.unavailable_message()?;
    let (index, _) = dock::by_name("music")?;
    if layout.placed(index).is_some() {
        return None;
    }
    let room = room(layout)?;
    let w = room.width.min(NOTE_W);
    let lines = dock::wrap(&message, w.checked_sub(4)?);
    let h = lines.len() as u16 + 2;
    Some((centred(room, w, h)?, lines))
}

/// What the face preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    Face(Form),
    /// No room for the face: `enlarge to preview`.
    Enlarge,
}

/// The face picker's preview card, while the clock itself doesn't show
/// the face (off, a chip, its text form, or hidden by the sheet): in the
/// largest part of the lamp the sheet leaves.
pub fn face_preview(area: Rect, layout: &Layout, model: &Model) -> Option<(Rect, Preview)> {
    let Overlay::Picker(p) = model.overlay else {
        return None;
    };
    if p.kind != PickerKind::Face {
        return None;
    }
    let sheet = picker::placement(area, layout, &p)?.footprint();
    if clock_shows_face(layout, model, sheet) {
        return None;
    }
    let room = room(layout)?;
    let r = beside(room, grow(sheet, 1))?;
    let opts = FaceOptions {
        hour24: model.settings.clock.hour24,
        seconds: false,
    };
    let avail = Size::new(r.width.saturating_sub(4), r.height.saturating_sub(2));
    match model.face.fit(opts, avail) {
        Some(f) if f.tier != Tier::Text || model.face.name() == "text" => Some((
            centred(r, f.size.width + 4, f.size.height + 2)?,
            Preview::Face(f),
        )),
        _ => Some((centred(r, ENLARGE.len() as u16 + 4, 3)?, Preview::Enlarge)),
    }
}

/// The largest part of `room` beside `cover`: left, right, above or below.
fn beside(room: Rect, cover: Rect) -> Option<Rect> {
    let cut = |x0: u16, y0: u16, x1: u16, y1: u16| {
        let (x0, y0) = (x0.max(room.x), y0.max(room.y));
        let (x1, y1) = (x1.min(room.right()), y1.min(room.bottom()));
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
    };
    let (r, b) = (room.right(), room.bottom());
    [
        cut(room.x, room.y, cover.x, b),
        cut(cover.right(), room.y, r, b),
        cut(room.x, room.y, r, cover.y),
        cut(room.x, cover.bottom(), r, b),
    ]
    .into_iter()
    .flatten()
    .filter(|c| !c.intersects(cover))
    .max_by_key(|c| u32::from(c.width) * u32::from(c.height))
}

/// Whether the clock widget is drawn as a face (not its text form), clear
/// of `sheet` (chrome it'd touch is left out).
fn clock_shows_face(layout: &Layout, model: &Model, sheet: Rect) -> bool {
    let Some((index, _)) = dock::by_name("clock") else {
        return false;
    };
    let Some(placed) = layout.placed(index) else {
        return false;
    };
    let near = grow(sheet, 1);
    let (place, clear) = match &layout.panel {
        Some(panel) if panel.items.iter().any(|p| p.widget == index) => {
            (Place::Side, !near.intersects(panel.rect))
        }
        _ => {
            let stack = layout
                .on_lava
                .iter()
                .find(|s| s.items.iter().any(|p| p.widget == index));
            (
                Place::Overlay,
                stack.is_some_and(|s| !near.intersects(halo(s.rect))),
            )
        }
    };
    let hour24 = model.settings.clock.hour24;
    let face = dock::clock_parts(
        model.face,
        hour24,
        place,
        placed.form,
        placed.rect,
        Alignment::Left,
    );
    clear && face.is_some_and(|(f, _, _)| f.tier != Tier::Text)
}

/// Every cell the cards take, for keeping chrome out from under them.
pub fn footprints(area: Rect, layout: &Layout, model: &Model) -> Vec<Rect> {
    let welcome = welcome_shown(model)
        .then(|| welcome_card(layout))
        .flatten()
        .map(|c| c.0);
    let note = music_note(layout, model).map(|n| n.0);
    let preview = face_preview(area, layout, model).map(|p| p.0);
    [welcome, note, preview].into_iter().flatten().collect()
}

/// The guide line, if a mode needs one: its rect in the toast row (else
/// the lamp's top row) with a 1-cell pad each side, and its text.
pub fn guide(layout: &Layout, model: &Model) -> Option<(Rect, String)> {
    let texts = if model.music.keys && model.overlay == Overlay::None {
        MUSIC_GUIDE
    } else if welcome_shown(model) && welcome_card(layout).is_none() {
        WELCOME_GUIDE
    } else {
        return None;
    };
    let row = layout
        .toast
        .or_else(|| layout.lamp.map(|l| Rect { height: 1, ..l }))?;
    let text = texts
        .iter()
        .find(|t| t.chars().count() + 2 <= usize::from(row.width))?;
    let w = text.chars().count() as u16 + 2;
    Some((
        Rect::new(row.x + (row.width - w) / 2, row.y, w, 1),
        format!(" {text} "),
    ))
}

/// A card's frame: cleared to the background, a rounded `metal` border,
/// `title` in `accent` and `bottom` in `dim`. Returns the inside.
fn frame(buf: &mut Buffer, r: Rect, title: &str, bottom: &str, model: &Model) -> Rect {
    let theme = &model.theme;
    let bg = Style::new().bg(super::background(model));
    Clear.render(r, buf);
    buf.set_style(r, bg);
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.role(Role::Metal)))
        .title(Line::styled(format!(" {title} "), theme.text(Role::Accent)));
    if !bottom.is_empty() {
        let line = Line::styled(format!(" {bottom} "), theme.text(Role::Dim));
        block = block.title_bottom(line.right_aligned());
    }
    let inner = block.inner(r);
    block.render(r, buf);
    inner
}

/// One line of a card: `(text, is a key)` runs, written from `x`.
fn runs(buf: &mut Buffer, (x, y): (u16, u16), parts: &[(&str, bool)], model: &Model) {
    let theme = &model.theme;
    let mut x = x;
    for &(s, key) in parts {
        let style = theme.text(if key { Role::Accent } else { Role::Text });
        buf.set_string(x, y, s, style);
        x += s.chars().count() as u16;
    }
}

/// Draw whichever cards are up, and the guide line unless a toast is.
pub fn draw(buf: &mut Buffer, area: Rect, layout: &Layout, model: &Model, toast_shown: bool) {
    if welcome_shown(model)
        && let Some((r, form)) = welcome_card(layout)
    {
        draw_welcome(buf, r, form, model);
    }
    if let Some((r, lines)) = music_note(layout, model) {
        let inner = frame(buf, r, "music", "Esc back", model);
        for (i, line) in lines.iter().enumerate() {
            let y = inner.y + i as u16;
            buf.set_string(inner.x + 1, y, line, model.theme.text(Role::Text));
        }
    }
    if let Some((r, preview)) = face_preview(area, layout, model) {
        let inner = frame(buf, r, "preview", "", model);
        match preview {
            Preview::Face(_) => {
                let theme = &model.theme;
                let opts = FaceOptions {
                    hour24: model.settings.clock.hour24,
                    seconds: false,
                };
                let style = FaceStyle {
                    main: theme.text(Role::Text),
                    dim: theme.text(Role::Dim),
                };
                let face_area = Rect::new(inner.x + 1, inner.y, inner.width - 2, inner.height);
                ClockWidget::new(model.face, model.local.time)
                    .options(opts)
                    .style(style)
                    .alignment(Alignment::Center)
                    .render(face_area, buf);
            }
            Preview::Enlarge => {
                buf.set_string(inner.x + 1, inner.y, ENLARGE, model.theme.text(Role::Dim));
            }
        }
    }
    if !toast_shown && let Some((r, text)) = guide(layout, model) {
        let style = model.theme.text(Role::Text).bg(super::background(model));
        buf.set_string(r.x, r.y, text, style);
    }
}

fn draw_welcome(buf: &mut Buffer, r: Rect, form: WelcomeForm, model: &Model) {
    let focus = model.settings.pomodoro.focus_min;
    match form {
        WelcomeForm::Full => {
            let inner = frame(buf, r, "Welcome to LavaTUI", "any key to start", model);
            let timer = format!("start a {focus}-minute focus timer");
            let keys: [(&str, &str); 5] = [
                ("s", "change the look"),
                ("p", "change the colours"),
                ("Space", &timer),
                ("?", "all keys and help"),
                ("q", "quit"),
            ];
            let x = inner.x + 2;
            for (i, (key, label)) in keys.iter().enumerate() {
                let y = inner.y + 1 + i as u16;
                runs(buf, (x, y), &[(key, true)], model);
                runs(buf, (x + 7, y), &[(label, false)], model);
            }
            let dim = model.theme.text(Role::Dim);
            let notes = [
                "Capital letters mean hold Shift.",
                "Your choices save automatically.",
            ];
            for (i, note) in notes.iter().enumerate() {
                buf.set_string(x, inner.y + 7 + i as u16, note, dim);
            }
        }
        WelcomeForm::Small => {
            let inner = frame(buf, r, "Welcome", "any key", model);
            let x = inner.x + 1;
            let rows: [&[(&str, bool)]; 3] = [
                &[
                    ("?", true),
                    (" help     ", false),
                    ("q", true),
                    (" quit", false),
                ],
                &[
                    ("s", true),
                    (" look     ", false),
                    ("p", true),
                    (" colours", false),
                ],
                &[("Space", true), (" focus timer", false)],
            ];
            for (i, parts) in rows.iter().enumerate() {
                runs(buf, (x, inner.y + i as u16), parts, model);
            }
            let dim = model.theme.text(Role::Dim);
            buf.set_string(x, inner.y + 3, "Shift = capital letters", dim);
            buf.set_string(x, inner.y + 4, "choices save themselves", dim);
        }
    }
}
