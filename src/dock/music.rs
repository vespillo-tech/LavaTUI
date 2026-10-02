//! Music (now playing): title, artist, album, a progress bar with the
//! times, play state and volume, and a small cover; `▶ title – artist` as
//! its chip. Off by default (`a` places it, `A` turns on the player keys).
//!
//! Forms, most preferred first (the layout keeps the first that fits):
//!
//! | form | size | shows |
//! |---|---|---|
//! | cover beside | ≥ 32 × 6 | a 12 × 6 cover left of the card |
//! | card | ≥ 20 × 6 | title, artist, album, ·, bar, `▶ 1:23  vol 70  3:45` |
//! | compact | ≥ 20 × 3 | title, artist, `▶ 1:23 ━━━─── 3:45` |
//! | line | ≤ 36 × 1 | `▶ title – artist` |
//!
//! The big cover is its own widget ([`super::Cover`]); the card's small
//! one is there only with `art.inline` on, while the cover widget is off,
//! when covers can be shown at all ([`Drawn`](super::cover::Drawn)) and the
//! track has one. It's drawn the cover widget's way (pixels or text cells);
//! until it has loaded a quiet placeholder holds its place, so nothing
//! jumps when it arrives. With no player to show, the widget is one calm
//! sentence instead (`Open Spotify to show music`), and its chip the short
//! of it (`♪ open Spotify`, else `♪ see Shift+A`: the music controls show
//! the whole sentence when the widget has no room for it).

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{Anchor, Backdrop, ChipText, DockWidget, Look, Place, WidgetForm, align_x};
use crate::app::{Account, Model};
use crate::glyphs::Glyphs;
use crate::media::{Snapshot, Status, Unavailable};
use crate::theme::Role;
use crate::ui::keymap::PlayerKey;

pub struct Music;

/// The cover beside the card: 12 cols, 6 rows, as tall as the card.
const SIDE_COVER: u16 = 12;
/// The card's rows, and its narrowest.
const CARD: (u16, u16) = (20, 6);
const COMPACT: (u16, u16) = (20, 3);
/// The card beside a cover is at least this wide.
const BESIDE_W: u16 = 18;
/// Widest one-line form and chip.
const LINE_MAX: u16 = 36;
const CHIP_MAX: u16 = 32;
/// Messages wrap at this width: in the panel (whose narrowest inside is
/// 20), and on the lava.
const MESSAGE_W: (u16, u16) = (20, 30);

/// `WidgetForm::variant`: the kind in the high byte, a cover size low.
const V_COVER_SIDE: u16 = 0x200;
const V_CARD: u16 = 0x300;
const V_COMPACT: u16 = 0x400;
const V_LINE: u16 = 0x500;
const V_MESSAGE: u16 = 0x600;

/// What the widget has to show, decided from the model (pure input to
/// [`music_forms`], so the layout tests can make any of them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Show {
    Track {
        /// The card's small cover is shown (`art.inline`, the cover widget
        /// off, pictures possible, and the track has one).
        cover: bool,
        /// Playing (the times tick every second).
        playing: bool,
        /// Width of the one-line form's text.
        line_w: u16,
    },
    /// A calm sentence: why there's no player, or that nothing is loaded
    /// (with its `♪`, when the glyphs have one).
    Message(String),
}

fn show(model: &Model) -> Show {
    let message = |text: &str| Show::Message(message_text(text, glyphs(model)));
    // Not read yet (the player connects on the next frame): never no
    // forms, which would take the whole panel down with it.
    let Some(snap) = model.music.snapshot.as_ref() else {
        return message("…");
    };
    match (&snap.status, &snap.track) {
        (Status::Unavailable(reason), _) => message(&reason.message(snap.player_name())),
        (Status::Connecting, _) => message("…"),
        (Status::Stopped, _) | (_, None) => message("nothing playing"),
        (Status::Playing | Status::Paused, Some(track)) => Show::Track {
            cover: model.inline_cover() && !track.artwork_url.is_empty(),
            playing: snap.status == Status::Playing,
            line_w: width(&line_text(snap, glyphs(model))).min(LINE_MAX),
        },
    }
}

/// The forms for `show` in `place`, most preferred first (pure).
pub fn music_forms(show: &Show, place: Place) -> Vec<WidgetForm> {
    match show {
        Show::Message(text) => {
            let lines = wrap(text, message_w(place));
            let w = lines.iter().map(|l| width(l)).max().unwrap_or(1);
            vec![WidgetForm::fixed(w, lines.len() as u16, V_MESSAGE)]
        }
        &Show::Track {
            cover,
            playing,
            line_w,
        } => {
            let ticking = |mut f: WidgetForm| {
                f.seconds = playing;
                f
            };
            let mut forms = Vec::new();
            if cover {
                forms.push(WidgetForm::fill(
                    SIDE_COVER + 2 + BESIDE_W,
                    CARD.1,
                    V_COVER_SIDE | SIDE_COVER,
                ));
            }
            forms.push(WidgetForm::fill(CARD.0, CARD.1, V_CARD));
            forms.push(WidgetForm::fill(COMPACT.0, COMPACT.1, V_COMPACT));
            let mut forms: Vec<WidgetForm> = forms.into_iter().map(ticking).collect();
            forms.push(WidgetForm::fixed(line_w.max(1), 1, V_LINE));
            forms
        }
    }
}

fn message_w(place: Place) -> u16 {
    match place {
        Place::Overlay => MESSAGE_W.1,
        _ => MESSAGE_W.0,
    }
}

fn message_text(text: &str, g: &Glyphs) -> String {
    format!("{}{text}", g.note)
}

/// The model's glyph set ([`crate::glyphs`]).
pub fn glyphs(model: &Model) -> &'static Glyphs {
    model.glyphs()
}

/// `▶`, `‖` or `■`.
fn glyph(snap: &Snapshot, g: &Glyphs) -> &'static str {
    match snap.status {
        Status::Playing => g.playing,
        Status::Paused => g.paused,
        _ => g.stopped,
    }
}

/// `▶ title – artist` (just the title when there's no artist).
fn line_text(snap: &Snapshot, g: &Glyphs) -> String {
    let Some(track) = &snap.track else {
        return String::new();
    };
    let g = glyph(snap, g);
    match track.artist.trim() {
        "" => format!("{g} {}", track.name),
        artist => format!("{g} {} – {artist}", track.name),
    }
}

impl DockWidget for Music {
    fn name(&self) -> &'static str {
        "music"
    }

    fn default_place(&self) -> Place {
        Place::Off
    }

    fn forms(&self, model: &Model, place: Place) -> Vec<WidgetForm> {
        music_forms(&show(model), place)
    }

    fn draw(&self, model: &Model, form: WidgetForm, area: Rect, look: Look, buf: &mut Buffer) {
        if form.variant & 0xff00 == V_MESSAGE {
            if let Show::Message(text) = show(model) {
                let lines = wrap(&text, form.size.width);
                let dim = model.theme.text(Role::Dim);
                for (i, line) in lines.iter().take(usize::from(area.height)).enumerate() {
                    let line = fit(line, area.width);
                    let x = area.x + align_x(look.align, area.width, width(&line));
                    buf.set_stringn(x, area.y + i as u16, &line, usize::from(area.width), dim);
                }
            }
            return;
        }
        let Some(snap) = &model.music.snapshot else {
            return;
        };
        let Some(parts) = parts(form, area, look.align) else {
            return;
        };
        let mut pen = Pen {
            model,
            snap,
            buf,
            lava: look.backdrop == Backdrop::Lava,
        };
        if parts.line {
            let style = model.theme.text(Role::Text);
            pen.buf_line(area, 0, &line_text(snap, glyphs(model)), style, look.align);
        }
        if let Some(r) = parts.compact {
            pen.compact(r);
        }
        if let Some(r) = parts.cover {
            pen.cover(r);
        }
        if let Some((r, align)) = parts.card {
            pen.card(r, align);
        }
    }

    /// Top left: clear of the clock (centre) and of the lyrics to come
    /// (bottom centre).
    fn default_anchor(&self) -> Anchor {
        Anchor::TopLeft
    }

    /// Playing 2 (above the clock), paused 1, otherwise 0; a player
    /// problem the user can fix 2, so its chip outlasts the clock's.
    fn rank(&self, model: &Model) -> u8 {
        match model.music.snapshot.as_ref().map(|s| &s.status) {
            Some(Status::Playing) => 2,
            Some(Status::Unavailable(Unavailable::Unsupported)) => 0,
            Some(Status::Unavailable(_)) => 2,
            Some(Status::Paused) => 1,
            _ => 0,
        }
    }

    /// `▶ title – artist` while playing, `‖ …` while paused; nothing
    /// without a track.
    fn chip(&self, model: &Model) -> Option<ChipText> {
        let snap = model.music.snapshot.as_ref()?;
        if let Status::Unavailable(reason) = &snap.status {
            return problem_chip(reason, snap.player_name(), glyphs(model));
        }
        snap.track.as_ref()?;
        if !matches!(snap.status, Status::Playing | Status::Paused) {
            return None;
        }
        Some(ChipText {
            text: fit(&line_text(snap, glyphs(model)), CHIP_MAX),
            ink: Role::Text,
        })
    }
}

/// The chip for a player problem: the next step when it's short, else
/// where to read it (the music controls' note).
fn problem_chip(reason: &Unavailable, player: &str, g: &Glyphs) -> Option<ChipText> {
    let note = g.note;
    let text = match reason {
        Unavailable::Unsupported => return None,
        Unavailable::NotRunning if player.contains(' ') => format!("{note}open your player"),
        Unavailable::NotRunning => format!("{note}open {player}"),
        _ => format!("{note}see Shift+A"),
    };
    Some(ChipText {
        text: fit(&text, CHIP_MAX),
        ink: Role::Text,
    })
}

/// Where a form's pieces go in its rect: what [`Music::draw`] draws and
/// what [`hit`] tests clicks against, so the two can't disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Parts {
    line: bool,
    compact: Option<Rect>,
    cover: Option<Rect>,
    card: Option<(Rect, Alignment)>,
}

fn parts(form: WidgetForm, area: Rect, align: Alignment) -> Option<Parts> {
    let size = form.variant & 0xff;
    let mut parts = Parts::default();
    match form.variant & 0xff00 {
        V_LINE => parts.line = true,
        V_COMPACT => parts.compact = Some(area),
        V_CARD => parts.card = Some((area, align)),
        V_COVER_SIDE => {
            if area.width < size + 2 + BESIDE_W || area.height < CARD.1 {
                return None;
            }
            parts.cover = Some(Rect::new(area.x, area.y, size, size / 2));
            let card = Rect::new(area.x + size + 2, area.y, area.width - size - 2, CARD.1);
            parts.card = Some((card, Alignment::Left));
        }
        _ => return None,
    }
    Some(parts)
}

/// Where the inline cover goes in a placed form, if it has one.
pub fn cover_rect(form: WidgetForm, area: Rect, align: Alignment) -> Option<Rect> {
    parts(form, area, align)?.cover
}

/// A clickable control in the widget (each also has a player key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Previous,
    PlayPause,
    Next,
    Like,
    Add,
    Playlists,
    LogIn,
}

impl Button {
    fn key(self) -> PlayerKey {
        match self {
            Button::Previous => PlayerKey::Previous,
            Button::PlayPause => PlayerKey::PlayPause,
            Button::Next => PlayerKey::Next,
            Button::Like => PlayerKey::Like,
            Button::Add => PlayerKey::AddToPlaylist,
            Button::Playlists => PlayerKey::Playlists,
            Button::LogIn => PlayerKey::Account,
        }
    }
}

/// A control as drawn: where, what, in which ink.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Control {
    rect: Rect,
    button: Button,
    text: String,
    ink: Role,
}

/// The heart: `♥` (accent) when liked, `♡` when not; `None` when there's
/// no login or the track isn't a Spotify one.
fn heart(model: &Model) -> Option<(String, Role)> {
    let g = glyphs(model);
    match model.liked()? {
        true => Some((g.liked.into(), Role::Accent)),
        false => Some((g.unliked.into(), Role::Dim)),
    }
}

/// The card's controls row: `◂◂  ‖  ▸▸` left, `♡  +  ≡` (or `log in`)
/// right, all quiet. Without the mouse only the heart shows, and only when
/// the track is liked. Right-hand controls drop from the end, then the
/// left ones go, rather than crowd.
fn card_controls(model: &Model, snap: &Snapshot, r: Rect) -> Vec<Control> {
    let mouse = model.settings.input.mouse;
    let g = glyphs(model);
    let y = r.y + 3;
    let mut left: Vec<(Button, String, Role)> = Vec::new();
    if mouse {
        let play = if snap.status == Status::Playing {
            g.paused
        } else {
            g.playing
        };
        left = vec![
            (Button::Previous, g.previous.into(), Role::Dim),
            (Button::PlayPause, play.into(), Role::Dim),
            (Button::Next, g.next.into(), Role::Dim),
        ];
    }
    let mut right: Vec<(Button, String, Role)> = Vec::new();
    let spotify_track = model.liked().is_some();
    match model.library.account() {
        Account::LoggedIn => {
            if let Some((text, ink)) = heart(model).filter(|(_, ink)| mouse || *ink == Role::Accent)
            {
                right.push((Button::Like, text, ink));
            }
            if mouse {
                if spotify_track {
                    right.push((Button::Add, g.add.into(), Role::Dim));
                }
                right.push((Button::Playlists, g.playlists.into(), Role::Dim));
            }
        }
        Account::LoggedOut if mouse => right.push((Button::LogIn, "log in".into(), Role::Dim)),
        Account::LoggingIn if mouse => {
            right.push((Button::LogIn, "logging in…".into(), Role::Dim));
        }
        _ => {}
    }
    // Two spaces between controls; one when that's what it takes to keep
    // them all; then the right-hand ones drop from the end.
    let group_w = |g: &[(Button, String, Role)], sep: u16| -> u16 {
        g.iter().map(|(_, t, _)| width(t)).sum::<u16>() + sep * (g.len() as u16).saturating_sub(1)
    };
    type Group = [(Button, String, Role)];
    let fits = |l: &Group, rt: &Group, sep| group_w(l, sep) + 3 + group_w(rt, sep) <= r.width;
    let mut sep = 2;
    if !fits(&left, &right, sep) {
        sep = 1;
    }
    while !right.is_empty() && !fits(&left, &right, sep) {
        right.pop();
    }
    if group_w(&left, sep) > r.width {
        left.clear();
    }
    let mut out = Vec::new();
    let mut x = r.x;
    for (button, text, ink) in left {
        let w = width(&text);
        out.push(Control {
            rect: Rect::new(x, y, w, 1),
            button,
            text,
            ink,
        });
        x += w + sep;
    }
    let mut x = r.right().saturating_sub(group_w(&right, sep));
    for (button, text, ink) in right {
        let w = width(&text);
        out.push(Control {
            rect: Rect::new(x, y, w, 1),
            button,
            text,
            ink,
        });
        x += w + sep;
    }
    out
}

/// The compact form's controls: the play glyph and the heart (at the end
/// of the title row).
fn compact_controls(model: &Model, snap: &Snapshot, r: Rect) -> Vec<Control> {
    let mut out = Vec::new();
    if model.settings.input.mouse && r.height >= 3 {
        let w = width(glyph(snap, glyphs(model))).min(r.width);
        out.push(Control {
            rect: Rect::new(r.x, r.y + 2, w, 1),
            button: Button::PlayPause,
            text: String::new(),
            ink: Role::Text,
        });
    }
    if let Some((text, ink)) =
        heart(model).filter(|(_, ink)| model.settings.input.mouse || *ink == Role::Accent)
        && r.width >= CARD.0
    {
        let w = width(&text);
        out.push(Control {
            rect: Rect::new(r.right() - w, r.y, w, 1),
            button: Button::Like,
            text,
            ink,
        });
    }
    out
}

/// The compact form's progress bar, when the row has room for one (else
/// it shows the status line).
fn compact_bar(snap: &Snapshot, g: &Glyphs, now: std::time::Instant, row: Rect) -> Option<Rect> {
    let left = format!("{} {}", glyph(snap, g), clock(snap.position_at(now)));
    let total = total_text(snap);
    let (lw, tw) = (width(&left), width(&total));
    if lw + tw + 6 > row.width {
        return None;
    }
    let bar_x = row.x + lw + 1;
    let bar_w = row.width - lw - 1 - if tw > 0 { tw + 1 } else { 0 };
    Some(Rect::new(bar_x, row.y, bar_w, 1))
}

/// What a click at (`col`, `row`) in this placed form does, if anything:
/// a control's player key, or a seek on the progress bar. The same
/// geometry the widget draws with.
pub fn hit(
    model: &Model,
    form: WidgetForm,
    area: Rect,
    align: Alignment,
    col: u16,
    row: u16,
) -> Option<PlayerKey> {
    let snap = model.music.snapshot.as_ref()?;
    snap.track.as_ref()?;
    if !matches!(snap.status, Status::Playing | Status::Paused) {
        return None;
    }
    let parts = parts(form, area, align)?;
    let at = (col, row).into();
    let seek = |bar: Rect| {
        let f = (f64::from(col - bar.x) + 0.5) / f64::from(bar.width.max(1));
        PlayerKey::SeekTo((f * 1000.0).round().clamp(0.0, 1000.0) as u16)
    };
    if let Some((r, _)) = parts.card {
        let controls = card_controls(model, snap, r);
        if let Some(c) = controls.iter().find(|c| c.rect.contains(at)) {
            return Some(c.button.key());
        }
        let bar = Rect::new(r.x, r.y + 4, r.width, 1);
        if bar.contains(at) {
            return Some(seek(bar));
        }
    }
    if let Some(r) = parts.compact {
        let controls = compact_controls(model, snap, r);
        if let Some(c) = controls.iter().find(|c| c.rect.contains(at)) {
            return Some(c.button.key());
        }
        let row = Rect::new(r.x, r.y + 2, r.width, 1);
        let bar = compact_bar(snap, glyphs(model), model.now, row);
        if let Some(bar) = bar.filter(|b| b.contains(at)) {
            return Some(seek(bar));
        }
    }
    None
}

/// Draws the parts of the widget.
struct Pen<'a, 'b> {
    model: &'a Model,
    snap: &'a Snapshot,
    buf: &'b mut Buffer,
    /// On the lava: the controls are bold there, strokes enough for
    /// their ink to read over bright wax.
    lava: bool,
}

impl Pen<'_, '_> {
    /// `text`, cut to fit, on row `dy` of `r`, lined up by `align`.
    fn buf_line(&mut self, r: Rect, dy: u16, text: &str, style: Style, align: Alignment) {
        let text = fit(text, r.width);
        let x = r.x + align_x(align, r.width, width(&text));
        self.buf
            .set_stringn(x, r.y + dy, &text, usize::from(r.width), style);
    }

    /// Title, artist, album, a blank row, the bar, the status line.
    fn card(&mut self, r: Rect, align: Alignment) {
        let Some(track) = &self.snap.track else {
            return;
        };
        let theme = &self.model.theme;
        let (text, dim) = (theme.text(Role::Text), theme.text(Role::Dim));
        self.buf_line(r, 0, &track.name, text, align);
        self.buf_line(r, 1, &track.artist, dim, align);
        self.buf_line(r, 2, &track.album, dim, align);
        for c in card_controls(self.model, self.snap, r) {
            let mut style = theme.text(c.ink);
            if self.lava {
                style = style.add_modifier(Modifier::BOLD);
            }
            self.buf.set_string(c.rect.x, c.rect.y, &c.text, style);
        }
        self.bar(Rect::new(r.x, r.y + 4, r.width, 1));
        self.status(Rect::new(r.x, r.y + 5, r.width, 1));
    }

    /// Title (and the heart), artist, and `▶ 1:23 ━━━─── 3:45`.
    fn compact(&mut self, r: Rect) {
        let Some(track) = &self.snap.track else {
            return;
        };
        let theme = &self.model.theme;
        let controls = compact_controls(self.model, self.snap, r);
        let heart = controls.iter().find(|c| c.button == Button::Like);
        let title_w = heart.map_or(r.width, |c| r.width - c.rect.width - 1);
        let title = Rect {
            width: title_w,
            ..r
        };
        self.buf_line(
            title,
            0,
            &track.name,
            theme.text(Role::Text),
            Alignment::Left,
        );
        if let Some(c) = heart {
            self.buf
                .set_string(c.rect.x, c.rect.y, &c.text, theme.text(c.ink));
        }
        self.buf_line(r, 1, &track.artist, theme.text(Role::Dim), Alignment::Left);
        let now = self.model.now;
        let total = self.total();
        let row = Rect::new(r.x, r.y + 2, r.width, 1);
        let Some(bar) = compact_bar(self.snap, glyphs(self.model), now, row) else {
            self.status(row);
            return;
        };
        self.left(row);
        self.bar(bar);
        let buf = &mut *self.buf;
        let tw = width(&total);
        buf.set_string(row.right() - tw, row.y, &total, theme.text(Role::Dim));
    }

    /// `━━━━━─────`: elapsed in `text` (`dim` while paused), the rest dim.
    fn bar(&mut self, r: Rect) {
        let theme = &self.model.theme;
        let progress = self.snap.progress_at(self.model.now).unwrap_or(0.0);
        let filled = ((progress.clamp(0.0, 1.0) * f64::from(r.width)).round() as u16).min(r.width);
        let on = if self.snap.status == Status::Playing {
            theme.text(Role::Text)
        } else {
            theme.text(Role::Dim)
        };
        let buf = &mut *self.buf;
        buf.set_string(r.x, r.y, "━".repeat(usize::from(filled)), on);
        let rest = "─".repeat(usize::from(r.width - filled));
        buf.set_string(r.x + filled, r.y, rest, theme.text(Role::Dim));
    }

    /// The glyph and elapsed time, left (the glyph in `accent` while the
    /// player keys are on: the one hint that they are).
    fn left(&mut self, r: Rect) {
        let theme = &self.model.theme;
        let g = glyph(self.snap, glyphs(self.model));
        let g_style = if self.model.music.keys {
            theme.text(Role::Accent)
        } else {
            theme.text(Role::Text)
        };
        let rest = format!(" {}", clock(self.snap.position_at(self.model.now)));
        let buf = &mut *self.buf;
        let (x, _) = buf.set_stringn(r.x, r.y, g, usize::from(r.width), g_style);
        let w = usize::from(r.right().saturating_sub(x));
        buf.set_stringn(x, r.y, rest, w, theme.text(Role::Text));
    }

    /// `left`'s text: the glyph and the elapsed time.
    fn left_text(&self) -> String {
        let g = glyph(self.snap, glyphs(self.model));
        format!("{g} {}", clock(self.snap.position_at(self.model.now)))
    }

    /// `▶ 1:23   ⇄ ↻  vol 70  3:45`: what fits, the total and volume first
    /// to go.
    fn status(&mut self, r: Rect) {
        let theme = &self.model.theme;
        let left = self.left_text();
        let g = glyphs(self.model);
        let mut right = vec![self.total()];
        let caps = self.model.music.capabilities();
        let mut modes = String::new();
        if caps.shuffle && self.snap.shuffle {
            modes.push_str(g.shuffle);
        }
        if caps.repeat && self.snap.repeat {
            if !modes.is_empty() {
                modes.push(' ');
            }
            modes.push_str(g.repeat);
        }
        if caps.volume {
            right.insert(0, format!("vol {}", self.snap.volume));
        }
        if !modes.is_empty() {
            right.insert(0, modes);
        }
        right.retain(|s| !s.is_empty());
        let lw = width(&left);
        // Drop from the left of the right-hand group until it all fits.
        while !right.is_empty() {
            let rw: u16 =
                right.iter().map(|s| width(s)).sum::<u16>() + 2 * (right.len() as u16 - 1);
            if lw + 2 + rw <= r.width {
                break;
            }
            right.remove(0);
        }
        self.left(r);
        let dim = theme.text(Role::Dim);
        let mut x = r.right();
        let buf = &mut *self.buf;
        for (i, part) in right.iter().rev().enumerate() {
            if i > 0 {
                x -= 2;
            }
            x -= width(part);
            buf.set_string(x, r.y, part, dim);
        }
    }

    fn total(&self) -> String {
        total_text(self.snap)
    }

    /// The small cover in `r` (the cover widget's drawing).
    fn cover(&mut self, r: Rect) {
        super::cover::draw_cover(self.model, r, self.buf);
    }
}

/// The track's length, `3:45` (empty when unknown).
fn total_text(snap: &Snapshot) -> String {
    snap.track
        .as_ref()
        .map(|t| t.duration)
        .filter(|d| !d.is_zero())
        .map_or_else(String::new, clock)
}

/// `1:23`, `1:02:03`.
fn clock(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Display width (CJK titles are two columns a character).
pub(super) fn width(s: &str) -> u16 {
    s.width().min(usize::from(u16::MAX)) as u16
}

/// `s` cut to `w` columns, with `…` when cut.
pub fn fit(s: &str, w: u16) -> String {
    let w = usize::from(w);
    if s.width() <= w {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if used + cw + 1 > w {
            break;
        }
        out.push(c);
        used += cw;
    }
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    if w > 0 {
        out.push('…');
    }
    out
}

/// `text` in lines of at most `w` columns, broken between words (a word
/// longer than a line is cut).
pub fn wrap(text: &str, w: u16) -> Vec<String> {
    let w = w.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let word = fit(word, w);
        let joined = if line.is_empty() {
            word.clone()
        } else {
            format!("{line} {word}")
        };
        if width(&joined) <= w {
            line = joined;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word;
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(cover: bool) -> Show {
        Show::Track {
            cover,
            playing: true,
            line_w: 30,
        }
    }

    #[test]
    fn forms_go_from_the_small_cover_down_to_one_line() {
        let forms = music_forms(&track(true), Place::Side);
        let kinds: Vec<u16> = forms.iter().map(|f| f.variant).collect();
        assert_eq!(kinds, [V_COVER_SIDE | 12, V_CARD, V_COMPACT, V_LINE]);
        assert_eq!(forms[0].size.width, 12 + 2 + 18);
        assert_eq!(forms[0].size.height, 6);
        let last = forms.last().unwrap();
        assert_eq!(
            (last.size.width, last.size.height, last.fill),
            (30, 1, false)
        );
        // The times tick while playing (a frozen lamp wakes for them); the
        // one-line form has none.
        assert!(forms[..forms.len() - 1].iter().all(|f| f.seconds));
        assert!(!last.seconds);
        // No cover (16 colours, the cover widget placed, `art.inline` off,
        // or a track without one): no cover forms.
        let plain = music_forms(&track(false), Place::Overlay);
        assert_eq!(plain.len(), 3);
        assert_eq!(plain[0].variant, V_CARD);
    }

    #[test]
    fn messages_wrap_to_their_place() {
        let long = "Allow control of Spotify: System Settings › Privacy & Security \
                    › Automation › your terminal › Spotify";
        for (place, w) in [(Place::Side, 20), (Place::Overlay, 30)] {
            let forms = music_forms(
                &Show::Message(message_text(long, &crate::glyphs::RICH)),
                place,
            );
            assert_eq!(forms.len(), 1);
            assert!(forms[0].size.width <= w, "{forms:?}");
            assert!(forms[0].size.height >= 3);
        }
        let short = music_forms(
            &Show::Message(message_text("Spotify isn't running", &crate::glyphs::RICH)),
            Place::Overlay,
        );
        assert_eq!(short[0].size, ratatui::layout::Size::new(23, 1));
    }

    #[test]
    fn fit_and_wrap_respect_display_width() {
        assert_eq!(fit("Slow Rise", 20), "Slow Rise");
        assert_eq!(fit("Convection (Long Version)", 12), "Convection…");
        assert_eq!(width(&fit("夜に駆ける", 5)), 5);
        assert_eq!(fit("夜に駆ける", 5), "夜に…");
        assert_eq!(fit("abc", 0), "");
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("supercalifragilistic", 6), ["super…"]);
        assert_eq!(wrap("", 6), [""]);
        for line in wrap("a b c d e f g h i j k l m n o p", 5) {
            assert!(width(&line) <= 5);
        }
    }

    #[test]
    fn times_read_like_a_player() {
        use std::time::Duration;
        assert_eq!(clock(Duration::from_secs(42)), "0:42");
        assert_eq!(clock(Duration::from_secs(214)), "3:34");
        assert_eq!(clock(Duration::from_secs(3723)), "1:02:03");
    }
}
