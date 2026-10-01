//! Cover: the playing track's album art on its own, placeable like any
//! widget (`o`: side → lava → off), at the detail and size of `[art]`.
//! Off by default.
//!
//! **Detail** (`art.detail`, `O` cycles it): `pixels` is the real picture
//! through the kitty graphics protocol (kitty, Ghostty; see
//! [`crate::graphics`]); `sextant` (2 × 3 pixels a cell, Unicode 13),
//! `quadrant` (2 × 2) and `halfblock` (1 × 2) draw it in text cells
//! ([`super::picture`]). `auto` picks pixels where the terminal has them,
//! else sextants where it's known to draw them, else quadrants. Text cells
//! need 256 colours or more; pixels any colour at all. With no way to
//! show it the widget is one calm line.
//!
//! **Size** (`art.size`): the largest cover it may be, as columns (rows
//! follow from the cell shape, so it's square): small 16, medium 24, large
//! 34, fill 64. Smaller ones are offered after it, down to 10 columns, and
//! the layout keeps the first that fits; past that it leaves (no chip).
//!
//! Its rank is the music widget's (2 playing, 1 paused, 0 otherwise); it's
//! last in the registry, so it gives way first. A click on it is play /
//! pause. The music card can keep a small cover of its own beside the
//! text (`art.inline`), shown only while this widget is off.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Color;
use serde::{Deserialize, Serialize};

use super::music::{fit, width, wrap};
use super::picture::{self, TextMode};
use super::{Anchor, Backdrop, ChipText, DockWidget, Look, Place, WidgetForm, align_x};
use crate::app::Model;
use crate::graphics;
use crate::media::Status;
use crate::media::art::ArtState;
use crate::theme::{ColorDepth, Ink, Rgb, Role};

pub struct Cover;

/// `[art]` in the config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtSettings {
    pub detail: Detail,
    pub size: CoverSize,
    /// The music card's own small cover, beside the text (while the
    /// cover widget is off).
    pub inline: bool,
}

impl Default for ArtSettings {
    fn default() -> Self {
        Self {
            detail: Detail::Auto,
            size: CoverSize::Medium,
            inline: true,
        }
    }
}

/// `art.detail`: how covers are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    /// The best the terminal can do.
    #[default]
    Auto,
    /// Real pixels (kitty graphics protocol).
    Pixels,
    Sextant,
    Quadrant,
    #[serde(rename = "halfblock")]
    HalfBlock,
}

impl Detail {
    pub const ALL: [Detail; 5] = [
        Detail::Auto,
        Detail::Pixels,
        Detail::Sextant,
        Detail::Quadrant,
        Detail::HalfBlock,
    ];

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&d| d == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            Detail::Auto => "auto",
            Detail::Pixels => "pixels",
            Detail::Sextant => "sextant",
            Detail::Quadrant => "quadrant",
            Detail::HalfBlock => "halfblock",
        }
    }
}

/// `art.size`: the largest the cover widget may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverSize {
    Small,
    #[default]
    Medium,
    Large,
    /// As big as the room allows.
    Fill,
}

impl CoverSize {
    /// The widest cover, in columns.
    pub fn max_cols(self) -> u16 {
        match self {
            CoverSize::Small => 16,
            CoverSize::Medium => 24,
            CoverSize::Large => 34,
            CoverSize::Fill => 64,
        }
    }
}

/// How covers are actually drawn, the setting resolved against the
/// terminal and the colour depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawn {
    Pixels,
    Text(TextMode),
    /// No picture at all (16 colours without pixels, or no colour).
    None,
}

/// What the terminal can show, read once at start ([`crate::graphics`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Caps {
    /// The kitty graphics protocol with Unicode placeholders.
    pub kitty: bool,
    /// Draws Unicode 13 block sextants (its own glyphs, not a font's).
    pub sextants: bool,
}

impl Caps {
    /// From the environment; never in tests (they say what they want).
    pub fn detect() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        let var = |k: &str| std::env::var(k).ok();
        let kitty = graphics::detect(var);
        Self {
            kitty,
            sextants: kitty || sextants(var),
        }
    }
}

/// Terminals known to draw block sextants themselves: WezTerm, foot,
/// Windows Terminal (and kitty / Ghostty, which have pixels anyway).
pub fn sextants(var: impl Fn(&str) -> Option<String>) -> bool {
    let program = var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    let term = var("TERM").unwrap_or_default();
    program == "wezterm"
        || program == "ghostty"
        || term.starts_with("foot")
        || term == "xterm-kitty"
        || var("WT_SESSION").is_some()
}

/// `detail` at this terminal and depth (pure).
pub fn resolve(detail: Detail, caps: Caps, depth: ColorDepth) -> Drawn {
    let text = matches!(depth, ColorDepth::TrueColor | ColorDepth::Ansi256);
    let best_text = if caps.sextants {
        TextMode::Sextant
    } else {
        TextMode::Quadrant
    };
    let as_text = |mode| if text { Drawn::Text(mode) } else { Drawn::None };
    match detail {
        _ if depth == ColorDepth::None => Drawn::None,
        Detail::Pixels => Drawn::Pixels,
        Detail::Auto if caps.kitty => Drawn::Pixels,
        Detail::Auto => as_text(best_text),
        Detail::Sextant => as_text(TextMode::Sextant),
        Detail::Quadrant => as_text(TextMode::Quadrant),
        Detail::HalfBlock => as_text(TextMode::HalfBlock),
    }
}

/// Cover widths offered, widest first (each up to the size's).
const WIDTHS: [u16; 11] = [64, 56, 48, 40, 34, 28, 24, 20, 16, 12, 10];

/// `WidgetForm::variant`.
const V_PICTURE: u16 = 0x100;
const V_MESSAGE: u16 = 0x200;
/// Messages wrap at this width: in the panel (whose narrowest inside is
/// 20), and on the lava.
const MESSAGE_W: (u16, u16) = (20, 30);

/// What the widget shows (pure input to [`cover_forms`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Show {
    /// The cover (or its placeholder while it loads).
    Picture,
    Message(&'static str),
}

fn show(model: &Model) -> Show {
    if model.pictures() == Drawn::None {
        return Show::Message("covers need 256 colours");
    }
    let Some(snap) = model.music.snapshot.as_ref() else {
        return Show::Message("…");
    };
    let track = match (&snap.status, &snap.track) {
        (Status::Playing | Status::Paused, Some(track)) => track,
        (Status::Connecting, _) => return Show::Message("…"),
        _ => return Show::Message("nothing playing"),
    };
    if track.artwork_url.is_empty() || model.music.art() == ArtState::Missing {
        return Show::Message("no cover");
    }
    Show::Picture
}

/// Rows for a square cover `cols` wide, cells `aspect` times as tall as
/// they're wide.
pub fn rows_for(cols: u16, aspect: f64) -> u16 {
    ((f64::from(cols) / aspect.max(0.5)).round() as u16).max(1)
}

/// The forms for `show` at `size`, most preferred first (pure).
pub fn cover_forms(show: &Show, place: Place, size: CoverSize, aspect: f64) -> Vec<WidgetForm> {
    match show {
        Show::Message(text) => {
            let lines = message(text, place);
            let w = lines.iter().map(|l| width(l)).max().unwrap_or(1).max(1);
            vec![WidgetForm::fixed(w, lines.len() as u16, V_MESSAGE)]
        }
        Show::Picture => WIDTHS
            .iter()
            .filter(|&&w| w <= size.max_cols())
            .map(|&w| WidgetForm::fixed(w, rows_for(w, aspect), V_PICTURE))
            .collect(),
    }
}

/// `♪ text`, wrapped for `place`.
fn message(text: &str, place: Place) -> Vec<String> {
    let w = match place {
        Place::Overlay => MESSAGE_W.1,
        _ => MESSAGE_W.0,
    };
    wrap(&format!("♪ {text}"), w)
}

/// Where the picture goes in a placed cover form (its own size, lined up
/// by `align`), if it is the picture form.
pub fn picture_rect(form: WidgetForm, area: Rect, align: Alignment) -> Option<Rect> {
    if form.variant != V_PICTURE {
        return None;
    }
    let w = form.size.width.min(area.width);
    let x = area.x + align_x(align, area.width, w);
    Some(Rect::new(x, area.y, w, form.size.height.min(area.height)))
}

impl DockWidget for Cover {
    fn name(&self) -> &'static str {
        "cover"
    }

    fn default_place(&self) -> Place {
        Place::Off
    }

    /// Top right: across from the music card (top left).
    fn default_anchor(&self) -> Anchor {
        Anchor::TopRight
    }

    /// The music widget's (playing 2, paused 1, otherwise 0), but only
    /// with a picture to show: a message is 0.
    fn rank(&self, model: &Model) -> u8 {
        if show(model) != Show::Picture {
            return 0;
        }
        match model.music.snapshot.as_ref().map(|s| &s.status) {
            Some(Status::Playing) => 2,
            Some(Status::Paused) => 1,
            _ => 0,
        }
    }

    fn forms(&self, model: &Model, place: Place) -> Vec<WidgetForm> {
        cover_forms(
            &show(model),
            place,
            model.settings.art.size,
            model.cell_aspect,
        )
    }

    fn draw(&self, model: &Model, form: WidgetForm, area: Rect, look: Look, buf: &mut Buffer) {
        if let Some(r) = picture_rect(form, area, look.align) {
            draw_cover(model, r, buf);
            return;
        }
        if let Show::Message(text) = show(model) {
            let place = match look.backdrop {
                Backdrop::Lava => Place::Overlay,
                Backdrop::Panel => Place::Side,
            };
            let dim = model.theme.text(Role::Dim);
            for (i, line) in message(text, place).iter().enumerate() {
                if i as u16 >= area.height {
                    break;
                }
                let line = fit(line, area.width);
                let x = area.x + align_x(look.align, area.width, width(&line));
                buf.set_stringn(x, area.y + i as u16, &line, usize::from(area.width), dim);
            }
        }
    }

    /// None: the music chip already names the track.
    fn chip(&self, _model: &Model) -> Option<ChipText> {
        None
    }
}

/// The playing track's cover in `r`, however this terminal draws it best
/// right now: its kitty image once that's all there, else text cells,
/// else (loading, or no colours for it) a quiet placeholder. Shared with
/// the music card's inline cover.
pub(super) fn draw_cover(model: &Model, r: Rect, buf: &mut Buffer) {
    let r = r.intersection(buf.area);
    let url = model
        .music
        .snapshot
        .as_ref()
        .and_then(|s| s.track.as_ref())
        .map(|t| t.artwork_url.as_str())
        .unwrap_or_default();
    let art = match model.music.art() {
        ArtState::Ready(art) => art,
        _ => return placeholder(model, r, buf),
    };
    let text = match model.pictures() {
        Drawn::Pixels => {
            let key = graphics::Key {
                source: url.to_owned(),
                cols: r.width,
                rows: r.height,
            };
            if let Some(id) = model.kitty.ready(&key) {
                let Rgb(red, green, blue) = art.mean();
                let bg = Color::Rgb(red, green, blue);
                return graphics::draw(buf, r, id, bg);
            }
            // Until the picture is all there: the best text cells.
            if model.caps.sextants {
                TextMode::Sextant
            } else {
                TextMode::Quadrant
            }
        }
        Drawn::Text(mode) => mode,
        Drawn::None => return placeholder(model, r, buf),
    };
    if model.theme.shows_images() {
        picture::draw(buf, r, &art, url, text, &model.theme);
    } else {
        placeholder(model, r, buf);
    }
}

/// A quiet tile where the cover will be: `bg` tinted toward `dim`, a dim
/// `♪` in the middle.
fn placeholder(model: &Model, r: Rect, buf: &mut Buffer) {
    let theme = &model.theme;
    let tile = theme
        .paint(Ink::Role(Role::Bg))
        .mix(Ink::Role(Role::Dim), 0.18)
        .color();
    for pos in r.positions() {
        buf[pos].set_char(' ').set_bg(tile);
    }
    let (cx, cy) = (r.x + r.width / 2, r.y + r.height / 2);
    if r.contains((cx, cy).into()) {
        buf[(cx, cy)]
            .set_char('♪')
            .set_fg(theme.role(Role::Dim))
            .set_bg(tile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KITTY: Caps = Caps {
        kitty: true,
        sextants: true,
    };
    const PLAIN: Caps = Caps {
        kitty: false,
        sextants: false,
    };

    #[test]
    fn auto_picks_the_best_the_terminal_has() {
        use ColorDepth::*;
        let r = |d, c, depth| resolve(d, c, depth);
        assert_eq!(r(Detail::Auto, KITTY, TrueColor), Drawn::Pixels);
        // Pixels don't need the palette's colours: 16 is fine.
        assert_eq!(r(Detail::Auto, KITTY, Ansi16), Drawn::Pixels);
        let wezterm = Caps {
            kitty: false,
            sextants: true,
        };
        assert_eq!(
            r(Detail::Auto, wezterm, TrueColor),
            Drawn::Text(TextMode::Sextant)
        );
        assert_eq!(
            r(Detail::Auto, PLAIN, Ansi256),
            Drawn::Text(TextMode::Quadrant)
        );
        assert_eq!(r(Detail::Auto, PLAIN, Ansi16), Drawn::None);
        // NO_COLOR: no pictures, whatever was asked.
        for d in Detail::ALL {
            assert_eq!(r(d, KITTY, None), Drawn::None, "{d:?}");
        }
    }

    #[test]
    fn a_chosen_detail_is_kept_where_it_can_be() {
        use ColorDepth::*;
        // Pixels when asked, even where the terminal wasn't recognised.
        assert_eq!(resolve(Detail::Pixels, PLAIN, TrueColor), Drawn::Pixels);
        assert_eq!(
            resolve(Detail::HalfBlock, KITTY, TrueColor),
            Drawn::Text(TextMode::HalfBlock)
        );
        assert_eq!(
            resolve(Detail::Sextant, PLAIN, Ansi256),
            Drawn::Text(TextMode::Sextant)
        );
        assert_eq!(resolve(Detail::Quadrant, KITTY, Ansi16), Drawn::None);
    }

    #[test]
    fn detail_cycles_through_every_value() {
        let mut d = Detail::Auto;
        for _ in 0..Detail::ALL.len() {
            d = d.next();
        }
        assert_eq!(d, Detail::Auto);
    }

    #[test]
    fn sizes_cap_the_forms_and_shrink_to_ten_columns() {
        for (size, widest) in [
            (CoverSize::Small, 16),
            (CoverSize::Medium, 24),
            (CoverSize::Large, 34),
            (CoverSize::Fill, 64),
        ] {
            let forms = cover_forms(&Show::Picture, Place::Side, size, 2.0);
            assert_eq!(forms[0].size.width, widest, "{size:?}");
            assert_eq!(forms.last().unwrap().size.width, 10);
            assert!(forms.windows(2).all(|w| w[0].size.width > w[1].size.width));
            for f in &forms {
                assert_eq!(f.size.height, f.size.width / 2, "square at 2:1 cells");
                assert!(!f.fill && !f.seconds);
            }
        }
        // Taller cells: fewer rows for the same square.
        let tall = cover_forms(&Show::Picture, Place::Overlay, CoverSize::Medium, 2.4);
        assert_eq!(tall[0].size.height, 10);
        let msg = cover_forms(
            &Show::Message("no cover"),
            Place::Side,
            CoverSize::Fill,
            2.0,
        );
        assert_eq!(msg.len(), 1);
        assert_eq!((msg[0].size.width, msg[0].size.height), (10, 1));
        // Longer ones wrap to fit the panel's narrowest.
        let long = Show::Message("covers need 256 colours");
        let side = cover_forms(&long, Place::Side, CoverSize::Fill, 2.0);
        assert!(
            side[0].size.width <= 20 && side[0].size.height == 2,
            "{side:?}"
        );
        let lava = cover_forms(&long, Place::Overlay, CoverSize::Fill, 2.0);
        assert_eq!(lava[0].size.height, 1);
    }

    #[test]
    fn settings_read_and_write_lowercase_names() {
        let s: ArtSettings =
            toml::from_str("detail = \"halfblock\"\nsize = \"fill\"\ninline = false").unwrap();
        assert_eq!(
            (s.detail, s.size, s.inline),
            (Detail::HalfBlock, CoverSize::Fill, false)
        );
        let text = toml::to_string(&ArtSettings::default()).unwrap();
        assert!(text.contains("detail = \"auto\""), "{text}");
        assert!(text.contains("size = \"medium\""), "{text}");
        assert!(toml::from_str::<ArtSettings>("detail = \"sixel\"").is_err());
    }
}
