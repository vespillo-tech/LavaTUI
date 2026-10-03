//! Lyrics: the playing track's words, the current line karaoke style
//! (the words sung so far bold `text`, the one being sung bold `accent`,
//! the rest `dim`), the lines around it dim, from LRCLIB (`src/lyrics/`, state in
//! `app/model/lyrics.rs`). Off by default: placing it (`y`) is the opt-in
//! to sending the track's title, artist, album and length to lrclib.net.
//!
//! **The line being sung is always shown whole** (design §4.6, lava-uqi).
//! Every form is sized by the song, once, when its lyrics arrive
//! ([`Sizing`]): it keeps `R` rows for the current line, the most rows
//! any of the song's lines wraps onto (between words) at the form's
//! width. Sizes are fixed per song, so nothing jumps from line to line.
//! Forms, most preferred first:
//!
//! | form | on the lava | beside the lamp | shows |
//! |---|---|---|---|
//! | five | W × (4 + R) | fill × (4 + R) | two lines back, the current line, two ahead |
//! | three | W, 36, 24 × (2 + R) | fill × (2 + R) | one back, the current line, one ahead |
//! | line | W, 36, 24 × R | fill × R | the current line alone |
//!
//! On the lava W is the song's widest line (20..=56), and 36 / 24 are for
//! small lamps. Beside the lamp a form fills the panel's width; each
//! comes at the narrowest widths (≥ 20) that hold every line in one, two
//! and three rows, and at 20 (in as many rows as that takes), so the
//! panel picks the shortest form its width allows (and grows for it from
//! 200 cols up, like for a big clock face).
//!
//! The current line starts on the row under the lines kept for those
//! before it; when it takes fewer rows than its form keeps, the lines
//! after it move up. A line around it shows whole when it fits in the
//! rows left, else on one row cut after a word with `…` (never mid-word,
//! but for a single word wider than the form). Words light one by one as
//! they're sung (times from `lyrics::words`: exact from word tags, else
//! estimated); the line just left dims over [`FADE`](crate::app::model);
//! a seek cuts. Gaps
//! (and the intro) show three dots filling as it passes. Plain (untimed)
//! lyrics scroll with the track's progress, unhighlighted.
//! Everything else is one calm dim sentence: `♪ instrumental`, `♪ no
//! lyrics on lrclib.net for this song` (`♪ not on lrclib.net` where that
//! doesn't fit), `♪ lyrics offline`, the player's state.
//!
//! No backing is assumed: the text is role colours over whatever is
//! behind it. Without colours the word being sung is underlined too.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::music::{fit, width, wrap};
use super::{Anchor, ChipText, DockWidget, Look, Place, WidgetForm, align_x};
use crate::app::{Fetch, Model};
use crate::lyrics::Lyrics as Words;
use crate::lyrics::lrc::Line;
use crate::lyrics::sync::Cursor;
use crate::media::Status;
use crate::theme::{Ink, Role};

pub struct Lyrics;

/// Narrowest and widest form on the lava.
const W: (u16, u16) = (20, 56);
/// Narrower fallbacks for small lamps.
const NARROW: [u16; 2] = [36, 24];
/// The side panel's narrowest and widest inside (`ui::layout`'s
/// `PANEL_W.0 - 2` and `PANEL_W_WIDE - 2`).
const SIDE_W: (u16, u16) = (20, 54);
/// Beside the lamp, forms come at the narrowest width that holds the
/// current line in up to this many rows (and at the narrowest panel).
const SIDE_ROWS: u16 = 3;
const CHIP_MAX: u16 = 32;
/// Messages wrap at this width: panel, lava.
const MESSAGE_W: (u16, u16) = (20, 30);

/// `WidgetForm::variant`: the kind high; for lines, the rows kept for the
/// lines before the current one low; for messages, which wording (low).
const V_LINES: u16 = 0x100;
const V_MESSAGE: u16 = 0x200;

/// What a song's forms are sized by, worked out once when its lyrics
/// arrive (`LyricsState::sizing`): its widest line, and how many rows its
/// longest line wraps onto at each width a form can have.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sizing {
    /// The widest line, in columns.
    pub widest: u16,
    /// The most rows a line takes at each width from `W.0` to `W.1`.
    rows: Vec<u16>,
}

impl Sizing {
    pub fn of<'a>(lines: impl IntoIterator<Item = &'a str>) -> Self {
        let lines: Vec<&str> = lines.into_iter().filter(|l| !l.trim().is_empty()).collect();
        let widths: Vec<u16> = lines.iter().map(|l| width(l)).collect();
        let widest = widths.iter().copied().max().unwrap_or(0);
        // Once per song, on the frame that gets them: only lines wider
        // than `w` are wrapped.
        let rows = (W.0..=W.1)
            .map(|w| {
                let rows = |(l, &lw): (&&str, &u16)| if lw <= w { 1 } else { wrap(l, w).len() };
                let most = lines.iter().zip(&widths).map(rows).max();
                most.unwrap_or(1).min(usize::from(u16::MAX)) as u16
            })
            .collect();
        Self { widest, rows }
    }

    /// The most rows any line takes, wrapped at `w` columns (`w` ≥ 20).
    pub fn rows(&self, w: u16) -> u16 {
        if w >= self.widest {
            return 1;
        }
        let at = usize::from(w.clamp(W.0, W.1) - W.0);
        self.rows.get(at).copied().unwrap_or(1).max(1)
    }
}

/// What there is to show (pure input to [`lyrics_forms`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Show<'a> {
    /// Lines (synced or plain), sized by the song.
    Lines(&'a Sizing),
    /// One calm sentence: the wordings, most preferred first (a shorter
    /// one for rooms the first doesn't fit).
    Message(Vec<String>),
}

fn show(model: &Model) -> Show<'_> {
    // With its `♪`, when the glyphs have one.
    let note = model.glyphs().note;
    let messages =
        |texts: &[&str]| Show::Message(texts.iter().map(|text| format!("{note}{text}")).collect());
    let message = |text: &str| messages(&[text]);
    let Some(snap) = model.music.snapshot.as_ref() else {
        return message("…");
    };
    match (&snap.status, &snap.track) {
        (Status::Unavailable(reason), _) => {
            return message(&reason.message_for(snap.player_name(), "lyrics"));
        }
        (Status::Connecting, _) => return message("…"),
        (Status::Stopped, _) | (_, None) => return message("nothing playing"),
        _ => {}
    }
    match &model.lyrics.found {
        None | Some(Fetch::Looking) => message("asking lrclib.net for lyrics…"),
        // Say whose shelf is bare: the lyrics site, not this app.
        Some(Fetch::NotFound) => {
            messages(&["no lyrics on lrclib.net for this song", "not on lrclib.net"])
        }
        Some(Fetch::Offline) => message("lyrics offline"),
        Some(Fetch::Lyrics(Words::Instrumental)) => message("instrumental"),
        Some(Fetch::Lyrics(_)) => Show::Lines(&model.lyrics.sizing),
    }
}

/// The forms for `show` in `place`, most preferred first (pure).
pub fn lyrics_forms(show: &Show, place: Place) -> Vec<WidgetForm> {
    match show {
        Show::Message(texts) => {
            let w = match place {
                Place::Overlay => MESSAGE_W.1,
                _ => MESSAGE_W.0,
            };
            texts
                .iter()
                .zip(0..)
                .map(|(text, n)| {
                    let lines = wrap(text, w);
                    let w = lines.iter().map(|l| width(l)).max().unwrap_or(1).max(1);
                    WidgetForm::fixed(w, lines.len() as u16, V_MESSAGE | n)
                })
                .collect()
        }
        Show::Lines(sizing) if place == Place::Overlay => lava_forms(sizing),
        Show::Lines(sizing) => side_forms(sizing),
    }
}

/// A form `w` wide with `back` lines before the current one, as many
/// after, and `rows` for the current one.
fn lines_form(w: u16, back: u16, rows: u16, fill: bool) -> WidgetForm {
    let (h, variant) = (2 * back + rows, V_LINES | back);
    if fill {
        WidgetForm::fill(w, h, variant)
    } else {
        WidgetForm::fixed(w, h, variant)
    }
}

/// On the lava: five at the song's width, then three and the line alone
/// at it and the narrower widths.
fn lava_forms(s: &Sizing) -> Vec<WidgetForm> {
    let full = s.widest.clamp(W.0, W.1);
    let mut widths = vec![full];
    widths.extend(NARROW.into_iter().filter(|&n| n < full));
    let mut forms = vec![lines_form(full, 2, s.rows(full), false)];
    for back in [1, 0] {
        forms.extend(
            widths
                .iter()
                .map(|&w| lines_form(w, back, s.rows(w), false)),
        );
    }
    forms
}

/// Beside the lamp: five, three and the line alone, each at the widths
/// of [`side_widths`] (filling the panel).
fn side_forms(s: &Sizing) -> Vec<WidgetForm> {
    let widths = side_widths(s);
    [2, 1, 0]
        .into_iter()
        .flat_map(|back| {
            widths
                .iter()
                .map(move |&w| lines_form(w, back, s.rows(w), true))
        })
        .collect()
}

/// The narrowest panel insides that hold every line in one, two and
/// three rows, widest first, then the narrowest panel's (in as many rows
/// as that takes): wider than one of these only spares rows, which the
/// lines after the current one use.
fn side_widths(s: &Sizing) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    for rows in 1..=SIDE_ROWS {
        let narrowest = (SIDE_W.0..=SIDE_W.1).find(|&w| s.rows(w) <= rows);
        if let Some(w) = narrowest.filter(|w| !out.contains(w)) {
            out.push(w);
        }
    }
    if !out.contains(&SIDE_W.0) {
        out.push(SIDE_W.0);
    }
    out
}

impl DockWidget for Lyrics {
    fn name(&self) -> &'static str {
        "lyrics"
    }

    /// Off: placing it is the opt-in to lookups on lrclib.net.
    fn default_place(&self) -> Place {
        Place::Off
    }

    /// Bottom centre, under the lamp's middle, clear of the clock.
    fn default_anchor(&self) -> Anchor {
        Anchor::Bottom
    }

    /// Lines while playing 2 (as music), paused 1, otherwise 0.
    fn rank(&self, model: &Model) -> u8 {
        let lines = matches!(show(model), Show::Lines(_));
        match model.music.snapshot.as_ref().map(|s| &s.status) {
            Some(Status::Playing) if lines => 2,
            Some(Status::Paused) if lines => 1,
            _ => 0,
        }
    }

    fn forms(&self, model: &Model, place: Place) -> Vec<WidgetForm> {
        lyrics_forms(&show(model), place)
    }

    fn draw(&self, model: &Model, form: WidgetForm, area: Rect, look: Look, buf: &mut Buffer) {
        let mut pen = Pen {
            model,
            buf,
            area,
            align: look.align,
        };
        let back = form.variant & 0xff;
        match (form.variant & 0xff00, show(model)) {
            (V_MESSAGE, Show::Message(texts)) => {
                let dim = model.theme.text(Role::Dim);
                let text = texts.get(usize::from(form.variant & 0xff));
                for (i, line) in wrap(text.map_or("", String::as_str), area.width)
                    .iter()
                    .enumerate()
                {
                    pen.text(i as u16, line, dim);
                }
            }
            (V_LINES, Show::Lines(_)) => match &model.lyrics.found {
                Some(Fetch::Lyrics(Words::Synced(_))) => pen.synced(back),
                Some(Fetch::Lyrics(Words::Plain(lines))) => pen.plain(lines, back),
                _ => {}
            },
            _ => {}
        }
    }

    /// The current line while playing synced lyrics (`♪` in a gap), cut
    /// after a word if it's long.
    fn chip(&self, model: &Model) -> Option<ChipText> {
        let snap = model.music.snapshot.as_ref()?;
        if snap.status != Status::Playing {
            return None;
        }
        let synced = model.lyrics.synced()?;
        let line = model
            .lyrics
            .cursor?
            .current(synced)
            .map_or("", |l| l.text.as_str());
        let text = format!("{}{line}", model.glyphs().note);
        let text = text.trim_end();
        // A gap with no note to show: no chip.
        (!text.is_empty()).then(|| ChipText {
            text: cut(text, CHIP_MAX),
            ink: Role::Text,
        })
    }
}

/// `text` on one row of `w`: whole if it fits, else cut after the last
/// word that fits (and any `,;:` it ends with), then `…`. Only a first
/// word wider than the row is cut inside.
pub fn cut(text: &str, w: u16) -> String {
    if width(text) <= w {
        return text.to_owned();
    }
    let mut out = String::new();
    for word in text.split_whitespace() {
        let longer = if out.is_empty() {
            word.to_owned()
        } else {
            format!("{out} {word}")
        };
        if width(&longer) + 1 > w {
            break;
        }
        out = longer;
    }
    let kept = out.trim_end_matches([',', ';', ':']).len();
    out.truncate(kept);
    if out.is_empty() {
        return fit(text, w);
    }
    out.push('…');
    out
}

/// A line next to the current one, in at most `free` rows of `w`: whole
/// when it fits, else one row cut after a word.
fn neighbour(text: &str, w: u16, free: u16) -> Vec<String> {
    let rows = wrap(text, w);
    if rows.len() <= usize::from(free) {
        rows
    } else {
        vec![cut(text, w)]
    }
}

/// The line being sung in at most `most` rows of `w`: whole (its form
/// keeps the rows for it), else (never, by the sizing) the last row cut
/// after a word.
fn whole(text: &str, w: u16, most: u16) -> Vec<String> {
    let most = usize::from(most.max(1));
    let mut rows = wrap(text, w);
    if rows.len() > most {
        let rest = rows[most - 1..].join(" ");
        rows.truncate(most - 1);
        rows.push(cut(&rest, w));
    }
    rows
}

/// [`whole`]'s rows as byte ranges of `text` (one-space separated, as
/// lyrics lines are), wrapped between words as [`wrap`] wraps them (a
/// word wider than `w` alone on its row), the last of `most` holding the
/// rest. Nothing allocated.
fn rows(text: &str, w: u16, most: u16) -> impl Iterator<Item = (usize, usize)> + '_ {
    let w = usize::from(w);
    let mut most = most.max(1);
    let mut at = 0;
    std::iter::from_fn(move || {
        if at >= text.len() {
            return None;
        }
        let start = at;
        let mut end = text.len();
        if most > 1 {
            most -= 1;
            end = start;
            let mut used = 0;
            for word in text[start..].split(' ') {
                let ww = word.width().min(w);
                if end > start && used + 1 + ww > w {
                    break;
                }
                used += ww + usize::from(end > start);
                end += word.len() + usize::from(end > start);
            }
        }
        at = end + 1;
        Some((start, end))
    })
}

/// The middle of [`Pen::around`]: the current line, or a gap's dots.
enum Current<'t> {
    Line(&'t str, Style),
    /// A synced line, word by word.
    Karaoke(&'t Line, Cursor),
    Dots(f32),
}

struct Pen<'a, 'b> {
    model: &'a Model,
    buf: &'b mut Buffer,
    area: Rect,
    align: Alignment,
}

impl Pen<'_, '_> {
    /// `text` on row `dy`, cut to fit, lined up by the look.
    fn text(&mut self, dy: u16, text: &str, style: Style) {
        self.row(dy, text, |_| style);
    }

    /// `text` on row `dy`, lined up by the look, each character in
    /// `style(its byte offset)`, cut with `…` as [`fit`] cuts if too
    /// wide. Drawn in runs of one style straight from `text`: nothing
    /// allocated.
    fn row(&mut self, dy: u16, text: &str, style: impl Fn(usize) -> Style) {
        if dy >= self.area.height || self.area.width == 0 {
            return;
        }
        let w = usize::from(self.area.width);
        let to = if text.width() <= w {
            text.len()
        } else {
            let mut to = 0;
            let mut used = 0;
            for (i, c) in text.char_indices() {
                let cw = c.width().unwrap_or(0);
                if used + cw + 1 > w {
                    break;
                }
                used += cw;
                to = i + c.len_utf8();
            }
            text[..to].trim_end().len()
        };
        let cut = to < text.len();
        let drawn = text[..to].width() + usize::from(cut);
        let mut x = self.area.x + align_x(self.align, self.area.width, drawn as u16);
        let (y, right) = (self.area.y + dy, self.area.right());
        let mut start = 0;
        while start < to {
            let run = style(start);
            let end = text[start..to]
                .char_indices()
                .find(|&(i, _)| style(start + i) != run)
                .map_or(to, |(i, _)| start + i);
            let room = usize::from(right.saturating_sub(x));
            x = self.buf.set_stringn(x, y, &text[start..end], room, run).0;
            start = end;
        }
        if cut && x < right {
            self.buf.set_stringn(x, y, "…", 1, style(to));
        }
    }

    /// The current synced line from row `dy`, whole in at most `most`
    /// rows, karaoke style: the words sung so far bold `text`, the one
    /// being sung bold `accent` (underlined too where there are no
    /// colours to tell it by), the rest `dim` (not bold). Returns the rows
    /// used.
    fn karaoke(&mut self, dy: u16, line: &Line, cursor: Cursor, most: u16) -> u16 {
        let theme = &self.model.theme;
        let sung = theme.text(Role::Text).add_modifier(Modifier::BOLD);
        let mut now = theme.text(Role::Accent).add_modifier(Modifier::BOLD);
        if !theme.has_color() {
            now = now.add_modifier(Modifier::UNDERLINED);
        }
        let ahead = theme.text(Role::Dim);
        let words = &line.words;
        let style = |at: usize| {
            // The word `at` is in (spaces between words: plain).
            let i = words.partition_point(|w| w.end as usize <= at);
            match words.get(i) {
                Some(w) if w.start as usize <= at => match cursor.word {
                    Some(c) if c == i => now,
                    _ if i < cursor.sung => sung,
                    _ => ahead,
                },
                _ => Style::new(),
            }
        };
        let text = line.text.as_str();
        let mut used = 0;
        for (start, end) in rows(text, self.area.width, most) {
            self.row(dy + used, &text[start..end], |at| style(start + at));
            used += 1;
        }
        used
    }

    /// `dim` → `text` by `k` (0..=1), as a plain foreground.
    fn tint(&self, k: f32) -> Style {
        let theme = &self.model.theme;
        let fg = theme
            .paint(Ink::Role(Role::Dim))
            .mix(Ink::Role(Role::Text), k)
            .color();
        Style::new().fg(fg)
    }

    /// Synced lyrics: the current line word by word from row `back`,
    /// the lines around it dim (the one just left fading down).
    fn synced(&mut self, back: u16) {
        let model = self.model;
        let state = &model.lyrics;
        let (Some(synced), Some(cursor)) = (state.synced(), state.cursor) else {
            return;
        };
        let k = state.fade(model.now);
        let was = state.changed.and_then(|(_, before)| before);
        let at = cursor.index.map_or(-1, |i| i as i64);
        let line = |i: i64| {
            let line = usize::try_from(i).ok().and_then(|i| synced.lines.get(i))?;
            Some(line.text.as_str())
        };
        let current = match cursor.current(synced).filter(|l| !l.is_gap()) {
            Some(current) => Current::Karaoke(current, cursor),
            // A gap, or the intro.
            None => Current::Dots(cursor.progress),
        };
        let (dim, fading) = (model.theme.text(Role::Dim), self.tint(1.0 - k));
        let before = |n: u16| {
            let i = at - i64::from(n);
            let left = was.is_some_and(|w| w as i64 == i) && k < 1.0;
            line(i).map(|text| (text, if left { fading } else { dim }))
        };
        let after = |n: u16| line(at + i64::from(n)).map(|text| (text, dim));
        self.around(back, current, before, after);
    }

    /// The lines around the current one: those before it in the `back`
    /// rows above it (nearest first, going up), the current one whole
    /// from row `back`, those after it below. A neighbour shows whole when
    /// it fits in the rows left, else on one row cut after a word.
    fn around<'t>(
        &mut self,
        back: u16,
        current: Current<'t>,
        before: impl Fn(u16) -> Option<(&'t str, Style)>,
        after: impl Fn(u16) -> Option<(&'t str, Style)>,
    ) {
        let (w, rows) = (self.area.width, self.area.height);
        let back = back.min(rows.saturating_sub(1));
        let used = match current {
            Current::Line(text, style) => {
                let parts = whole(text, w, rows - back);
                for (i, part) in parts.iter().enumerate() {
                    self.text(back + i as u16, part, style);
                }
                parts.len() as u16
            }
            Current::Karaoke(line, cursor) => self.karaoke(back, line, cursor, rows - back),
            Current::Dots(progress) => {
                self.dots(back, progress);
                1
            }
        };
        let mut top = back;
        for n in 1.. {
            let Some((text, style)) = before(n).filter(|_| top > 0) else {
                break;
            };
            let parts = neighbour(text, w, top);
            top -= parts.len() as u16;
            for (i, part) in parts.iter().enumerate() {
                self.text(top + i as u16, part, style);
            }
        }
        let mut y = back + used;
        for n in 1.. {
            let Some((text, style)) = after(n).filter(|_| y < rows) else {
                break;
            };
            let parts = neighbour(text, w, rows - y);
            for (i, part) in parts.iter().enumerate() {
                self.text(y + i as u16, part, style);
            }
            y += parts.len() as u16;
        }
    }

    /// `•  •  •` on row `dy`, lit (`text`) one by one as `progress` passes.
    fn dots(&mut self, dy: u16, progress: f32) {
        const DOTS: u16 = 7;
        if dy >= self.area.height || DOTS > self.area.width {
            return;
        }
        let x0 = self.area.x + align_x(self.align, self.area.width, DOTS);
        let lit = (progress.clamp(0.0, 1.0) * 3.0).floor() as u16;
        let theme = &self.model.theme;
        for n in 0..3 {
            let style = theme.text(if n < lit { Role::Text } else { Role::Dim });
            self.buf[(x0 + n * 3, self.area.y + dy)]
                .set_char('•')
                .set_style(style);
        }
    }

    /// Plain lyrics: the line as far through them as the track is, from
    /// row `back`, `text` but not bold, the rest dim.
    fn plain(&mut self, lines: &[String], back: u16) {
        let model = self.model;
        let Some(snap) = model.music.snapshot.as_ref() else {
            return;
        };
        let progress = snap.progress_at(model.now).unwrap_or(0.0);
        let centre = ((progress.clamp(0.0, 1.0) * lines.len() as f64) as usize)
            .min(lines.len().saturating_sub(1));
        let Some(middle) = lines.get(centre) else {
            return;
        };
        let (text, dim) = (model.theme.text(Role::Text), model.theme.text(Role::Dim));
        let line = |i: Option<usize>| i.and_then(|i| lines.get(i)).map(|l| (l.as_str(), dim));
        self.around(
            back,
            Current::Line(middle, text),
            |n| line(centre.checked_sub(usize::from(n))),
            |n| line(Some(centre + usize::from(n))),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Size;

    /// A made-up song (the demo's first), widest line 47.
    pub const SONG: &[&str] = &[
        "Down at the bottom where the warm light grows",
        "A little wax is waking, and it slowly goes",
        "",
        "Slow rise, slow rise",
        "Cooling at the top and coming down to try again",
        "And then it starts to rise again",
    ];

    fn sizes(forms: &[WidgetForm]) -> Vec<(u16, u16)> {
        forms
            .iter()
            .map(|f| (f.size.width, f.size.height))
            .collect()
    }

    #[test]
    fn karaoke_rows_wrap_as_wrap_does() {
        let lines = SONG.iter().copied().chain([
            "Supercalifragilisticexpialidocious is a word too wide",
            "君の名は yeah 사랑해",
        ]);
        for text in lines.filter(|l| !l.is_empty()) {
            for w in 8..60 {
                let ours: Vec<&str> = rows(text, w, u16::MAX).map(|(a, b)| &text[a..b]).collect();
                let theirs: Vec<String> = wrap(text, w)
                    .into_iter()
                    .map(|r| r.trim_end_matches('…').to_owned())
                    .collect();
                assert_eq!(ours.len(), theirs.len(), "{text:?} at {w}");
                for (o, t) in ours.iter().zip(&theirs) {
                    assert!(
                        o.starts_with(t.as_str()),
                        "{text:?} at {w}: {ours:?} vs {theirs:?}"
                    );
                }
            }
            // At most `most` rows, the last holding the rest.
            let two: Vec<_> = rows(text, 10, 2).collect();
            assert!(two.len() <= 2);
            assert_eq!(two.last().unwrap().1, text.len());
        }
    }

    #[test]
    fn sizing_counts_the_rows_the_longest_line_wraps_onto() {
        let s = Sizing::of(SONG.iter().copied());
        assert_eq!(s.widest, 47);
        for w in W.0..=W.1 {
            let most = SONG.iter().map(|l| wrap(l, w).len() as u16).max().unwrap();
            assert_eq!(s.rows(w), most, "at {w}");
            // Wider never takes more rows.
            assert!(s.rows(w + 1) <= s.rows(w), "at {w}");
        }
        assert_eq!((s.rows(47), s.rows(46), s.rows(20)), (1, 2, 3));
        assert_eq!(Sizing::default().rows(20), 1);
    }

    #[test]
    fn forms_on_the_lava_keep_rows_for_the_whole_line() {
        let s = Sizing::of(SONG.iter().copied());
        let forms = lyrics_forms(&Show::Lines(&s), Place::Overlay);
        // 47 holds every line on one row; 36 and 24 need two.
        assert_eq!(
            sizes(&forms),
            [
                (47, 5),
                (47, 3),
                (36, 4),
                (24, 4),
                (47, 1),
                (36, 2),
                (24, 2)
            ]
        );
        assert!(forms.iter().all(|f| !f.fill && !f.seconds));
        // Short songs: never narrower than 20; long lines: never wider
        // than 56, wrapped instead.
        let short = Sizing::of(["la la la"]);
        let narrow = lyrics_forms(&Show::Lines(&short), Place::Overlay);
        assert_eq!(narrow[0].size, Size::new(20, 5));
        assert_eq!(narrow.len(), 3, "{narrow:?}");
        let long = "word ".repeat(30);
        let long = Sizing::of([long.as_str()]);
        let wide = lyrics_forms(&Show::Lines(&long), Place::Overlay);
        assert_eq!(wide[0].size, Size::new(56, 4 + 3));
    }

    #[test]
    fn side_forms_come_at_the_narrowest_width_for_each_row_count() {
        let s = Sizing::of(SONG.iter().copied());
        let forms = lyrics_forms(&Show::Lines(&s), Place::Side);
        assert!(forms.iter().all(|f| f.fill));
        let two = (W.0..=47).find(|&w| s.rows(w) <= 2).unwrap();
        assert_eq!(
            sizes(&forms),
            [
                (47, 5),
                (two, 6),
                (20, 7),
                (47, 3),
                (two, 4),
                (20, 5),
                (47, 1),
                (two, 2),
                (20, 3)
            ]
        );
        // A line so long even three rows of the widest panel can't hold
        // it: the narrowest panel, in as many rows as it takes.
        let long = "word ".repeat(40);
        let long = Sizing::of([long.as_str()]);
        let forms = lyrics_forms(&Show::Lines(&long), Place::Side);
        let rows = long.rows(20);
        assert_eq!(sizes(&forms), [(20, rows + 4), (20, rows + 2), (20, rows)]);
    }

    #[test]
    fn cut_ends_after_a_word() {
        assert_eq!(cut("Slow rise, slow rise", 30), "Slow rise, slow rise");
        assert_eq!(cut("Slow rise, slow rise", 12), "Slow rise…");
        assert_eq!(
            cut("Floating like a thought behind your eyes", 28),
            "Floating like a thought…"
        );
        assert_eq!(cut("Unbelievably", 6), "Unbel…");
        for w in 2..=48 {
            for line in SONG {
                let c = cut(line, w);
                assert!(width(&c) <= w, "{c:?} at {w}");
                if let Some(kept) = c.strip_suffix('…').filter(|_| c != *line) {
                    let next = line[kept.len()..].chars().next();
                    let first = !kept.contains(' ');
                    assert!(
                        first || matches!(next, Some(' ' | ',' | ';' | ':')),
                        "{line:?} cut mid-word at {w}: {c:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_current_line_is_whole_in_the_rows_its_form_keeps() {
        let s = Sizing::of(SONG.iter().copied());
        for w in W.0..=W.1 {
            for line in SONG {
                let rows = whole(line, w, s.rows(w));
                assert_eq!(
                    rows.join(" "),
                    line.split_whitespace().collect::<Vec<_>>().join(" ")
                );
            }
        }
        // More than its rows (never offered): the last row cut after a word.
        let rows = whole(SONG[4], 20, 2);
        assert_eq!(rows, ["Cooling at the top", "and coming down to…"]);
        assert_eq!(neighbour(SONG[4], 20, 3).len(), 3);
        assert_eq!(neighbour(SONG[4], 20, 2), ["Cooling at the top…"]);
    }

    fn message(texts: &[&str]) -> Show<'static> {
        Show::Message(texts.iter().map(|t| t.to_string()).collect())
    }

    const NOT_FOUND: [&str; 2] = [
        "♪ no lyrics on lrclib.net for this song",
        "♪ not on lrclib.net",
    ];

    #[test]
    fn messages_are_calm_forms_most_preferred_first() {
        for place in [Place::Side, Place::Overlay] {
            let forms = lyrics_forms(&message(&NOT_FOUND), place);
            assert_eq!(forms.len(), 2);
            assert!(forms.iter().all(|f| f.size.width <= 30));
            assert!(forms[0].size.height > forms[1].size.height);
        }
        let short = lyrics_forms(&message(&["♪ instrumental"]), Place::Overlay);
        assert_eq!(short.len(), 1);
        assert_eq!(short[0].size, Size::new(14, 1));
    }

    #[test]
    fn not_found_names_the_lyrics_site_and_is_never_cut() {
        // The long wording wraps (2 rows on the lava, 3 in a side panel),
        // the short one is a single row; every word survives either way.
        for (place, rows, w) in [(Place::Overlay, 2, 30), (Place::Side, 3, 20)] {
            let forms = lyrics_forms(&message(&NOT_FOUND), place);
            assert_eq!(forms[0].size.height, rows, "{place:?}");
            assert_eq!(forms[1].size, Size::new(19, 1));
            for text in NOT_FOUND {
                let lines = wrap(text, w);
                assert!(lines.iter().all(|l| width(l) <= w), "{lines:?}");
                assert_eq!(lines.join(" "), text);
                assert!(
                    lines.concat().contains("lrclib.net") || lines.join(" ").contains("lrclib.net")
                );
            }
        }
    }
}
