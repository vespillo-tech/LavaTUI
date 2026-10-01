//! Render styles: turn the sampled wax field into terminal cells.
//!
//! The pipeline, once per frame, all inside [`LampView`]:
//!
//! 1. The active [`Style`] says how many square sample pixels it wants per
//!    cell ([`Grid`]: half-block 1×2, braille 2×4, …).
//! 2. The field is sampled at that grid into a reused buffer (or at a
//!    reduced grid and upsampled, above [`SAMPLE_BUDGET`]).
//! 3. An optional [`Lighting`] pass fills a per-sample brightness buffer.
//! 4. The style draws the [`Canvas`] (samples + mask + light + theme) into
//!    the buffer, cell by cell, inside its `Rect` only.
//!
//! Adding a style: one file in `styles/` implementing [`Style`], plus one
//! line in the `styles::ALL` registry.

mod cell;
mod styles;
#[cfg(test)]
mod tests;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::StatefulWidget;

use crate::light::Lighting;
use crate::sim::{Field, SURFACE, Sample};
use crate::theme::{Ink, Role, Theme};

/// Most samples a frame may take (docs/design.md §2.4). Above it the field
/// is sampled coarser and upsampled bilinearly.
pub const SAMPLE_BUDGET: usize = 400_000;

/// Sample pixels per terminal cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub x: u16,
    pub y: u16,
}

impl Grid {
    #[cfg_attr(not(test), expect(dead_code, reason = "glyph styles: lava-y7g"))]
    pub const CELL: Grid = Grid { x: 1, y: 1 };
    pub const HALF_BLOCK: Grid = Grid { x: 1, y: 2 };
    pub const BRAILLE: Grid = Grid { x: 2, y: 4 };
}

/// A way of drawing the lamp.
///
/// Implementations are stateless unit structs (any per-frame scratch lives
/// in [`LampState`]), registered once in `styles::ALL`.
pub trait Style: Sync {
    /// Lowercase name, shown in the UI and used in config.
    fn name(&self) -> &'static str;
    /// Sample pixels per cell. The canvas passed to [`draw`](Self::draw)
    /// is exactly `area.width × grid.x` by `area.height × grid.y`.
    fn grid(&self) -> Grid;
    /// Draw `canvas` into `area` of `buf`. Must write every cell of `area`
    /// and nothing outside it.
    fn draw(&self, canvas: &Canvas, area: Rect, buf: &mut Buffer);
}

/// A handle to a registered style; cheap to copy and store in app state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StyleId(usize);

impl StyleId {
    pub fn all() -> impl ExactSizeIterator<Item = StyleId> {
        (0..styles::ALL.len()).map(StyleId)
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "config / --style land with lava-xxx")
    )]
    pub fn by_name(name: &str) -> Option<StyleId> {
        Self::all().find(|id| id.style().name() == name)
    }

    pub fn style(self) -> &'static dyn Style {
        styles::ALL[self.0]
    }

    /// Position in the cycle (0-based), for `name  i/n` toasts.
    #[cfg_attr(not(test), expect(dead_code, reason = "toasts land with lava-xxx"))]
    pub fn index(self) -> usize {
        self.0
    }

    pub fn next(self) -> StyleId {
        StyleId((self.0 + 1) % styles::ALL.len())
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "picker lands with lava-xxx"))]
    pub fn prev(self) -> StyleId {
        StyleId((self.0 + styles::ALL.len() - 1) % styles::ALL.len())
    }
}

/// What a style draws from: the sampled field at its grid, plus everything
/// it needs to colour it.
pub struct Canvas<'a> {
    samples: &'a [Sample],
    light: Option<&'a [f32]>,
    /// Per sample row: the container's `[lo, hi)` sample columns.
    mask: &'a [(usize, usize)],
    pub width: usize,
    pub height: usize,
    pub theme: &'a Theme,
    /// Seconds since launch, for styles that animate on their own.
    #[expect(dead_code, reason = "animated styles: lava-y7g")]
    pub time: f64,
}

impl Canvas<'_> {
    /// Sample at pixel (`x`, `y`); `y` runs down.
    #[inline]
    pub fn at(&self, x: usize, y: usize) -> Sample {
        self.samples[y * self.width + x]
    }

    /// Brightness factor at a pixel: 1.0 unless a lighting pass ran.
    #[inline]
    pub fn light(&self, x: usize, y: usize) -> f32 {
        self.light.map_or(1.0, |l| l[y * self.width + x])
    }

    /// Whether the pixel is inside the container (always, in bleed).
    #[inline]
    pub fn inside(&self, x: usize, y: usize) -> bool {
        let (lo, hi) = self.mask[y];
        (lo..hi).contains(&x)
    }

    /// What shows where there's no wax: `liquid` inside the container,
    /// `bg` outside it.
    #[inline]
    pub fn backdrop(&self, x: usize, y: usize) -> Ink {
        Ink::Role(if self.inside(x, y) {
            Role::Liquid
        } else {
            Role::Bg
        })
    }
}

/// Smooth 0 → 1 wax coverage across the surface, for anti-aliasing: half
/// of `EDGE` either side of [`SURFACE`].
#[inline]
pub fn coverage(density: f32) -> f32 {
    const EDGE: f32 = 0.24;
    // Quantised: an edge pixel changes colour only every 1/STEPS of a
    // pixel of movement, not every frame (bandwidth, §7).
    const STEPS: f32 = 6.0;
    (smoothstep((density - (SURFACE - EDGE / 2.0)) / EDGE) * STEPS).round() / STEPS
}

/// Where a wax temperature sits on the wax gradient (0 cool … 1 hot).
/// Wax lives between the cool top liquid and the heater pool.
#[inline]
pub fn wax_heat(temp: f32) -> f32 {
    ((temp - 0.25) / 0.65).clamp(0.0, 1.0)
}

/// Ordered-dither threshold in (0, 1) for pixel (`x`, `y`): an 8×8 Bayer
/// matrix anchored to the canvas, so the pattern is static frame to frame.
#[inline]
pub fn bayer(x: usize, y: usize) -> f32 {
    #[rustfmt::skip]
    const BAYER8: [[u8; 8]; 8] = [
        [ 0, 32,  8, 40,  2, 34, 10, 42],
        [48, 16, 56, 24, 50, 18, 58, 26],
        [12, 44,  4, 36, 14, 46,  6, 38],
        [60, 28, 52, 20, 62, 30, 54, 22],
        [ 3, 35, 11, 43,  1, 33,  9, 41],
        [51, 19, 59, 27, 49, 17, 57, 25],
        [15, 47,  7, 39, 13, 45,  5, 37],
        [63, 31, 55, 23, 61, 29, 53, 21],
    ];
    (f32::from(BAYER8[y % 8][x % 8]) + 0.5) / 64.0
}

#[inline]
pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Scratch buffers that persist across frames, so steady-state rendering
/// never allocates. Keep one per lamp.
#[derive(Debug, Default)]
pub struct LampState {
    samples: Vec<Sample>,
    coarse: Vec<Sample>,
    light: Vec<f32>,
    mask: Vec<(usize, usize)>,
}

/// The lamp as a widget: samples `field` at the style's grid and draws it.
/// This is the one seam the TUI uses:
///
/// ```ignore
/// frame.render_stateful_widget(LampView { field, style, theme, time, lighting: None }, area, &mut lamp_state);
/// ```
pub struct LampView<'a> {
    pub field: &'a Field,
    pub style: &'a dyn Style,
    pub theme: &'a Theme,
    pub time: f64,
    /// Optional lighting pass (lava-5ak).
    pub lighting: Option<&'a dyn Lighting>,
}

impl StatefulWidget for LampView<'_> {
    type State = LampState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut LampState) {
        let area = area.intersection(buf.area);
        if area.is_empty() {
            return;
        }
        let grid = self.style.grid();
        let width = usize::from(area.width) * usize::from(grid.x);
        let height = usize::from(area.height) * usize::from(grid.y);
        let n = width * height;

        state.samples.resize(n, Sample::default());
        if n <= SAMPLE_BUDGET {
            self.field.fill(&mut state.samples, width, height);
        } else {
            let k = (SAMPLE_BUDGET as f64 / n as f64).sqrt();
            let cw = ((width as f64 * k) as usize).max(1);
            let ch = ((height as f64 * k) as usize).max(1);
            state.coarse.resize(cw * ch, Sample::default());
            self.field.fill(&mut state.coarse, cw, ch);
            upsample(&state.coarse, cw, ch, &mut state.samples, width, height);
        }

        let shape = self.field.shape();
        state.mask.clear();
        state.mask.extend((0..height).map(|y| {
            let world_y = 1.0 - (y as f64 + 0.5) / height as f64;
            let half = shape.width_fraction(world_y) * width as f64 / 2.0;
            let lo = (width as f64 / 2.0 - half).round().max(0.0) as usize;
            (lo, width - lo.min(width / 2))
        }));

        let light = match self.lighting {
            Some(lighting) => {
                state.light.clear();
                state.light.resize(n, 1.0);
                lighting.shade(&state.samples, width, height, &mut state.light);
                Some(state.light.as_slice())
            }
            None => None,
        };

        let canvas = Canvas {
            samples: &state.samples,
            light,
            mask: &state.mask,
            width,
            height,
            theme: self.theme,
            time: self.time,
        };
        self.style.draw(&canvas, area, buf);
    }
}

/// Bilinear upsample of a `sw × sh` grid into `dw × dh` (pixel centres
/// aligned).
fn upsample(src: &[Sample], sw: usize, sh: usize, dst: &mut [Sample], dw: usize, dh: usize) {
    let axis = |i: usize, d: usize, s: usize| {
        let p = ((i as f32 + 0.5) * s as f32 / d as f32 - 0.5).clamp(0.0, (s - 1) as f32);
        let i0 = p as usize;
        (i0, (i0 + 1).min(s - 1), p - i0 as f32)
    };
    let lerp = |a: Sample, b: Sample, t: f32| Sample {
        density: a.density + (b.density - a.density) * t,
        temp: a.temp + (b.temp - a.temp) * t,
    };
    for y in 0..dh {
        let (y0, y1, fy) = axis(y, dh, sh);
        for x in 0..dw {
            let (x0, x1, fx) = axis(x, dw, sw);
            let top = lerp(src[y0 * sw + x0], src[y0 * sw + x1], fx);
            let bottom = lerp(src[y1 * sw + x0], src[y1 * sw + x1], fx);
            dst[y * dw + x] = lerp(top, bottom, fy);
        }
    }
}
