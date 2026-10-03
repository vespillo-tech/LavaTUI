//! Cover: the playing track's album art on its own, placeable like any
//! widget (`o`: side → lava → off), at the detail and size of `[art]`.
//! Off by default.
//!
//! **Detail** (`art.detail`, `O` cycles it; on screen the cover quality):
//! `sharp` is the real picture through whichever pixel protocol the
//! terminal speaks: kitty graphics (kitty, Ghostty), iTerm2 inline images
//! (iTerm2, WezTerm) or sixel (foot, mlterm, Konsole); see
//! [`crate::graphics`]. Without one it's the finest text cells: sextants
//! (2 × 3 pixels a cell, Unicode 13) where they're known to be drawn, else
//! quadrants (2 × 2). `small-pixels`, `medium-pixels` and `big-pixels` are
//! pixel art, the cover in flat square blocks, about 32, 16 and 10
//! across: where the terminal shows pictures, a picture of exactly that
//! many squares; elsewhere drawn in whole and half cells, fewer on a small
//! cover ([`super::picture`]). While a picture is on its way the cover is
//! never drawn in text cells instead (they'd look different): the
//! picture already up, if it's the same size, else a blank tile in the
//! cover's colour. `auto` is `sharp`. Text cells need 256 colours or
//! more; pixels any colour at all. With no way to show it the widget is
//! one calm line.
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
use crate::graphics::{self, Protocol};
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

/// `art.detail`: how covers are drawn, as fine or coarse as the user
/// likes. Older names still load as the nearest look: `pixels` / `photo`
/// / `sextant` / `fine` are `sharp`; `quadrant` / `medium` / `pixelated`
/// `medium-pixels`; `halfblock` / `coarse` / `chunky` `big-pixels`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Detail {
    /// The best the terminal can do (sharp).
    #[default]
    Auto,
    /// The real picture where the terminal shows pictures, else the finest
    /// text cells.
    #[serde(alias = "pixels", alias = "photo", alias = "sextant", alias = "fine")]
    Sharp,
    /// Pixel art, about 32 blocks across.
    SmallPixels,
    /// Pixel art, about 16 blocks across.
    #[serde(alias = "quadrant", alias = "medium", alias = "pixelated")]
    MediumPixels,
    /// Pixel art, about 10 blocks across.
    #[serde(alias = "halfblock", alias = "coarse", alias = "chunky")]
    BigPixels,
}

impl Detail {
    pub const ALL: [Detail; 5] = [
        Detail::Auto,
        Detail::Sharp,
        Detail::SmallPixels,
        Detail::MediumPixels,
        Detail::BigPixels,
    ];

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&d| d == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// The name the user reads (toasts, settings).
    pub fn label(self) -> &'static str {
        match self {
            Detail::Auto => "auto",
            _ => self.grain().label(),
        }
    }

    /// How coarse the picture is.
    pub fn grain(self) -> Grain {
        match self {
            Detail::Auto | Detail::Sharp => Grain::Sharp,
            Detail::SmallPixels => Grain::Pixels(0),
            Detail::MediumPixels => Grain::Pixels(1),
            Detail::BigPixels => Grain::Pixels(2),
        }
    }
}

/// The pixel-art sizes, as the user reads them (by `picture::PIXEL_ART`
/// level).
const PIXEL_NAMES: [&str; 3] = ["small pixels", "medium pixels", "big pixels"];

/// How coarse a cover is drawn: as sharp as it can be, or pixel art at a
/// level of [`picture::PIXEL_ART`] (0 the finest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grain {
    Sharp,
    Pixels(usize),
}

impl Grain {
    pub fn label(self) -> &'static str {
        match self {
            Grain::Sharp => "sharp",
            Grain::Pixels(level) => PIXEL_NAMES[level.min(PIXEL_NAMES.len() - 1)],
        }
    }

    /// The same in text cells, in a terminal with `caps`.
    pub fn text(self, caps: Caps) -> TextMode {
        match self {
            Grain::Sharp if caps.sextants => TextMode::Sextant,
            Grain::Sharp => TextMode::Quadrant,
            Grain::Pixels(level) => TextMode::Pixels(level),
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
    /// A picture, sharp or pixel art.
    Pixels(Protocol, Grain),
    Text(TextMode),
    /// No picture at all (16 colours without pixels, or no colour).
    None,
}

impl Drawn {
    /// How fine the picture is, as the user reads it.
    pub fn label(self) -> &'static str {
        match self {
            Drawn::Pixels(_, grain) => grain.label(),
            Drawn::Text(TextMode::Sextant | TextMode::Quadrant) => "sharp",
            Drawn::Text(TextMode::Pixels(level)) => Grain::Pixels(level).label(),
            Drawn::None => "no pictures in this terminal",
        }
    }
}

/// What the terminal can show, read once at start ([`crate::graphics`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Caps {
    /// The terminal's pixel protocol, if it has one.
    pub pixels: Option<Protocol>,
    /// Draws Unicode 13 block sextants (its own glyphs, not a font's).
    pub sextants: bool,
}

impl Caps {
    /// From the environment; never in tests (they say what they want).
    /// A pixel protocol the terminal still has to confirm
    /// ([`graphics::probe`]) comes back apart, `pixels` staying `None`
    /// until it does: everywhere but Windows (whose console input doesn't
    /// pass replies on), unless `LAVATUI_GRAPHICS` said it.
    pub fn detect() -> (Self, Option<Protocol>) {
        if cfg!(test) {
            return (Self::default(), None);
        }
        let var = |k: &str| std::env::var(k).ok();
        let ghostex = graphics::ghostex_env();
        let pixels = graphics::detect(var).filter(|_| !ghostex || graphics::forced(var));
        let caps = Self {
            pixels,
            sextants: !ghostex && (pixels == Some(Protocol::Kitty) || sextants(var)),
        };
        if pixels.is_some() && cfg!(unix) && !graphics::forced(var) {
            (
                Self {
                    pixels: None,
                    ..caps
                },
                pixels,
            )
        } else {
            (caps, None)
        }
    }
}

/// Terminals known to draw block sextants themselves: WezTerm, foot,
/// Windows Terminal (and kitty / Ghostty, which have pixels anyway). Never
/// through a multiplexer: its screen draws them, whatever `TERM_PROGRAM`
/// it inherited (Ghostex's shows `?`).
pub fn sextants(var: impl Fn(&str) -> Option<String>) -> bool {
    if graphics::multiplexed(&var) {
        return false;
    }
    let program = var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    let term = var("TERM").unwrap_or_default();
    program == "wezterm"
        || program == "ghostty"
        || term.starts_with("foot")
        || term == "xterm-kitty"
        || var("WT_SESSION").is_some()
}

/// `detail` at this terminal and depth (pure): a picture where the
/// terminal shows them, else the same grain in text cells. Where there are
/// none (not recognised, or not confirmed) it's always text: never a
/// protocol the terminal may not speak (kitty placeholders show as `?`).
/// `LAVATUI_GRAPHICS` is the way to name one.
pub fn resolve(detail: Detail, caps: Caps, depth: ColorDepth) -> Drawn {
    let grain = detail.grain();
    match caps.pixels {
        _ if depth == ColorDepth::None => Drawn::None,
        Some(protocol) => Drawn::Pixels(protocol, grain),
        None if matches!(depth, ColorDepth::TrueColor | ColorDepth::Ansi256) => {
            Drawn::Text(grain.text(caps))
        }
        None => Drawn::None,
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
    Message(String),
}

/// [`what`], its message with its `♪` when the glyphs have one.
fn show(model: &Model) -> Show {
    match what(model) {
        Show::Message(text) => Show::Message(format!("{}{text}", model.glyphs().note)),
        picture => picture,
    }
}

fn what(model: &Model) -> Show {
    if model.pictures() == Drawn::None {
        return Show::Message("covers need 256 colours".into());
    }
    let Some(snap) = model.music.snapshot.as_ref() else {
        return Show::Message("…".into());
    };
    let track = match (&snap.status, &snap.track) {
        (Status::Playing | Status::Paused, Some(track)) => track,
        (Status::Connecting, _) => return Show::Message("…".into()),
        // The player's own problem and next step, as the music widget says.
        (Status::Unavailable(reason), _) => {
            return Show::Message(reason.message_for(snap.player_name(), "album art"));
        }
        _ => return Show::Message("nothing playing".into()),
    };
    if track.artwork_url.is_empty() || model.music.art() == ArtState::Missing {
        return Show::Message("no cover".into());
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

/// `text`, wrapped for `place`.
fn message(text: &str, place: Place) -> Vec<String> {
    let w = match place {
        Place::Overlay => MESSAGE_W.1,
        _ => MESSAGE_W.0,
    };
    wrap(text, w)
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
            for (i, line) in message(&text, place).iter().enumerate() {
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
    fn chip<'m>(&self, _model: &'m Model) -> Option<ChipText<'m>> {
        None
    }
}

/// The playing track's cover in `r`, however this terminal draws it best
/// right now: its picture once that's all there (kitty placeholders, or
/// the spot an iTerm2 / sixel picture is placed over), else text cells,
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
        Drawn::Pixels(protocol, grain) => {
            let key = picture_key(url, r, grain);
            let Rgb(red, green, blue) = art.mean();
            let bg = Color::Rgb(red, green, blue);
            // The picture once it's all there. Until then (a frame or two
            // for kitty, a few ms of sixel encoding) the one already up if
            // it's this size, else a blank tile: never text cells, whose
            // blocks and colours look different from the picture's.
            match protocol {
                Protocol::Kitty => {
                    let up = || model.kitty.shown_sized(r.width, r.height);
                    if let Some(id) = model.kitty.ready(&key) {
                        return graphics::draw(buf, r, id, bg);
                    }
                    if model.kitty.pending(&key) {
                        return match up() {
                            Some(id) => graphics::draw(buf, r, id, bg),
                            None => blank(r, bg, buf),
                        };
                    }
                }
                Protocol::Iterm | Protocol::Sixel => {
                    if model.inline.shows(r) {
                        return graphics::inline::draw(buf, r, bg);
                    }
                    if model.inline.pending() {
                        return blank(r, bg, buf);
                    }
                }
            }
            // No picture can come (no room for one, or it couldn't be
            // made): the cover in text cells.
            grain.text(model.caps)
        }
        Drawn::Text(mode) => mode,
        Drawn::None => return placeholder(model, r, buf),
    };
    if model.theme.shows_images() {
        picture::draw(
            buf,
            r,
            &art,
            url,
            (text, model.translucent_cells()),
            &model.theme,
        );
    } else {
        placeholder(model, r, buf);
    }
}

/// The picture in pixels of the cover at `source`, drawn in `r` at
/// `grain`: pixel art is `PIXEL_ART` squares across and down (32, 16 or
/// 10), however big the cover; text cells have their own (`pixel_grid`).
pub fn picture_key(source: &str, r: Rect, grain: Grain) -> graphics::Key {
    let grid = match grain {
        Grain::Sharp => None,
        Grain::Pixels(level) => {
            let n = picture::PIXEL_ART[level.min(picture::PIXEL_ART.len() - 1)];
            Some((n, n))
        }
    };
    graphics::Key {
        source: source.to_owned(),
        cols: r.width,
        rows: r.height,
        grid,
    }
}

/// The cover's spot while its picture is on its way: its average colour.
fn blank(r: Rect, bg: Color, buf: &mut Buffer) {
    for p in r.positions() {
        buf[p].set_symbol(" ").set_fg(bg).set_bg(bg);
    }
}

/// A quiet tile where the cover will be: `bg` tinted toward `dim`, a dim
/// `♪` in the middle (when the glyphs have one).
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
    let note = model.glyphs().note.trim();
    if r.contains((cx, cy).into()) && !note.is_empty() {
        buf[(cx, cy)]
            .set_symbol(note)
            .set_fg(theme.role(Role::Dim))
            .set_bg(tile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KITTY: Caps = Caps {
        pixels: Some(Protocol::Kitty),
        sextants: true,
    };
    const PLAIN: Caps = Caps {
        pixels: None,
        sextants: false,
    };
    const SIXEL: Caps = Caps {
        pixels: Some(Protocol::Sixel),
        sextants: false,
    };
    const ITERM: Caps = Caps {
        pixels: Some(Protocol::Iterm),
        sextants: true,
    };

    #[test]
    fn auto_picks_the_best_the_terminal_has() {
        use ColorDepth::*;
        let r = |d, c, depth| resolve(d, c, depth);
        let pixels = |p| Drawn::Pixels(p, Grain::Sharp);
        assert_eq!(r(Detail::Auto, KITTY, TrueColor), pixels(Protocol::Kitty));
        assert_eq!(r(Detail::Auto, ITERM, TrueColor), pixels(Protocol::Iterm));
        assert_eq!(r(Detail::Auto, SIXEL, Ansi256), pixels(Protocol::Sixel));
        // Pixels don't need the palette's colours: 16 is fine.
        assert_eq!(r(Detail::Auto, KITTY, Ansi16), pixels(Protocol::Kitty));
        assert_eq!(r(Detail::Auto, SIXEL, Ansi16), pixels(Protocol::Sixel));
        let wezterm = Caps {
            pixels: Option::None,
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

    /// lava-bq0: every quality is its own look, in pictures and in text
    /// cells alike (a terminal with pictures used to show the same photo
    /// for most of them).
    #[test]
    fn every_quality_is_its_own_look_everywhere() {
        use ColorDepth::*;
        for caps in [KITTY, ITERM, SIXEL, PLAIN] {
            for depth in [TrueColor, Ansi256] {
                let drawn: Vec<Drawn> = Detail::ALL[1..]
                    .iter()
                    .map(|&d| resolve(d, caps, depth))
                    .collect();
                for (i, a) in drawn.iter().enumerate() {
                    assert!(
                        drawn[i + 1..].iter().all(|b| a != b),
                        "{caps:?} {depth:?}: {drawn:?}"
                    );
                }
                let labels: Vec<&str> = drawn.iter().map(|d| d.label()).collect();
                assert_eq!(
                    labels,
                    ["sharp", "small pixels", "medium pixels", "big pixels"]
                );
                assert_eq!(
                    resolve(Detail::Auto, caps, depth),
                    drawn[0],
                    "auto is sharp"
                );
            }
        }
        // Pictures: pixel art goes as pixels too.
        assert_eq!(
            resolve(Detail::BigPixels, KITTY, Ansi16),
            Drawn::Pixels(Protocol::Kitty, Grain::Pixels(2))
        );
        assert_eq!(
            resolve(Detail::SmallPixels, PLAIN, Ansi256),
            Drawn::Text(TextMode::Pixels(0))
        );
        assert_eq!(resolve(Detail::MediumPixels, PLAIN, Ansi16), Drawn::None);
        // Sharp only uses sextants where they're drawn.
        assert_eq!(
            resolve(Detail::Sharp, PLAIN, TrueColor),
            Drawn::Text(TextMode::Quadrant)
        );
        assert_eq!(
            Detail::ALL.map(Detail::grain),
            [
                Grain::Sharp,
                Grain::Sharp,
                Grain::Pixels(0),
                Grain::Pixels(1),
                Grain::Pixels(2)
            ]
        );
    }

    /// Settings saved by older versions (and this one's own names) load
    /// as the nearest look; each is written back by its own name.
    #[test]
    fn old_detail_names_still_load() {
        for (names, detail) in [
            (
                &["pixels", "photo", "sextant", "fine", "sharp"][..],
                Detail::Sharp,
            ),
            (&["small-pixels"], Detail::SmallPixels),
            (
                &["quadrant", "medium", "pixelated", "medium-pixels"],
                Detail::MediumPixels,
            ),
            (
                &["halfblock", "coarse", "chunky", "big-pixels"],
                Detail::BigPixels,
            ),
            (&["auto"], Detail::Auto),
        ] {
            for name in names {
                let s: ArtSettings = toml::from_str(&format!("detail = \"{name}\"")).unwrap();
                assert_eq!(s.detail, detail, "{name}");
            }
        }
        for detail in Detail::ALL {
            let text = toml::to_string(&ArtSettings {
                detail,
                ..ArtSettings::default()
            })
            .unwrap();
            let name = detail.label().replace(' ', "-");
            assert!(text.contains(&format!("detail = \"{name}\"")), "{text}");
        }
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
            &Show::Message("♪ no cover".into()),
            Place::Side,
            CoverSize::Fill,
            2.0,
        );
        assert_eq!(msg.len(), 1);
        assert_eq!((msg[0].size.width, msg[0].size.height), (10, 1));
        // Longer ones wrap to fit the panel's narrowest.
        let long = Show::Message("♪ covers need 256 colours".into());
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
            toml::from_str("detail = \"big-pixels\"\nsize = \"fill\"\ninline = false").unwrap();
        assert_eq!(
            (s.detail, s.size, s.inline),
            (Detail::BigPixels, CoverSize::Fill, false)
        );
        let text = toml::to_string(&ArtSettings::default()).unwrap();
        assert!(text.contains("detail = \"auto\""), "{text}");
        assert!(text.contains("size = \"medium\""), "{text}");
        assert!(toml::from_str::<ArtSettings>("detail = \"sixel\"").is_err());
    }
}
