//! Optional lighting / glow pass over the sampled field.
//!
//! The seam is [`Lighting`]: `render::LampView` fills its sample grid, then
//! (when a lighting pass is given) asks it for a brightness factor per
//! sample, which styles read through `Canvas::light`.
//!
//! [`Lamplight`] is the one real pass. It reads only the samples, in one
//! sweep, with no scratch buffers:
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

use crate::render::smoothstep;
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
const RUN: usize = 16;

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
        let rig = Rig::new(height);
        let base_from = 1.0 - BASE_REACH;
        for y in 0..height {
            let row = &samples[y * width..(y + 1) * width];
            // Rows run down the screen: `up` is the row above.
            let up = &samples[y.saturating_sub(1) * width..][..width];
            let down = &samples[(y + 1).min(height - 1) * width..][..width];
            let out = &mut out[y * width..(y + 1) * width];

            let v = (y as f32 + 0.5) / height as f32;
            let base = ((v - base_from) / BASE_REACH).clamp(0.0, 1.0);
            let base = BASE * base * base;

            // Edge columns repeat their own sample as the missing neighbour.
            let last = width - 1;
            let edge = |x: usize| {
                let (l, r) = (row[x.saturating_sub(1)], row[(x + 1).min(last)]);
                rig.pixel(
                    row[x],
                    l.density,
                    r.density,
                    up[x].density,
                    down[x].density,
                    base,
                )
            };
            out[0] = edge(0);
            out[last] = edge(last);

            // The interior in runs, cheapest case first: open liquid (no
            // wax reaches it) is just the base light; liquid in a blob's
            // tail only needs the glow; runs with wax get the full model.
            // Each is a straight zip, which the compiler vectorises.
            let open = quantize(1.0 + base);
            for x0 in (1..last).step_by(RUN) {
                let x1 = (x0 + RUN).min(last);
                let densest = row[x0..x1].iter().fold(0.0f32, |m, s| m.max(s.density));
                if densest <= 0.0 {
                    out[x0..x1].fill(open);
                    continue;
                }
                if densest <= WAX_FROM {
                    for (s, o) in row[x0..x1].iter().zip(&mut out[x0..x1]) {
                        *o = quantize(rig.liquid(*s, base));
                    }
                    continue;
                }
                let run = row[x0..x1]
                    .iter()
                    .zip(&row[x0 - 1..x1 - 1])
                    .zip(&row[x0 + 1..x1 + 1])
                    .zip(&up[x0..x1])
                    .zip(&down[x0..x1])
                    .zip(&mut out[x0..x1]);
                for (((((s, l), r), u), d), o) in run {
                    *o = rig.pixel(*s, l.density, r.density, u.density, d.density, base);
                }
            }
        }
    }
}

/// The light setup, resolved once per frame.
struct Rig {
    /// Unit vector toward the key light.
    key: [f32; 3],
    /// Blinn-Phong half vector (key + view).
    half: [f32; 3],
    /// Diffuse level facing the viewer straight on, which maps to 1.0.
    flat: f32,
    /// Density difference across two pixels → gradient per world unit.
    grad_scale: f32,
}

impl Rig {
    fn new(height: usize) -> Self {
        let key = normalize(KEY);
        Rig {
            key,
            half: normalize([key[0], key[1], key[2] + 1.0]),
            flat: diffuse(key[2]),
            // Central differences span two pixels; a pixel is 1/height
            // world units.
            grad_scale: 0.5 * height as f32,
        }
    }

    /// Brightness of one sample from its neighbours' densities. Branch-free
    /// so the row loop vectorises.
    #[inline(always)]
    fn pixel(&self, s: Sample, left: f32, right: f32, up: f32, down: f32, base: f32) -> f32 {
        let heat = heat(s.temp);
        let liquid = self.liquid(s, base);

        // Wax: a dome normal from depth + gradient, lit by the key.
        let gx = (right - left) * self.grad_scale;
        let gy = (up - down) * self.grad_scale;
        let n = normal(gx, gy, (s.density - SURFACE) * (1.0 / DOME));
        let shade = diffuse(dot(n, self.key)) - self.flat;
        let spec = pow32(dot(n, self.half).max(0.0));
        let lit =
            1.0 + (DIFFUSE * shade + SPECULAR * spec) * (1.0 - SELF_GLOW * heat) + 0.15 * base;

        let wax = smoothstep((s.density - WAX_FROM) * (1.0 / EDGE));
        quantize(liquid + (lit - liquid) * wax)
    }

    /// Liquid brightness: base light + glow from the kernel tail of hot wax.
    #[inline(always)]
    fn liquid(&self, s: Sample, base: f32) -> f32 {
        let reach = (s.density * (1.0 / SURFACE)).clamp(0.0, 1.0);
        let glow = GLOW * reach * reach * heat(s.temp);
        1.0 + (base + glow).min(LIQUID_MAX)
    }
}

/// Surface normal (x right, y up, z out of the screen) for a wax pixel
/// with density gradient (`gx`, `gy`) in world units (y up) and `depth`
/// above the surface in [`DOME`]s. Points outward (down the gradient),
/// edge-on at the surface, facing the viewer at depth ≥ 1 or where the
/// field is flat. Unit length; finite for finite input.
#[inline(always)]
fn normal(gx: f32, gy: f32, depth: f32) -> [f32; 3] {
    // Near a blob's centre density falls off with r², so `1 - depth` ∝ r²
    // and its square root ∝ r: a sphere's profile.
    let sphere = (1.0 - depth.clamp(0.0, 1.0)).sqrt();
    // Tilt fades out where the field is flat (no trustworthy direction):
    // sin θ = sphere · len / (len + FLAT), so the direction (g / len)
    // times sin θ needs no division by `len`.
    let len = (gx * gx + gy * gy).sqrt();
    let k = sphere / (len + FLAT);
    let sin = k * len;
    let cos = (1.0 - sin * sin).max(0.0).sqrt();
    [-gx * k, -gy * k, cos]
}

/// As `render::wax_heat`, with the divide folded into a multiply (this
/// loop is arithmetic-bound).
#[inline(always)]
fn heat(temp: f32) -> f32 {
    ((temp - 0.25) * (1.0 / 0.65)).clamp(0.0, 1.0)
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
