//! Palettes and colour depth: the one place a colour is decided.
//!
//! A [`Palette`] defines the nine [`Role`]s of docs/design.md §5.1, each
//! with a hex colour plus 256- and 16-colour fallbacks. A [`Theme`] is a
//! palette at a detected [`ColorDepth`]; everything that draws asks it for
//! colours, either a role or a point on a continuous ramp ([`Ink`]), and
//! gets back a ratatui [`Color`] that is right for the terminal:
//!
//! * **truecolor**: exact RGB, lerped freely (lightly quantised so slow
//!   drifts don't repaint every cell every frame).
//! * **256**: lerped in RGB, then snapped to the nearest xterm index, or,
//!   for the lamp, ordered-dithered between the two that best mix to it
//!   ([`Theme::dithering`]).
//! * **16**: no blending. Ramps step through the three wax colours, mixes
//!   pick whichever side dominates.
//! * **none**: `Color::Reset` everywhere; styles carry the picture with
//!   glyph shape and density alone.
//!
//! Styles never name a colour themselves.

mod palettes;
#[cfg(test)]
mod tests;
mod xterm;

#[cfg(test)]
pub use xterm::delta_e;

use std::sync::LazyLock;

use ratatui::style::{Color, Modifier, Style};

/// The terminal's own default colour, as a foreground or a background:
/// what NO_COLOR draws everything in, and what `transparent` leaves.
pub const TERMINAL_DEFAULT: Color = Color::Reset;

/// Colours closer than this (OKLab ΔE) count as the same for drawing a
/// half-block cell in one colour ([`Theme::near`], [`Theme::merge`]).
pub const NEAR: f32 = 0.03;

/// The nine colour roles every palette defines (docs/design.md §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// App background: behind the chrome, around the panel.
    Bg,
    /// The lamp's liquid, behind the wax.
    Liquid,
    WaxCool,
    WaxMid,
    WaxHot,
    /// Overlay borders.
    Metal,
    Text,
    Dim,
    /// The one accent.
    Accent,
}

impl Role {
    #[cfg(test)]
    pub const ALL: [Role; 9] = [
        Role::Bg,
        Role::Liquid,
        Role::WaxCool,
        Role::WaxMid,
        Role::WaxHot,
        Role::Metal,
        Role::Text,
        Role::Dim,
        Role::Accent,
    ];
}

/// An 8-bit-per-channel colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(hex: u32) -> Self {
        Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
    }

    /// `self` → `other` by `t` (0..=1).
    #[inline]
    pub fn lerp(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgb(
            mix(self.0, other.0),
            mix(self.1, other.1),
            mix(self.2, other.2),
        )
    }

    /// Multiply brightness by `k`; saturates at white.
    #[inline]
    pub fn scale(self, k: f32) -> Rgb {
        let s = |c: u8| (f32::from(c) * k).round().clamp(0.0, 255.0) as u8;
        Rgb(s(self.0), s(self.1), s(self.2))
    }
}

/// One role's colour at every depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Swatch {
    /// `None` for palettes that only use the terminal's own colours.
    pub rgb: Option<Rgb>,
    /// xterm-256 index (hand-picked per §5.2), `None` like `rgb`.
    pub x256: Option<u8>,
    /// 16-colour fallback; `Color::Reset` = terminal default.
    pub ansi: Color,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Palette {
    pub name: &'static str,
    /// Indexed by `Role as usize`.
    swatches: [Swatch; 9],
}

impl Palette {
    /// Every palette, in cycle order; the first is the default (`lava`).
    pub fn all() -> &'static [Palette] {
        &palettes::PALETTES
    }

    pub fn by_name(name: &str) -> Option<&'static Palette> {
        Self::all().iter().find(|p| p.name == name)
    }

    #[inline]
    pub fn swatch(&self, role: Role) -> Swatch {
        self.swatches[role as usize]
    }

    /// Whether this palette has its own RGB colours (everything but `ansi`).
    fn has_rgb(&self) -> bool {
        self.swatches.iter().all(|s| s.rgb.is_some())
    }
}

/// How many colours the terminal can show (docs/design.md §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    TrueColor,
    Ansi256,
    Ansi16,
    /// `NO_COLOR`: no colour at all.
    None,
}

impl ColorDepth {
    /// Detect from the environment: `NO_COLOR` → none, `COLORTERM`
    /// truecolor/24bit → truecolor, `TERM` *256color* → 256, else 16.
    pub fn detect() -> Self {
        let var = |k| std::env::var(k).ok();
        Self::from_env(
            var("NO_COLOR").as_deref(),
            var("COLORTERM").as_deref(),
            var("TERM").as_deref(),
        )
    }

    /// [`detect`](Self::detect) on explicit values (testable).
    pub fn from_env(no_color: Option<&str>, colorterm: Option<&str>, term: Option<&str>) -> Self {
        if no_color.is_some_and(|v| !v.is_empty()) {
            ColorDepth::None
        } else if colorterm.is_some_and(|v| {
            let v = v.to_ascii_lowercase();
            v == "truecolor" || v == "24bit"
        }) {
            ColorDepth::TrueColor
        } else if term.is_some_and(|t| t.contains("256color")) {
            ColorDepth::Ansi256
        } else {
            ColorDepth::Ansi16
        }
    }
}

/// Something to colour with: a fixed role or a point on a ramp.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ink {
    Role(Role),
    /// Wax gradient, 0 = `wax_cool` … ½ = `wax_mid` … 1 = `wax_hot`.
    Wax(f32),
}

/// Ramp resolution. 64 steps is smooth in truecolor and is the LUT size
/// §5.3 asks for in 256 colours; it also keeps slow temperature drifts
/// from changing a cell's colour every frame.
const RAMP_STEPS: usize = 64;
/// Truecolor channels are rounded to multiples of this, so sub-visible
/// colour drift doesn't repaint cells (ratatui only sends changed cells).
const TRUECOLOR_QUANT: u8 = 4;

/// A palette at a colour depth. Cheap to build; rebuild it when either
/// changes.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    palette: &'static Palette,
    depth: ColorDepth,
    /// True when colours can be blended (truecolor / 256 with an RGB palette).
    blend: bool,
    wax: [Rgb; RAMP_STEPS],
    /// Per-role overrides of the palette ([`with_role`](Self::with_role)),
    /// indexed by `Role as usize`.
    repaint: [Option<Repaint>; 9],
    /// 256 colours only: blends come out as `Color::Rgb`, left for
    /// [`dither`](Self::dither) to resolve per cell.
    dither: bool,
}

/// A role's colour taken from a [`Paint`] instead of the palette.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Repaint {
    rgb: Rgb,
    fallback: Color,
    index: Option<u8>,
}

impl Theme {
    pub fn new(palette: &'static Palette, depth: ColorDepth) -> Self {
        let blend =
            palette.has_rgb() && matches!(depth, ColorDepth::TrueColor | ColorDepth::Ansi256);
        let mut theme = Theme {
            palette,
            depth,
            blend,
            wax: [Rgb::default(); RAMP_STEPS],
            repaint: [None; 9],
            dither: false,
        };
        theme.build_ramps();
        theme
    }

    /// This theme, for drawing into a region that is then passed through
    /// [`dither`](Self::dither) cell by cell: blended colours (mixes,
    /// scales, ramps) come back as exact `Color::Rgb`, unmixed roles as
    /// their hand-picked index. `None` unless the theme blends in 256
    /// colours, the one depth that dithers.
    pub fn dithering(&self) -> Option<Theme> {
        (self.blend && self.depth == ColorDepth::Ansi256).then(|| Theme {
            dither: true,
            ..self.clone()
        })
    }

    /// Resolve a colour a [`dithering`](Self::dithering) theme drew, at a
    /// point where an ordered dither's threshold is `threshold` (0..1): RGB
    /// becomes one of the two xterm indices that best mix to it. A colour
    /// whose nearest index keeps its hue, or whose best pair looks the
    /// same, gets that one index everywhere; anything not RGB passes
    /// through.
    #[inline]
    pub fn dither(&self, c: Color, threshold: f32) -> Color {
        match c {
            Color::Rgb(r, g, b) if self.dither => {
                Color::Indexed(xterm::dither(Rgb(r, g, b), threshold))
            }
            c => c,
        }
    }

    /// This theme with `role` repainted as `paint` (any mix or scale of
    /// inks), e.g. the liquid pulsing toward `accent` for the phase-change
    /// flash (§4.5). Everything drawn in the role follows, ramps included.
    pub fn with_role(&self, role: Role, paint: Paint<'_>) -> Theme {
        let mut theme = self.clone();
        theme.repaint[role as usize] = Some(Repaint {
            rgb: paint.rgb,
            fallback: paint.fallback,
            index: paint.index,
        });
        theme.build_ramps();
        theme
    }

    fn build_ramps(&mut self) {
        let rgb = |r| self.rgb(Ink::Role(r));
        let (cool, mid, hot) = (rgb(Role::WaxCool), rgb(Role::WaxMid), rgb(Role::WaxHot));
        self.wax = ramp(&[(0.0, cool), (0.5, mid), (1.0, hot)]);
    }

    pub fn palette(&self) -> &'static Palette {
        self.palette
    }

    pub fn depth(&self) -> ColorDepth {
        self.depth
    }

    /// Whether colours blend smoothly. When false (16 colours, none, the
    /// `ansi` palette) styles should show gradients with glyph density.
    #[inline]
    pub fn blends(&self) -> bool {
        self.blend
    }

    /// Whether there is any colour at all.
    #[inline]
    pub fn has_color(&self) -> bool {
        self.depth != ColorDepth::None
    }

    /// Start a colour from `ink`; refine it with [`Paint::mix`] /
    /// [`Paint::scale`], then [`Paint::color`].
    #[inline]
    pub fn paint(&self, ink: Ink) -> Paint<'_> {
        Paint {
            theme: self,
            rgb: self.rgb(ink),
            fallback: self.fallback(ink),
            index: match ink {
                Ink::Role(role) => match self.repaint[role as usize] {
                    Some(r) => r.index,
                    None => self.palette.swatch(role).x256,
                },
                _ => None,
            },
        }
    }

    /// Shorthand for `paint(ink).color()`.
    #[inline]
    pub fn color(&self, ink: Ink) -> Color {
        self.paint(ink).color()
    }

    /// Shorthand for a role's colour.
    #[inline]
    pub fn role(&self, role: Role) -> Color {
        self.color(Ink::Role(role))
    }

    /// The app background: `bg`, or the terminal's own when `transparent`
    /// (§9), for the chrome and around it.
    pub fn background(&self, transparent: bool) -> Color {
        if transparent {
            TERMINAL_DEFAULT
        } else {
            self.role(Role::Bg)
        }
    }

    /// `c` faded `t` of the way toward the palette's `bg`, for dimming what
    /// a sheet covers. Only in truecolor, and only for RGB colours: other
    /// colours come back as they are.
    pub fn fade_to_bg(&self, c: Color, t: f32) -> Color {
        let bg = self.palette.swatch(Role::Bg).rgb;
        match (c, bg) {
            (Color::Rgb(r, g, b), Some(bg)) if self.depth == ColorDepth::TrueColor => {
                let Rgb(r, g, b) = Rgb(r, g, b).lerp(bg, t);
                Color::Rgb(r, g, b)
            }
            _ => c,
        }
    }

    /// `c`, an already drawn colour, veiled `t` of the way toward `role`:
    /// the soft backing behind widgets on the lava. Truecolor RGB only
    /// (256 colours would snap the tints to greys); otherwise whichever
    /// side dominates.
    pub fn veil(&self, c: Color, role: Role, t: f32) -> Color {
        let to = self.paint(Ink::Role(role));
        match c {
            Color::Rgb(r, g, b) if self.blend && self.depth == ColorDepth::TrueColor => {
                let Rgb(r, g, b) = Rgb(r, g, b).lerp(to.rgb, t);
                Color::Rgb(r, g, b)
            }
            _ if t >= 0.5 => to.color(),
            _ => c,
        }
    }

    /// `c`, an already drawn colour, moved `t` of the way toward `to`, as
    /// this depth draws it (truecolor: quantised RGB; 256 colours: the
    /// nearest xterm index): what floating text knocks a cell's background
    /// back to behind a letter that wouldn't read (§4.6). `None` where
    /// colours don't blend or either colour can't be told.
    pub fn toward(&self, c: Color, to: Color, t: f32) -> Option<Color> {
        if !self.blend {
            return None;
        }
        let rgb = seen(c)?.lerp(seen(to)?, t);
        Some(match self.depth {
            ColorDepth::Ansi256 => Color::Indexed(xterm::nearest(rgb)),
            _ => quantised(rgb),
        })
    }

    /// The two inks for glyphs drawn straight onto the lava (widgets with
    /// no backing, §4.6), `(light, dark)`: the palette's `text` and `bg`,
    /// lighter first (paper's text is the dark one), white and black
    /// where they're the terminal's defaults.
    pub fn floating_inks(&self) -> (Color, Color) {
        let pick = |role, named| match self.role(role) {
            TERMINAL_DEFAULT => named,
            c => c,
        };
        let (text, bg) = (pick(Role::Text, Color::White), pick(Role::Bg, Color::Black));
        let lum = |c| seen(c).map_or(0.0, luminance);
        if lum(text) >= lum(bg) {
            (text, bg)
        } else {
            (bg, text)
        }
    }

    /// A drawn colour's WCAG relative luminance (0..=1), for
    /// [`contrast`], or `None` when it can't be told (the terminal's
    /// defaults).
    pub fn luminance(&self, c: Color) -> Option<f32> {
        seen(c).map(luminance)
    }

    /// As [`Theme::luminance`], for a cell *background* on a terminal
    /// that shows backgrounds at `opacity` (glyphs stay opaque): the
    /// colour over a dark desktop, measured from Ghostty captures at
    /// 0.75 (the lava liquid `#23160C` shows as `#19130D`).
    pub fn shown_luminance(&self, c: Color, opacity: Option<f32>) -> Option<f32> {
        let rgb = seen(c)?;
        Some(luminance(opacity.map_or(rgb, |a| rgb.scale(a))))
    }

    /// What a cell split into `a` and `b` halves looks like from afar:
    /// their mean in truecolor, else `b`.
    pub fn mean(&self, a: Color, b: Color) -> Color {
        match (a, b) {
            (Color::Rgb(r, g, bl), Color::Rgb(r2, g2, b2))
                if self.depth == ColorDepth::TrueColor =>
            {
                let Rgb(r, g, b) = Rgb(r, g, bl).lerp(Rgb(r2, g2, b2), 0.5);
                Color::Rgb(r, g, b)
            }
            _ => b,
        }
    }

    /// Whether `a` and `b` look the same side by side: within [`NEAR`]
    /// (OKLab ΔE) of each other. Colours that can't be told (the
    /// terminal's defaults) are near only themselves.
    #[cfg(test)]
    pub fn near(&self, a: Color, b: Color) -> bool {
        a == b
            || match (seen(a), seen(b)) {
                (Some(x), Some(y)) => xterm::delta_e(x, y) <= NEAR,
                _ => false,
            }
    }

    /// The one colour a half-block cell may show for pixels `a` and `b`
    /// when they're within `within` (OKLab ΔE; [`NEAR`]: they look the
    /// same): `a` if they're equal, else their mean (so neither pixel moves
    /// more than half of `within`). `None` when they're further apart, or
    /// either isn't RGB (16 colours, none: there's no mean to take).
    #[inline]
    pub fn merge(&self, a: Color, b: Color, within: f32) -> Option<Color> {
        match (a, b) {
            _ if a == b => Some(a),
            (Color::Rgb(r, g, bl), Color::Rgb(r2, g2, b2)) => {
                let (x, y) = (Rgb(r, g, bl), Rgb(r2, g2, b2));
                (within == f32::INFINITY || xterm::delta_e_squared(x, y) <= within * within).then(
                    || {
                        let Rgb(r, g, b) = x.lerp(y, 0.5);
                        Color::Rgb(r, g, b)
                    },
                )
            }
            _ => None,
        }
    }

    /// Whether `a` is darker than `b` (unknown colours aren't).
    #[inline]
    pub fn darker(&self, a: Color, b: Color) -> bool {
        match (seen(a), seen(b)) {
            (Some(x), Some(y)) => luminance(x) < luminance(y),
            _ => false,
        }
    }

    /// A picture's pixel (album art): exact in truecolor, the nearest xterm
    /// index in 256 colours, and `None` below that, where a picture can't
    /// be shown at all (the widget then drops it). Palette-independent.
    pub fn image(&self, c: Rgb) -> Option<Color> {
        match self.depth {
            ColorDepth::TrueColor => Some(Color::Rgb(c.0, c.1, c.2)),
            ColorDepth::Ansi256 => Some(Color::Indexed(xterm::nearest(c))),
            ColorDepth::Ansi16 | ColorDepth::None => None,
        }
    }

    /// Whether [`image`](Self::image) shows pictures at this depth.
    pub fn shows_images(&self) -> bool {
        self.image(Rgb::default()).is_some()
    }

    /// A text style in `role`. In NO_COLOR, `accent` becomes bold (§5.3).
    pub fn text(&self, role: Role) -> Style {
        let style = Style::new().fg(self.role(role));
        if self.depth == ColorDepth::None && role == Role::Accent {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
    }

    #[inline]
    fn rgb(&self, ink: Ink) -> Rgb {
        let lut = |lut: &[Rgb; RAMP_STEPS], t: f32| {
            lut[(t.clamp(0.0, 1.0) * (RAMP_STEPS - 1) as f32).round() as usize]
        };
        match ink {
            Ink::Role(role) => match self.repaint[role as usize] {
                Some(r) => r.rgb,
                None => self.palette.swatch(role).rgb.unwrap_or_default(),
            },
            Ink::Wax(t) => lut(&self.wax, t),
        }
    }

    /// The colour used when blending is off.
    ///
    /// Deliberately not `#[inline]`, unlike the rest of the paint path:
    /// every paint computes it, and inlining it into blended styles' pixel
    /// loops cost the blended styles up to ~12 % (bench_lamp).
    fn fallback(&self, ink: Ink) -> Color {
        if self.depth == ColorDepth::None {
            return TERMINAL_DEFAULT;
        }
        let role = match ink {
            Ink::Role(role) => role,
            Ink::Wax(t) => wax_step(t),
        };
        if let Some(r) = self.repaint[role as usize] {
            return r.fallback;
        }
        let swatch = self.palette.swatch(role);
        match self.depth {
            ColorDepth::TrueColor => swatch
                .rgb
                .map_or(swatch.ansi, |c| Color::Rgb(c.0, c.1, c.2)),
            ColorDepth::Ansi256 => swatch.x256.map_or(swatch.ansi, Color::Indexed),
            _ => swatch.ansi,
        }
    }
}

/// A drawn colour as RGB, as near as can be told: xterm indices from the
/// standard table, named colours as xterm's defaults; `None` for the
/// terminal's own default.
fn seen(c: Color) -> Option<Rgb> {
    const SYSTEM: [u32; 16] = [
        0x000000, 0xCD0000, 0x00CD00, 0xCDCD00, 0x0000EE, 0xCD00CD, 0x00CDCD, 0xE5E5E5, 0x7F7F7F,
        0xFF0000, 0x00FF00, 0xFFFF00, 0x5C5CFF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
    ];
    let index = match c {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => return Some(Rgb(r, g, b)),
        Color::Indexed(i) => i,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
    };
    Some(match index {
        0..16 => Rgb::hex(SYSTEM[usize::from(index)]),
        16..232 => {
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            let i = index - 16;
            Rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let v = 8 + 10 * (index - 232);
            Rgb(v, v, v)
        }
    })
}

/// A blended truecolor colour, its channels rounded to
/// [`TRUECOLOR_QUANT`].
#[inline]
fn quantised(Rgb(r, g, b): Rgb) -> Color {
    let q = |c: u8| c.saturating_add(TRUECOLOR_QUANT / 2) / TRUECOLOR_QUANT * TRUECOLOR_QUANT;
    Color::Rgb(q(r), q(g), q(b))
}

/// The WCAG contrast ratio (1..=21) of two relative luminances.
pub fn contrast(a: f32, b: f32) -> f32 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// WCAG relative luminance (0..=1).
fn luminance(c: Rgb) -> f32 {
    /// sRGB channel → linear light, per 8-bit value.
    static LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| {
        std::array::from_fn(|v| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
    });
    let lin = |v: u8| LINEAR[usize::from(v)];
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

/// The three discrete wax steps, for depths that can't blend.
#[inline]
fn wax_step(t: f32) -> Role {
    if t < 1.0 / 3.0 {
        Role::WaxCool
    } else if t < 2.0 / 3.0 {
        Role::WaxMid
    } else {
        Role::WaxHot
    }
}

/// Sample a piecewise-linear gradient into a LUT.
fn ramp(stops: &[(f32, Rgb)]) -> [Rgb; RAMP_STEPS] {
    std::array::from_fn(|i| {
        let t = i as f32 / (RAMP_STEPS - 1) as f32;
        let k = stops
            .windows(2)
            .position(|w| t <= w[1].0)
            .unwrap_or(stops.len() - 2);
        let ((t0, a), (t1, b)) = (stops[k], stops[k + 1]);
        a.lerp(b, (t - t0) / (t1 - t0))
    })
}

/// A colour being mixed. Blends in RGB when the theme can, otherwise picks
/// discretely. Cheap to copy; resolve with [`Paint::color`].
#[derive(Debug, Clone, Copy)]
pub struct Paint<'t> {
    theme: &'t Theme,
    rgb: Rgb,
    fallback: Color,
    /// The palette's hand-picked 256 index, while the paint is still an
    /// unmixed role (such paints also skip truecolor quantisation).
    index: Option<u8>,
}

impl Paint<'_> {
    /// Move `amount` (0..=1) of the way toward `other`. Without blending,
    /// the dominant side wins.
    #[inline]
    pub fn mix(self, other: Ink, amount: f32) -> Self {
        Paint {
            rgb: self.rgb.lerp(self.theme.rgb(other), amount),
            fallback: if amount >= 0.5 {
                self.theme.fallback(other)
            } else {
                self.fallback
            },
            index: if amount > 0.0 { None } else { self.index },
            ..self
        }
    }

    /// Brighten (`k > 1`) or darken (`k < 1`). Only visible when blending.
    #[inline]
    pub fn scale(self, k: f32) -> Self {
        Paint {
            rgb: self.rgb.scale(k),
            index: if k == 1.0 { self.index } else { None },
            ..self
        }
    }

    #[inline]
    pub fn color(self) -> Color {
        if !self.theme.blend {
            return self.fallback;
        }
        let Rgb(r, g, b) = self.rgb;
        match self.theme.depth {
            ColorDepth::Ansi256 => match self.index {
                Some(i) => Color::Indexed(i),
                None if self.theme.dither => Color::Rgb(r, g, b),
                None => Color::Indexed(xterm::nearest(self.rgb)),
            },
            // Unmixed roles stay exact; blends are quantised.
            _ if self.index.is_some() => Color::Rgb(r, g, b),
            _ => quantised(self.rgb),
        }
    }
}
