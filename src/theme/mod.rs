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
//! * **256**: lerped in RGB, then snapped to the nearest xterm index.
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

use ratatui::style::{Color, Modifier, Style};

/// The nine colour roles every palette defines (docs/design.md §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// App background outside the glass.
    Bg,
    /// Glass interior / bleed background.
    Liquid,
    WaxCool,
    WaxMid,
    WaxHot,
    /// Cap, base, overlay borders.
    #[cfg_attr(not(test), expect(dead_code, reason = "glass frame: lava-xxx"))]
    Metal,
    #[cfg_attr(not(test), expect(dead_code, reason = "chrome: lava-xxx"))]
    Text,
    Dim,
    /// The one accent.
    Accent,
}

impl Role {
    #[cfg_attr(not(test), expect(dead_code, reason = "palette picker: lava-xxx"))]
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
    pub fn lerp(self, other: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgb(
            mix(self.0, other.0),
            mix(self.1, other.1),
            mix(self.2, other.2),
        )
    }

    /// Lighting: multiply brightness by `k`. Brightening (`k > 1`) is
    /// eased by the colour's own lightness, so dark liquid glows nearly the
    /// full amount while light colours barely move: highlights keep their
    /// hue and the light palette doesn't blow out into white halos.
    pub fn scale(self, k: f32) -> Rgb {
        let k = if k > 1.0 {
            let luma = (0.2126 * f32::from(self.0)
                + 0.7152 * f32::from(self.1)
                + 0.0722 * f32::from(self.2))
                / 255.0;
            1.0 + (k - 1.0) * (1.0 - luma)
        } else {
            k
        };
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

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "config / --palette land with lava-xxx")
    )]
    pub fn by_name(name: &str) -> Option<&'static Palette> {
        Self::all().iter().find(|p| p.name == name)
    }

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
    /// Thermal ramp, 0 = `liquid` → `wax_cool` → `wax_mid` → 1 = `wax_hot`.
    Heat(f32),
}

/// Ramp resolution. 64 steps is smooth in truecolor and is the LUT size
/// §5.3 asks for in 256 colours; it also keeps slow temperature drifts
/// from changing a cell's colour every frame.
const RAMP_STEPS: usize = 64;
/// Truecolor channels are rounded to multiples of this, so sub-visible
/// colour drift doesn't repaint cells (ratatui only sends changed cells).
const TRUECOLOR_QUANT: u8 = 4;
/// Where `wax_cool` sits on the thermal ramp (`liquid` is at 0).
const HEAT_COOL_AT: f32 = 0.4;

/// A palette at a colour depth. Cheap to build; rebuild it when either
/// changes.
#[derive(Debug, Clone)]
pub struct Theme {
    palette: &'static Palette,
    depth: ColorDepth,
    /// True when colours can be blended (truecolor / 256 with an RGB palette).
    blend: bool,
    wax: [Rgb; RAMP_STEPS],
    heat: [Rgb; RAMP_STEPS],
}

impl Theme {
    pub fn new(palette: &'static Palette, depth: ColorDepth) -> Self {
        let blend =
            palette.has_rgb() && matches!(depth, ColorDepth::TrueColor | ColorDepth::Ansi256);
        let rgb = |r| palette.swatch(r).rgb.unwrap_or_default();
        let (liquid, cool, mid, hot) = (
            rgb(Role::Liquid),
            rgb(Role::WaxCool),
            rgb(Role::WaxMid),
            rgb(Role::WaxHot),
        );
        let wax = ramp(&[(0.0, cool), (0.5, mid), (1.0, hot)]);
        let heat = ramp(&[(0.0, liquid), (HEAT_COOL_AT, cool), (0.7, mid), (1.0, hot)]);
        Theme {
            palette,
            depth,
            blend,
            wax,
            heat,
        }
    }

    pub fn palette(&self) -> &'static Palette {
        self.palette
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "toast fades: lava-xxx"))]
    pub fn depth(&self) -> ColorDepth {
        self.depth
    }

    /// Whether colours blend smoothly. When false (16 colours, none, the
    /// `ansi` palette) styles should show gradients with glyph density.
    pub fn blends(&self) -> bool {
        self.blend
    }

    /// Whether there is any colour at all.
    pub fn has_color(&self) -> bool {
        self.depth != ColorDepth::None
    }

    /// Start a colour from `ink`; refine it with [`Paint::mix`] /
    /// [`Paint::scale`], then [`Paint::color`].
    pub fn paint(&self, ink: Ink) -> Paint<'_> {
        Paint {
            theme: self,
            rgb: self.rgb(ink),
            fallback: self.fallback(ink),
            index: match ink {
                Ink::Role(role) => self.palette.swatch(role).x256,
                _ => None,
            },
        }
    }

    /// Shorthand for `paint(ink).color()`.
    pub fn color(&self, ink: Ink) -> Color {
        self.paint(ink).color()
    }

    /// Shorthand for a role's colour.
    pub fn role(&self, role: Role) -> Color {
        self.color(Ink::Role(role))
    }

    /// A text style in `role`. In NO_COLOR, `accent` becomes bold (§5.3).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "chrome styling lands with lava-xxx")
    )]
    pub fn text(&self, role: Role) -> Style {
        let style = Style::new().fg(self.role(role));
        if self.depth == ColorDepth::None && role == Role::Accent {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
    }

    fn rgb(&self, ink: Ink) -> Rgb {
        let lut = |lut: &[Rgb; RAMP_STEPS], t: f32| {
            lut[(t.clamp(0.0, 1.0) * (RAMP_STEPS - 1) as f32).round() as usize]
        };
        match ink {
            Ink::Role(role) => self.palette.swatch(role).rgb.unwrap_or_default(),
            Ink::Wax(t) => lut(&self.wax, t),
            Ink::Heat(t) => lut(&self.heat, t),
        }
    }

    /// The colour used when blending is off.
    fn fallback(&self, ink: Ink) -> Color {
        if self.depth == ColorDepth::None {
            return Color::Reset;
        }
        let role = match ink {
            Ink::Role(role) => role,
            Ink::Wax(t) => wax_step(t),
            Ink::Heat(t) if t < HEAT_COOL_AT * 0.75 => Role::Liquid,
            Ink::Heat(t) => wax_step((t - HEAT_COOL_AT) / (1.0 - HEAT_COOL_AT)),
        };
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

/// The three discrete wax steps, for depths that can't blend.
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

    /// Brighten (`k > 1`) or darken (`k < 1`). Only visible when blending;
    /// discrete depths show light through glyphs instead.
    pub fn scale(self, k: f32) -> Self {
        Paint {
            rgb: self.rgb.scale(k),
            index: if k == 1.0 { self.index } else { None },
            ..self
        }
    }

    pub fn color(self) -> Color {
        if !self.theme.blend {
            return self.fallback;
        }
        let Rgb(r, g, b) = self.rgb;
        match self.theme.depth {
            ColorDepth::Ansi256 => {
                Color::Indexed(self.index.unwrap_or_else(|| xterm::nearest(self.rgb)))
            }
            // Unmixed roles stay exact; blends are quantised.
            _ if self.index.is_some() => Color::Rgb(r, g, b),
            _ => {
                let q = |c: u8| {
                    c.saturating_add(TRUECOLOR_QUANT / 2) / TRUECOLOR_QUANT * TRUECOLOR_QUANT
                };
                Color::Rgb(q(r), q(g), q(b))
            }
        }
    }
}
