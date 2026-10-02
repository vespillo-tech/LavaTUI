//! Lyrics: the playing track's words, the current line bright and bold,
//! the lines around it dim, from LRCLIB (`src/lyrics/`, state in
//! `app/model/lyrics.rs`). Off by default: placing it (`y`) is the opt-in
//! to sending the track's title, artist, album and length to lrclib.net.
//!
//! Forms, most preferred first (W = the song's widest line, 20..=56 on
//! the lava; the side panel's width beside the lamp):
//!
//! | form | size | shows |
//! |---|---|---|
//! | five | W × 5 | two lines back, the current line, two ahead |
//! | three | W × 3 | one back, the current line, one ahead |
//! | narrower | 36 / 24 × 3 | the same, for a small lamp |
//! | line | W / 36 / 24 × 1 | the current line alone |
//!
//! Sizes are fixed per song, so nothing jumps from line to line. A current
//! line wider than the form wraps onto the row below (in the 3- and 5-row
//! forms; the next line makes way), others are cut with `…`. A new line
//! brightens over [`FADE`](crate::app::model) while the old one dims; a
//! seek cuts. Gaps (and the intro) show three dots filling as it passes.
//! Plain (untimed) lyrics scroll with the track's progress, unhighlighted.
//! Everything else is one calm dim sentence: `♪ instrumental`, `♪ no
//! lyrics for this track`, `♪ lyrics offline`, the player's state.
//!
//! No backing is assumed: the text is role colours over whatever is
//! behind it (bold `text` current line, `dim` neighbours).

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};

use super::music::{fit, width, wrap};
use super::{Anchor, ChipText, DockWidget, Look, Place, WidgetForm, align_x};
use crate::app::{Fetch, Model};
use crate::lyrics::Lyrics as Words;
use crate::media::Status;
use crate::theme::{Ink, Role};

pub struct Lyrics;

/// Narrowest and widest form on the lava.
const W: (u16, u16) = (20, 56);
/// Narrower fallbacks for small lamps.
const NARROW: [u16; 2] = [36, 24];
/// The side panel's narrowest inside.
const SIDE_MIN: u16 = 20;
const CHIP_MAX: u16 = 32;
/// Messages wrap at this width: panel, lava.
const MESSAGE_W: (u16, u16) = (20, 30);

/// `WidgetForm::variant`: the kind high, the row count low.
const V_LINES: u16 = 0x100;
const V_MESSAGE: u16 = 0x200;

/// What there is to show (pure input to [`lyrics_forms`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Show {
    /// Lines (synced or plain); `widest` sizes the forms.
    Lines {
        widest: u16,
    },
    Message(String),
}

fn show(model: &Model) -> Show {
    let message = |text: &str| Show::Message(text.into());
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
        None | Some(Fetch::Looking) => message("looking for lyrics…"),
        Some(Fetch::NotFound) => message("no lyrics for this track"),
        Some(Fetch::Offline) => message("lyrics offline"),
        Some(Fetch::Lyrics(Words::Instrumental)) => message("instrumental"),
        Some(Fetch::Lyrics(_)) => Show::Lines {
            widest: model.lyrics.widest,
        },
    }
}

/// The forms for `show` in `place`, most preferred first (pure).
pub fn lyrics_forms(show: &Show, place: Place) -> Vec<WidgetForm> {
    match show {
        Show::Message(text) => {
            let w = match place {
                Place::Overlay => MESSAGE_W.1,
                _ => MESSAGE_W.0,
            };
            let lines = wrap(&message_text(text), w);
            let w = lines.iter().map(|l| width(l)).max().unwrap_or(1).max(1);
            vec![WidgetForm::fixed(w, lines.len() as u16, V_MESSAGE)]
        }
        &Show::Lines { widest } => {
            if place != Place::Overlay {
                return [5, 3, 1]
                    .map(|rows| WidgetForm::fill(SIDE_MIN, rows, V_LINES | rows))
                    .to_vec();
            }
            let full = widest.clamp(W.0, W.1);
            let mut widths = vec![full];
            widths.extend(NARROW.into_iter().filter(|&n| n < full));
            let mut forms = vec![
                WidgetForm::fixed(full, 5, V_LINES | 5),
                WidgetForm::fixed(full, 3, V_LINES | 3),
            ];
            forms.extend(
                widths[1..]
                    .iter()
                    .map(|&w| WidgetForm::fixed(w, 3, V_LINES | 3)),
            );
            forms.extend(widths.iter().map(|&w| WidgetForm::fixed(w, 1, V_LINES | 1)));
            forms
        }
    }
}

fn message_text(text: &str) -> String {
    format!("♪ {text}")
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
        let lines = matches!(show(model), Show::Lines { .. });
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
        match (form.variant & 0xff00, show(model)) {
            (V_MESSAGE, Show::Message(text)) => {
                let dim = model.theme.text(Role::Dim);
                for (i, line) in wrap(&message_text(&text), area.width).iter().enumerate() {
                    pen.text(i as u16, line, dim);
                }
            }
            (V_LINES, Show::Lines { .. }) => match &model.lyrics.found {
                Some(Fetch::Lyrics(Words::Synced(_))) => pen.synced(),
                Some(Fetch::Lyrics(Words::Plain(lines))) => pen.plain(lines),
                _ => {}
            },
            _ => {}
        }
    }

    /// The current line while playing synced lyrics (`♪` in a gap).
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
        Some(ChipText {
            text: fit(format!("♪ {line}").trim_end(), CHIP_MAX),
            ink: Role::Text,
        })
    }
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
        if dy >= self.area.height {
            return;
        }
        let text = fit(text, self.area.width);
        let x = self.area.x + align_x(self.align, self.area.width, width(&text));
        self.buf.set_stringn(
            x,
            self.area.y + dy,
            &text,
            usize::from(self.area.width),
            style,
        );
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

    /// Lines around the current one, the current one bold and bright.
    fn synced(&mut self) {
        let model = self.model;
        let state = &model.lyrics;
        let (Some(synced), Some(cursor)) = (state.synced(), state.cursor) else {
            return;
        };
        let rows = self.area.height;
        let mid = rows / 2;
        let k = state.fade(model.now);
        let was = state.changed.and_then(|(_, before)| before);
        let at = cursor.index.map_or(-1, |i| i as i64);
        let line = |i: i64| usize::try_from(i).ok().and_then(|i| synced.lines.get(i));

        let cur_rows = match cursor.current(synced).filter(|l| !l.is_gap()) {
            Some(current) => {
                let bright = self.tint(k).add_modifier(Modifier::BOLD);
                let parts = self.current_rows(&current.text, rows >= 3);
                for (i, part) in parts.iter().enumerate() {
                    self.text(mid + i as u16, part, bright);
                }
                parts.len() as u16
            }
            // A gap, or the intro.
            None => {
                self.dots(mid, cursor.progress);
                1
            }
        };
        let dim = model.theme.text(Role::Dim);
        // Before it, going up (the line just left fades down from bright)…
        for dy in 1..=mid {
            let i = at - i64::from(dy);
            let Some(before) = line(i) else { break };
            let style = if was.is_some_and(|w| w as i64 == i) && k < 1.0 {
                self.tint(1.0 - k)
            } else {
                dim
            };
            self.text(mid - dy, &before.text, style);
        }
        // …and after it, going down.
        for (n, y) in (mid + cur_rows..rows).enumerate() {
            let Some(after) = line(at + 1 + n as i64) else {
                break;
            };
            self.text(y, &after.text, dim);
        }
    }

    /// The current line as one row, or two when it's too wide and `two`
    /// rows are free (the second cut with `…` if still too wide).
    fn current_rows(&self, text: &str, two: bool) -> Vec<String> {
        let w = self.area.width;
        if !two || width(text) <= w {
            return vec![fit(text, w)];
        }
        let first = wrap(text, w).swap_remove(0);
        let words: Vec<&str> = text.split_whitespace().collect();
        let taken = first.split_whitespace().count().max(1);
        let rest = words.get(taken..).unwrap_or_default().join(" ");
        if rest.is_empty() {
            vec![first]
        } else {
            vec![first, fit(&rest, w)]
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

    /// Plain lyrics: a window that moves with the track's progress.
    fn plain(&mut self, lines: &[String]) {
        let model = self.model;
        let Some(snap) = model.music.snapshot.as_ref() else {
            return;
        };
        let progress = snap.progress_at(model.now).unwrap_or(0.0);
        let rows = usize::from(self.area.height);
        let centre = ((progress.clamp(0.0, 1.0) * lines.len() as f64) as usize)
            .min(lines.len().saturating_sub(1));
        let first = centre.saturating_sub(rows / 2);
        let (text, dim) = (model.theme.text(Role::Text), model.theme.text(Role::Dim));
        for (dy, i) in (first..lines.len()).take(rows).enumerate() {
            let style = if i == centre { text } else { dim };
            self.text(dy as u16, &lines[i], style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Size;

    #[test]
    fn forms_on_the_lava_shrink_to_one_line() {
        let forms = lyrics_forms(&Show::Lines { widest: 44 }, Place::Overlay);
        let sizes: Vec<(u16, u16)> = forms
            .iter()
            .map(|f| (f.size.width, f.size.height))
            .collect();
        assert_eq!(
            sizes,
            [
                (44, 5),
                (44, 3),
                (36, 3),
                (24, 3),
                (44, 1),
                (36, 1),
                (24, 1)
            ]
        );
        assert!(forms.iter().all(|f| !f.fill && !f.seconds));
        // Short songs: never narrower than 20; long lines: never wider than 56.
        let narrow = lyrics_forms(&Show::Lines { widest: 9 }, Place::Overlay);
        assert_eq!(narrow[0].size, Size::new(20, 5));
        assert_eq!(narrow.len(), 3, "{narrow:?}");
        let wide = lyrics_forms(&Show::Lines { widest: 200 }, Place::Overlay);
        assert_eq!(wide[0].size, Size::new(56, 5));
    }

    #[test]
    fn side_forms_fill_the_panel() {
        let forms = lyrics_forms(&Show::Lines { widest: 44 }, Place::Side);
        assert!(forms.iter().all(|f| f.fill && f.size.width == SIDE_MIN));
        let rows: Vec<u16> = forms.iter().map(|f| f.size.height).collect();
        assert_eq!(rows, [5, 3, 1]);
    }

    #[test]
    fn messages_are_one_calm_form() {
        for place in [Place::Side, Place::Overlay] {
            let forms = lyrics_forms(&Show::Message("no lyrics for this track".into()), place);
            assert_eq!(forms.len(), 1);
            assert!(forms[0].size.width <= 30);
        }
        let short = lyrics_forms(&Show::Message("instrumental".into()), Place::Overlay);
        assert_eq!(short[0].size, Size::new(14, 1));
    }
}
