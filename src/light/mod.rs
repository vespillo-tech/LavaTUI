//! Optional lighting / glow pass over the sampled field.
//!
//! The seam is [`Lighting`]: `render::LampView` fills its sample grid, then
//! (when a lighting pass is given) asks it for a brightness factor per
//! sample, which styles read through `Canvas::light`.
//!
//! [`Lamplight`] is the one real pass. It reads only the samples, one sweep
//! down the rows, with no heap scratch:
//!
//! * **Shape.** Each wax pixel gets a normal: its tilt comes from how deep
//!   it sits inside the surface (rim = edge-on, core = facing you), its
//!   direction from the density gradient (outward). So every blob reads as
//!   a soft dome whatever its size or the resolution.
//! * **Key light.** Half-Lambert from a fixed light up and to the left, so
//!   the lower-right of each blob falls into a gentle shadow, plus a small
//!   Blinn-Phong highlight near the top-left of each dome. Hot wax glows on
//!   its own, so its shading is flattened (which also keeps the hot pool,
//!   whose surface is a crease-prone slab, free of streaky highlights).
//! * **Glow.** The metaball kernel already falls off smoothly outside the
//!   surface; that tail, weighted by the wax's heat, brightens the liquid
//!   around hot blobs (and above the hot pool) for free, with no blur pass.
//! * **Base.** The heater lights the bottom third of the liquid, fading
//!   upward (docs/design.md §2.1).
//!
//! Sample pixels are square (every `Grid` is), so the gradient needs no
//! aspect correction. Tuning constants are below; keep it subtle.
//!
//! **Cost.** It runs every frame over every sample (96 k for braille at
//! 200×60), so the sweep is built for it: each row goes in runs of [`RUN`]
//! pixels, and a run only pays for what reaches it (open liquid is the base
//! light alone, the glow tail adds the glow, only runs with wax get the
//! dome); every loop is a branch-free zip the compiler vectorises. On fine
//! grids (see [`FINE`]) the dome shading, the costly smooth part, comes
//! from a half-resolution grid of nodes and is interpolated; the wax edge
//! stays per pixel.

use std::ops::Range;

use crate::sim::{SURFACE, Sample};

/// Computes per-sample brightness from the sampled field.
pub trait Lighting {
    /// Fill `out` (row-major, `width × height`, the same layout as
    /// `samples`) with a brightness factor: 1.0 = unlit, > 1 brighter.
    /// `out` arrives filled with 1.0.
    fn shade(&self, samples: &[Sample], width: usize, height: usize, out: &mut [f32]);
}

/// Direction *toward* the key light: x right, y up, z toward the viewer.
/// Up-left and mostly in front, so domes are lit, not rimmed.
const KEY: [f32; 3] = [-0.45, 0.6, 0.66];
/// How much the key light's direction moves wax brightness, around 1.0.
const DIFFUSE: f32 = 0.6;
/// Peak specular brightness added on blob tops.
const SPECULAR: f32 = 0.32;
/// Density above the surface over which a dome rises from edge-on (at the
/// surface) to facing the viewer: about a lone blob's peak, so each blob
/// curves like a sphere all the way to its centre.
const DOME: f32 = 0.5;
/// Gradient magnitude (density per world unit) below which the wax counts
/// as flat (deep inside merged wax, the pool's interior).
const FLAT: f32 = 0.6;
/// How much hot wax ignores the key light (it glows on its own): 0..1.
/// Also keeps creases in the hot pool from catching streaky highlights.
const SELF_GLOW: f32 = 0.7;
/// Liquid brightness gain right next to the hottest wax.
const GLOW: f32 = 0.9;
/// Liquid brightness gain at the very bottom, from the heater.
const BASE: f32 = 0.55;
/// Share of the canvas height the base light reaches up.
const BASE_REACH: f32 = 1.0 / 3.0;
/// Most the liquid is ever brightened (glow + base), so the hot pool and
/// heater together don't wash the bottom out.
const LIQUID_MAX: f32 = 1.0;
/// Width of the wax/liquid blend across the surface (as `render::coverage`).
const EDGE: f32 = 0.24;
/// Density where the wax starts blending in over the liquid.
const WAX_FROM: f32 = SURFACE - EDGE / 2.0;
/// Light levels per unit of brightness (see [`quantize`]).
const STEPS: f32 = 24.0;
/// Pixels per run when picking the cheapest model for a stretch of row.
/// Even, so runs start on a node (see [`FINE`]). Shorter runs skip more
/// liquid, longer ones cost less to classify; 16 measures best.
const RUN: usize = 16;
/// Canvas height (pixels per world unit) from which the dome shading is
/// computed on nodes, every other pixel of every other row, and
/// interpolated between. It varies over a blob's radius, many pixels at
/// this resolution (braille at 200×60 is 240); the sharp wax/liquid edge,
/// the glow and the base light stay per pixel.
///
/// What nodes can't hold are the creases the dome model draws a pixel
/// wide: where a core's normal snaps to face-on (density `SURFACE + DOME`)
/// and along ridges and valleys that peak below it (tails, necks, where
/// merged lobes meet), where the normal flips side within a pixel. Those
/// come out two pixels wide and a few light steps softer. At cell size
/// that's a shade in about one cell in a thousand and the odd braille
/// stipple dot (measured on the live sim), so it isn't worth shading them
/// exactly: catching them costs about as much as the exact pass.
///
/// Below this (half-block grids) interpolation starts to flip glyphs in
/// the dithered styles, so it stays exact.
const FINE: usize = 160;
/// Widest canvas shaded coarse (node rows live on the stack); wider ones
/// (only ever a sliver of a lamp) are shaded exactly.
const COARSE_WIDTH: usize = 4096;
/// Nodes per run (one per pixel pair). A run interpolates up to the next
/// run's first node too.
const NODES: usize = RUN / 2;
/// Canvas rows per node row.
const NODE_ROWS: usize = 2;
/// Node blocks in a row of the widest coarse canvas: one per run, plus one
/// past the last.
const BLOCKS: usize = COARSE_WIDTH / RUN + 1;

/// The lamp's lighting: key light + highlight on the wax, glow around hot
/// wax and warm light from the base. Stateless; share one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lamplight;

impl Lighting for Lamplight {
    fn shade(&self, samples: &[Sample], width: usize, height: usize, out: &mut [f32]) {
        debug_assert_eq!(samples.len(), width * height);
        debug_assert_eq!(out.len(), width * height);
        if width == 0 || height == 0 {
            return;
        }
        let rig = Rig::new(width, height);
        if height >= FINE && width <= COARSE_WIDTH {
            rig.shade_coarse(samples, out);
        } else {
            rig.shade_exact(samples, out);
        }
    }
}

/// What a run of pixels needs, cheapest first.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Run {
    /// No wax reaches it: just the base light.
    Open,
    /// Liquid in a blob's tail: base + glow.
    Tail,
    /// Wax somewhere: the full model.
    Wax,
}

impl Run {
    #[inline(always)]
    fn of(run: &[Sample]) -> Run {
        // Densities are ≥ 0, so their bits order as integers (an integer
        // max vectorises; a float one is a serial chain). A NaN or negative
        // one reads as wax, which is always safe.
        let densest = run.iter().fold(0, |m, s| s.density.to_bits().max(m));
        if densest == 0 {
            Run::Open
        } else if densest <= WAX_FROM.to_bits() {
            Run::Tail
        } else {
            Run::Wax
        }
    }
}

/// The light setup, resolved once per frame.
struct Rig {
    width: usize,
    height: usize,
    /// Unit vector toward the key light.
    key: [f32; 3],
    /// Blinn-Phong half vector (key + view).
    half: [f32; 3],
    /// Diffuse level facing the viewer straight on, which maps to 1.0.
    flat: f32,
    /// [`FLAT`] as a density difference across two pixels (the central
    /// differences the gradient is taken from; a pixel is 1/height world
    /// units).
    flat_diff: f32,
}

impl Rig {
    fn new(width: usize, height: usize) -> Self {
        let key = normalize(KEY);
        Rig {
            width,
            height,
            key,
            half: normalize([key[0], key[1], key[2] + 1.0]),
            flat: diffuse(key[2]),
            flat_diff: FLAT / (0.5 * height as f32),
        }
    }

    /// Every pixel exactly, row by row, in [`Run`]s. Runs with wax are
    /// gathered into stretches and shaded a stretch at a time (long loops
    /// beat many short ones). Each step is a straight zip, which the
    /// compiler vectorises.
    fn shade_exact(&self, samples: &[Sample], out: &mut [f32]) {
        let rows = samples
            .chunks_exact(self.width)
            .zip(out.chunks_exact_mut(self.width));
        for (y, (row, out)) in rows.enumerate() {
            let base = self.base(y);
            let mut stretch = None;
            for (r, run) in row.chunks(RUN).enumerate() {
                let x0 = r * RUN;
                let kind = Run::of(run);
                if kind == Run::Wax {
                    stretch.get_or_insert(x0);
                    continue;
                }
                if let Some(from) = stretch.take() {
                    self.exact_span(samples, y, from..x0, base, &mut out[from..x0]);
                }
                let out = &mut out[x0..x0 + run.len()];
                if kind == Run::Open {
                    out.fill(quantize(1.0 + base));
                } else {
                    for (s, o) in run.iter().zip(out) {
                        *o = quantize(self.liquid(self.glow(*s), base));
                    }
                }
            }
            if let Some(from) = stretch {
                self.exact_span(samples, y, from..self.width, base, &mut out[from..]);
            }
        }
    }

    /// Exact light for pixels `span` of row `y`, which has wax.
    fn exact_span(
        &self,
        samples: &[Sample],
        y: usize,
        span: Range<usize>,
        base: f32,
        out: &mut [f32],
    ) {
        let width = self.width;
        let at = |y: usize| &samples[y * width..][..width];
        // Rows run down the screen: `up` is the row above. Edges repeat
        // their own sample as the missing neighbour.
        let (row, up, down) = (
            at(y),
            at(y.saturating_sub(1)),
            at((y + 1).min(self.height - 1)),
        );
        let (x0, x1, last) = (span.start, span.end, width - 1);
        debug_assert_eq!(out.len(), x1 - x0);
        let light = |s: Sample, left: f32, right: f32, up: f32, down: f32| {
            let dome = self.dome(s, left, right, up, down);
            quantize(self.blend(s, dome, self.liquid(self.glow(s), base), base))
        };
        // The interior `a..b` has both neighbours; the edges clamp.
        let a = x0.max(1);
        let b = x1.min(last).max(a);
        for x in (x0..a.min(x1)).chain(b..x1) {
            let (l, r) = (row[x.saturating_sub(1)], row[(x + 1).min(last)]);
            out[x - x0] = light(row[x], l.density, r.density, up[x].density, down[x].density);
        }
        if a >= b {
            return;
        }
        let inner = row[a..b]
            .iter()
            .zip(&row[a - 1..b - 1])
            .zip(&row[a + 1..b + 1])
            .zip(&up[a..b])
            .zip(&down[a..b])
            .zip(&mut out[a - x0..b - x0]);
        for (((((s, l), r), u), d), o) in inner {
            *o = light(*s, l.density, r.density, u.density, d.density);
        }
    }

    /// As [`shade_exact`](Self::shade_exact), but the dome shading comes
    /// from [`Nodes`], interpolated, a run at a time.
    fn shade_coarse(&self, samples: &[Sample], out: &mut [f32]) {
        let mut nodes = Nodes::new(self.height);
        let rows = samples
            .chunks_exact(self.width)
            .zip(out.chunks_exact_mut(self.width));
        for (y, (row, out)) in rows.enumerate() {
            let base = self.base(y);
            nodes.enter(y);
            let runs = row.chunks(RUN).zip(out.chunks_mut(RUN)).enumerate();
            for (r, (row, out)) in runs {
                match Run::of(row) {
                    Run::Open => out.fill(quantize(1.0 + base)),
                    Run::Tail => {
                        for (s, o) in row.iter().zip(out) {
                            *o = quantize(self.liquid(self.glow(*s), base));
                        }
                    }
                    Run::Wax => {
                        let dome = nodes.get(
                            y,
                            r,
                            |y, x0, nodes| self.dome_nodes(samples, y, x0, nodes),
                            |y, x| self.dome_at(samples, y, x),
                        );
                        spread(&dome, out);
                        for (s, o) in row.iter().zip(out) {
                            let liquid = self.liquid(self.glow(*s), base);
                            *o = quantize(self.blend(*s, *o, liquid, base));
                        }
                    }
                }
            }
        }
    }

    /// Bare dome shading on canvas row `y` for the run starting at `x0`:
    /// one node per pixel pair, on the even pixel, clamped to the edge.
    fn dome_nodes(&self, samples: &[Sample], y: usize, x0: usize, nodes: &mut [f32; NODES]) {
        let width = self.width;
        let at = |y: usize| &samples[y.min(self.height - 1) * width..][..width];
        let (row, up, down) = (at(y), at(y.saturating_sub(1)), at(y + 1));
        if x0 >= 1 && x0 + RUN <= width {
            // Interior: walk pixel pairs, which the compiler turns into
            // vector loads that split even and odd pixels.
            fn pairs(row: &[Sample], from: usize) -> std::slice::Iter<'_, [Sample; 2]> {
                row[from..from + RUN].as_chunks().0.iter()
            }
            let span = pairs(row, x0)
                .zip(pairs(row, x0 - 1))
                .zip(pairs(up, x0))
                .zip(pairs(down, x0))
                .zip(nodes);
            for ((((here, left), up), down), node) in span {
                let (l, r) = (left[0].density, here[1].density);
                *node = self.dome(here[0], l, r, up[0].density, down[0].density);
            }
        } else {
            for (k, node) in nodes.iter_mut().enumerate() {
                *node = self.dome_at(samples, y, x0 + 2 * k);
            }
        }
    }

    /// Bare dome shading at pixel (`x`, `y`), both clamped to the canvas.
    fn dome_at(&self, samples: &[Sample], y: usize, x: usize) -> f32 {
        let (width, last) = (self.width, self.width - 1);
        let at = |y: usize| &samples[y.min(self.height - 1) * width..][..width];
        let (y, x) = (y.min(self.height - 1), x.min(last));
        let (row, up, down) = (at(y), at(y.saturating_sub(1)), at(y + 1));
        self.dome(
            row[x],
            row[x.saturating_sub(1)].density,
            row[(x + 1).min(last)].density,
            up[x].density,
            down[x].density,
        )
    }

    /// Base light at row `y`: the heater, in the bottom [`BASE_REACH`].
    fn base(&self, y: usize) -> f32 {
        let v = (y as f32 + 0.5) / self.height as f32;
        let base = unit((v - (1.0 - BASE_REACH)) / BASE_REACH);
        BASE * base * base
    }

    /// Bare dome shading of a wax sample from its neighbours' densities:
    /// the key light and highlight on a dome normal from depth + gradient,
    /// flattened on hot wax. Branch-free so the row loops vectorise.
    #[inline(always)]
    fn dome(&self, s: Sample, left: f32, right: f32, up: f32, down: f32) -> f32 {
        // Raw differences: only the gradient's direction and its size
        // against `FLAT` matter, so the scale goes on `flat_diff` instead.
        let depth = (s.density - SURFACE) * (1.0 / DOME);
        let n = normal(right - left, up - down, depth, self.flat_diff);
        let shade = diffuse(dot(n, self.key)) - self.flat;
        let spec = pow32(dot(n, self.half).max(0.0));
        1.0 + (DIFFUSE * shade + SPECULAR * spec) * (1.0 - SELF_GLOW * heat(s.temp))
    }

    /// Light of a sample from its bare `dome` shading and `liquid` light:
    /// the lit wax (with a little base light) blended over the liquid
    /// across the surface.
    #[inline(always)]
    fn blend(&self, s: Sample, dome: f32, liquid: f32, base: f32) -> f32 {
        let lit = dome + 0.15 * base;
        let wax = unit((s.density - WAX_FROM) * (1.0 / EDGE));
        let wax = wax * wax * (3.0 - 2.0 * wax);
        liquid + (lit - liquid) * wax
    }

    /// Glow from the kernel tail of hot wax around a sample.
    #[inline(always)]
    fn glow(&self, s: Sample) -> f32 {
        let reach = unit(s.density * (1.0 / SURFACE));
        GLOW * reach * reach * heat(s.temp)
    }

    /// Liquid brightness: base light + glow.
    #[inline(always)]
    fn liquid(&self, glow: f32, base: f32) -> f32 {
        1.0 + (base + glow).min(LIQUID_MAX)
    }
}

/// Dome shading nodes for coarse shading, computed on demand a run's worth
/// (a block) at a time: node row `j` is canvas row `NODE_ROWS · j`, and
/// each canvas row reads the node rows at and below it, so two rolling
/// rows do. Lives on the stack (~16 KB).
struct Nodes {
    /// The canvas's last row: node rows past it clamp to it.
    last: usize,
    /// `[j % 2][block]`.
    values: [[[f32; NODES]; BLOCKS]; 2],
    /// How much of each block is in: nothing, its first node only (all a
    /// run before it needs), or all of it.
    done: [[Done; BLOCKS]; 2],
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Done {
    None,
    First,
    All,
}

impl Nodes {
    fn new(height: usize) -> Self {
        Nodes {
            last: height - 1,
            values: [[[0.0; NODES]; BLOCKS]; 2],
            done: [[Done::None; BLOCKS]; 2],
        }
    }

    /// Moving on to canvas row `y`: on a node row, the node row below is a
    /// new one.
    fn enter(&mut self, y: usize) {
        if y.is_multiple_of(NODE_ROWS) {
            self.done[(y / NODE_ROWS + 1) % 2].fill(Done::None);
        }
    }

    /// Nodes for run `r` of canvas row `y` (and the next run's first),
    /// from the node row at `y`, or between rows interpolated from the two
    /// either side. Missing nodes come from `block(row, x0, nodes)` (a
    /// run's worth) and `one(row, x)`.
    #[inline(always)]
    fn get(
        &mut self,
        y: usize,
        r: usize,
        mut block: impl FnMut(usize, usize, &mut [f32; NODES]),
        mut one: impl FnMut(usize, usize) -> f32,
    ) -> [f32; NODES + 1] {
        let j = y / NODE_ROWS;
        // Node row `j + 1` sits on the last canvas row if it would fall
        // past it, so weigh by where it really is.
        let (top, bottom) = (NODE_ROWS * j, (NODE_ROWS * (j + 1)).min(self.last));
        let t = (y - top) as f32 / (bottom - top).max(1) as f32;
        let rows: &[(usize, f32)] = if t == 0.0 {
            &[(j, 1.0)]
        } else {
            &[(j, 1.0 - t), (j + 1, t)]
        };
        let mut out = [0.0; NODES + 1];
        for &(j, weight) in rows {
            let (values, done) = (&mut self.values[j % 2], &mut self.done[j % 2]);
            if done[r] != Done::All {
                block((NODE_ROWS * j).min(self.last), r * RUN, &mut values[r]);
                done[r] = Done::All;
            }
            if done[r + 1] == Done::None {
                values[r + 1][0] = one((NODE_ROWS * j).min(self.last), (r + 1) * RUN);
                done[r + 1] = Done::First;
            }
            for (o, v) in out.iter_mut().zip(&values[r]) {
                *o += weight * v;
            }
            out[NODES] += weight * values[r + 1][0];
        }
        out
    }
}

/// Interpolate a run from its nodes: even pixels sit on a node, odd ones
/// halfway to the next.
#[inline(always)]
fn spread(nodes: &[f32; NODES + 1], out: &mut [f32]) {
    let (here, next) = (&nodes[..NODES], &nodes[1..]);
    if let Ok(out) = <&mut [f32; RUN]>::try_from(&mut *out) {
        // A whole run: fixed size, so this unrolls into a few vector ops.
        for ((pair, a), b) in out.as_chunks_mut::<2>().0.iter_mut().zip(here).zip(next) {
            pair[0] = *a;
            pair[1] = 0.5 * (a + b);
        }
        return;
    }
    for (x, o) in out.iter_mut().enumerate() {
        let k = x / 2;
        *o = if x % 2 == 0 {
            here[k]
        } else {
            0.5 * (here[k] + next[k])
        };
    }
}

/// Surface normal (x right, y up, z out of the screen) for a wax pixel
/// with density gradient (`gx`, `gy`) (y up) and `depth` above the surface
/// in [`DOME`]s, where a gradient of size `flat` is [`FLAT`]. Points
/// outward (down the gradient), edge-on at the surface, facing the viewer
/// at depth ≥ 1 or where the field is flat. Unit length; finite for finite
/// input.
#[inline(always)]
fn normal(gx: f32, gy: f32, depth: f32, flat: f32) -> [f32; 3] {
    // Near a blob's centre density falls off with r², so `1 - depth` ∝ r²
    // and its square root ∝ r: a sphere's profile.
    let sphere = (1.0 - unit(depth)).sqrt();
    // Tilt fades out where the field is flat (no trustworthy direction):
    // sin θ = sphere · len / (len + FLAT), so the direction (g / len)
    // times sin θ needs no division by `len`.
    let len = (gx * gx + gy * gy).sqrt();
    let k = sphere / (len + flat);
    let sin = k * len;
    let cos = (1.0 - sin * sin).max(0.0).sqrt();
    [-gx * k, -gy * k, cos]
}

/// As `render::wax_heat`, with the divide folded into a multiply (this
/// loop is arithmetic-bound).
#[inline(always)]
fn heat(temp: f32) -> f32 {
    unit((temp - 0.25) * (1.0 / 0.65))
}

/// Clamp to 0..=1 (NaN → 0). `f32::clamp` keeps NaN, which stops the
/// row loops from vectorising; `max` + `min` doesn't.
#[inline(always)]
#[expect(clippy::manual_clamp, reason = "f32::clamp doesn't vectorise here")]
fn unit(x: f32) -> f32 {
    x.max(0.0).min(1.0)
}

/// Snap to [`STEPS`] levels per unit, so slow motion repaints a cell only
/// when its light moves a visible step, not every frame (bandwidth, §7).
#[inline(always)]
fn quantize(light: f32) -> f32 {
    (light * STEPS).round() * (1.0 / STEPS)
}

/// Half-Lambert, squared: soft wrap-around falloff, never fully black.
#[inline]
fn diffuse(n_dot_l: f32) -> f32 {
    let h = 0.5 + 0.5 * n_dot_l;
    h * h
}

#[inline]
fn pow32(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    let x8 = x4 * x4;
    let x16 = x8 * x8;
    x16 * x16
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = dot(v, v).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}

#[cfg(test)]
mod tests;
