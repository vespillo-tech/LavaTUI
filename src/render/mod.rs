//! Render styles: turn the sampled wax field into terminal cells.
//!
//! The pipeline, once per frame, all inside [`LampView`]:
//!
//! 1. The active [`LampStyle`] says how many square sample pixels it wants per
//!    cell ([`Grid`]: half-block 1×2, braille 2×4, …).
//! 2. The field is sampled at that grid into a reused buffer (or at a
//!    reduced grid and upsampled, above [`SAMPLE_BUDGET`]).
//! 3. An optional [`Lighting`] pass fills a per-sample brightness buffer.
//! 4. The style draws the [`Canvas`] (samples + mask + light + theme) into
//!    the buffer, cell by cell, inside its `Rect` only.
//! 5. Cells the container's walls cut through are reshaped to half / quarter
//!    cells (`walls`), so the bottle's silhouette is smooth.
//!
//! Adding a style: one file in `styles/` implementing [`LampStyle`], plus
//! one line in the `styles::ALL` registry.

mod canvas;
mod cell;
mod styles;
#[cfg(test)]
mod tests;
mod walls;

pub use canvas::Canvas;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::StatefulWidget;

use crate::light::Lighting;
use crate::sim::{Field, SURFACE, Sample, WAX_TEMP};
use crate::theme::Theme;

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
    pub const CELL: Grid = Grid { x: 1, y: 1 };
    pub const HALF_BLOCK: Grid = Grid { x: 1, y: 2 };
    pub const BRAILLE: Grid = Grid { x: 2, y: 4 };
}

/// A way of drawing the lamp: implemented by a unit struct per style
/// (stateless; any per-frame scratch lives in [`LampState`]) and listed
/// once in `styles::ALL`.
pub trait LampStyle {
    /// Lowercase name, shown in the UI and used in config.
    const NAME: &'static str;
    /// Sample pixels per cell. The canvas passed to [`draw`](Self::draw)
    /// is exactly `area.width × grid.x` by `area.height × grid.y`.
    const GRID: Grid;
    /// Draw `canvas` into its `area` of `buf`. Must write every cell of the
    /// area and nothing outside it.
    fn draw(canvas: &Canvas, buf: &mut Buffer);
}

/// A registered [`LampStyle`], as the app holds it.
#[derive(Debug)]
pub struct StyleEntry {
    name: &'static str,
    grid: Grid,
    draw: fn(&Canvas, &mut Buffer),
}

impl StyleEntry {
    pub const fn of<S: LampStyle>() -> Self {
        StyleEntry {
            name: S::NAME,
            grid: S::GRID,
            draw: S::draw,
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn grid(&self) -> Grid {
        self.grid
    }

    pub fn draw(&self, canvas: &Canvas, buf: &mut Buffer) {
        (self.draw)(canvas, buf);
    }
}

/// A handle to a registered style; cheap to copy and store in app state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StyleId(usize);

impl StyleId {
    pub fn all() -> impl ExactSizeIterator<Item = StyleId> {
        (0..styles::ALL.len()).map(StyleId)
    }

    /// The style called `name`, or that an old name (an alias) now means.
    pub fn by_name(name: &str) -> Option<StyleId> {
        let name = styles::ALIASES
            .iter()
            .find(|(old, _)| *old == name)
            .map_or(name, |(_, new)| new);
        Self::all().find(|id| id.style().name() == name)
    }

    pub fn style(self) -> &'static StyleEntry {
        &styles::ALL[self.0]
    }

    /// Position in the cycle (0-based), for `name  i/n` toasts.
    pub fn index(self) -> usize {
        self.0
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
    (soft_edge(density, EDGE) * STEPS).round() / STEPS
}

/// Smooth, unquantised 0 → 1 across a band `edge` wide centred on
/// [`SURFACE`].
#[inline]
pub fn soft_edge(density: f32, edge: f32) -> f32 {
    smoothstep((density - (SURFACE - edge / 2.0)) / edge)
}

/// Where a wax temperature sits on the wax gradient (0 cool … 1 hot):
/// its place in the sim's [`WAX_TEMP`] span.
#[inline]
pub fn wax_heat(temp: f32) -> f32 {
    const SPAN: f32 = WAX_TEMP.1 - WAX_TEMP.0;
    ((temp - WAX_TEMP.0) / SPAN).clamp(0.0, 1.0)
}

/// Lighting as a nudge along a 0..1 level (heat, glyph density) rather
/// than a colour scale, for depths that can't blend: "lighting adds
/// density, not colour" (docs/design.md §5.3). Shadow sides step down,
/// highlights and glow step up; unlit (1.0) leaves `level` as it is.
#[inline]
pub fn lit(level: f32, light: f32) -> f32 {
    const GAIN: f32 = 0.6;
    if light == 1.0 {
        return level;
    }
    (level + GAIN * (light - 1.0)).clamp(0.0, 1.0)
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
/// frame.render_stateful_widget(LampView { field, style, theme, time, lighting: None, options: LampOptions::default() }, area, &mut lamp_state);
/// ```
pub struct LampView<'a> {
    pub field: &'a Field,
    pub style: &'a StyleEntry,
    pub theme: &'a Theme,
    pub time: f64,
    /// Optional lighting pass (lava-5ak).
    pub lighting: Option<&'a dyn Lighting>,
    pub options: LampOptions,
}

/// How a [`LampView`] draws, beyond the style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LampOptions {
    /// Sample at half resolution per axis and upsample (adaptive quality,
    /// docs/design.md §7).
    pub reduced: bool,
    /// Leave cells outside the container to the terminal's own background
    /// (`theme.transparent`, §9) instead of painting `bg`.
    pub transparent: bool,
}

/// Samples a frame of `n` grid pixels actually takes: the budget caps it,
/// and a reduced grid takes a quarter.
pub fn samples_taken(n: usize, reduced: bool) -> usize {
    sample_budget(n, reduced).min(n)
}

fn sample_budget(n: usize, reduced: bool) -> usize {
    if reduced {
        SAMPLE_BUDGET.min(n.div_ceil(4))
    } else {
        SAMPLE_BUDGET
    }
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
        let budget = sample_budget(n, self.options.reduced);
        if n <= budget {
            self.field.fill(&mut state.samples, width, height);
        } else {
            let k = (budget as f64 / n as f64).sqrt();
            let cw = ((width as f64 * k) as usize).max(1);
            let ch = ((height as f64 * k) as usize).max(1);
            state.coarse.resize(cw * ch, Sample::default());
            self.field.fill(&mut state.coarse, cw, ch);
            upsample(&state.coarse, cw, ch, &mut state.samples, width, height);
        }

        let shape = self.field.shape();
        walls::mask(shape, area, grid, &mut state.mask);

        let light = match self.lighting {
            Some(lighting) => {
                state.light.clear();
                state.light.resize(n, 1.0);
                lighting.shade(&state.samples, width, height, &mut state.light);
                // Light lives inside the container: outside the glass is
                // the app background, which stays unlit.
                for (row, &(lo, hi)) in state.light.chunks_exact_mut(width).zip(&state.mask) {
                    row[..lo].fill(1.0);
                    row[hi..].fill(1.0);
                }
                Some(state.light.as_slice())
            }
            None => None,
        };

        let canvas = Canvas {
            area,
            samples: &state.samples,
            light,
            mask: &state.mask,
            width,
            height,
            theme: self.theme,
            time: self.time,
        };
        self.style.draw(&canvas, buf);
        if self.options.transparent {
            walls::clear_outside(shape, area, buf);
        }
        let outside = self.theme.background(self.options.transparent);
        if self.theme.blends() {
            walls::smooth(shape, self.theme, outside, area, buf);
        }
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
