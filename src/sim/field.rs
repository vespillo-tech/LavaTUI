//! The metaball field renderers sample.
//!
//! Coordinates are normalised to the lamp viewport: `u` runs 0 → 1 left to
//! right, `v` runs 0 → 1 **top to bottom** (screen order). The viewport keeps
//! the lamp's visual aspect, so square sample pixels see round blobs.
//!
//! Each blob adds a smooth, compactly supported bump `PEAK · (1 − q²)²`,
//! where `q` is the elliptical distance in units of `SUPPORT × radius`. The
//! bump crosses [`SURFACE`] exactly at the blob's radius, so thresholding at
//! `SURFACE` draws each lone blob at its true size, and nearby blobs fuse
//! with a neck instead of just overlapping. The pool adds a soft slab along
//! the base.

use super::{Shape, World, ambient_temp, pool_surface};

/// Density at a wax surface. Inside is `>= SURFACE`; a lone blob peaks at
/// about 1.05 and overlaps go higher, so clamp before mapping to colour.
pub const SURFACE: f32 = 0.5;

/// Kernel reach, in blob radii.
const SUPPORT: f32 = 1.8;
/// Scales the kernel so it equals [`SURFACE`] at one radius.
const PEAK: f32 = {
    let q2 = 1.0 / (SUPPORT * SUPPORT);
    SURFACE / ((1.0 - q2) * (1.0 - q2))
};
/// Half-thickness of the pool's soft surface.
const POOL_BAND: f32 = 0.035;
/// Weight given to the liquid's temperature when blending, so `temp` fades
/// smoothly from wax to liquid at the edges.
const LIQUID_WEIGHT: f32 = 0.02;

/// One field sample.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Sample {
    /// Wax density; `>= SURFACE` is inside wax.
    pub density: f32,
    /// Temperature 0 (cold) … 1 (hot): of the wax where there is wax,
    /// fading to the liquid's (warm at the base, cool at the top) outside.
    pub temp: f32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Kernel {
    x: f32,
    pub(super) y: f32,
    /// `1 / (SUPPORT · half-extent)²` per axis.
    inv_x2: f32,
    inv_y2: f32,
    /// Support half-extents (bounding box for culling).
    reach_x: f32,
    reach_y: f32,
    temp: f32,
}

/// A frame's snapshot of the wax, ready to sample. Keep one around and call
/// [`Field::prepare`] each frame: it reuses its buffers, so steady-state
/// sampling never allocates.
#[derive(Debug, Default)]
pub struct Field {
    pub(super) kernels: Vec<Kernel>,
    shape: Shape,
    view_width: f32,
    wall_width: f64,
    pool_level: f64,
    time: f64,
}

impl Field {
    /// Snapshot `world`, interpolated `alpha` (0..=1) of the way from the
    /// previous step to the current one (`FixedStep::alpha`).
    pub fn prepare(&mut self, world: &World, alpha: f64) {
        let alpha = alpha.clamp(0.0, 1.0);
        self.kernels.clear();
        self.kernels.extend(world.blobs.iter().map(|blob| {
            let pose = blob.prev.lerp(blob.pose(), alpha);
            let reach = f64::from(SUPPORT) * pose.radius;
            let (reach_x, reach_y) = (reach / pose.stretch, reach * pose.stretch);
            Kernel {
                x: pose.x as f32,
                y: pose.y as f32,
                inv_x2: (1.0 / (reach_x * reach_x)) as f32,
                inv_y2: (1.0 / (reach_y * reach_y)) as f32,
                reach_x: reach_x as f32,
                reach_y: reach_y as f32,
                temp: blob.temp as f32,
            }
        }));
        self.shape = world.shape;
        self.view_width = world.view_width as f32;
        self.wall_width = world.wall_width;
        self.pool_level =
            world.prev_pool_level + (world.pool_level() - world.prev_pool_level) * alpha;
        self.time = world.time - (1.0 - alpha) * world.last_dt;
    }

    /// Viewport width in world units (the lamp's visual aspect).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by render styles (lava-bdj)")
    )]
    pub fn aspect(&self) -> f32 {
        self.view_width
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by render styles (lava-bdj)")
    )]
    /// Container shape, for masking the glass.
    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// Sample one point (`u`, `v` normalised, `v` down).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by render styles (lava-bdj)")
    )]
    pub fn sample(&self, u: f32, v: f32) -> Sample {
        let x = (u - 0.5) * self.view_width;
        let y = 1.0 - v;
        let mut acc = Sample::default();
        for k in &self.kernels {
            let (dx, dy) = (x - k.x, y - k.y);
            if dx.abs() < k.reach_x && dy.abs() < k.reach_y {
                add_kernel(&mut acc, k, dx * dx * k.inv_x2 + dy * dy * k.inv_y2);
            }
        }
        if self.in_container(f64::from(x), f64::from(y)) {
            let surface = pool_surface(self.pool_level, f64::from(x), self.time) as f32;
            add_pool(&mut acc, surface - y);
        }
        finish(&mut acc, ambient_temp(f64::from(y)) as f32);
        acc
    }

    /// Fill `out` (row-major, `cols × rows`) with samples at pixel centres:
    /// pixel `(i, j)` is `sample((i + ½) / cols, (j + ½) / rows)`. Cost
    /// scales with the wax area on screen, not blobs × pixels.
    pub fn fill(&self, out: &mut [Sample], cols: usize, rows: usize) {
        assert_eq!(out.len(), cols * rows, "fill: buffer is not cols × rows");
        out.fill(Sample::default());
        if cols == 0 || rows == 0 {
            return;
        }
        let px_w = self.view_width / cols as f32;
        let px_h = 1.0 / rows as f32;
        let left = -0.5 * self.view_width;
        let x_at = |i: usize| left + (i as f32 + 0.5) * px_w;
        let y_at = |j: usize| 1.0 - (j as f32 + 0.5) * px_h;

        for k in &self.kernels {
            let Some((i0, i1)) = span(
                (k.x - k.reach_x - left) / px_w,
                (k.x + k.reach_x - left) / px_w,
                cols,
            ) else {
                continue;
            };
            let Some((j0, j1)) = span(
                (1.0 - k.y - k.reach_y) / px_h,
                (1.0 - k.y + k.reach_y) / px_h,
                rows,
            ) else {
                continue;
            };
            for j in j0..=j1 {
                let dy = y_at(j) - k.y;
                let qy = dy * dy * k.inv_y2;
                if qy >= 1.0 {
                    continue;
                }
                let row = &mut out[j * cols..(j + 1) * cols];
                for (i, s) in row.iter_mut().enumerate().take(i1 + 1).skip(i0) {
                    let dx = x_at(i) - k.x;
                    add_kernel(s, k, qy + dx * dx * k.inv_x2);
                }
            }
        }

        // Pool: only the rows its surface can reach.
        let top = (self.pool_level + 1.5 * super::POOL_WAVE) as f32 + POOL_BAND;
        if let Some((j0, _)) = span((1.0 - top) / px_h, rows as f32, rows) {
            for i in 0..cols {
                let x = x_at(i);
                let surface = pool_surface(self.pool_level, f64::from(x), self.time) as f32;
                for j in j0..rows {
                    let y = y_at(j);
                    if self.in_container(f64::from(x), f64::from(y)) {
                        add_pool(&mut out[j * cols + i], surface - y);
                    }
                }
            }
        }

        for (j, row) in out.chunks_exact_mut(cols).enumerate() {
            let liquid = ambient_temp(f64::from(y_at(j))) as f32;
            for s in row {
                finish(s, liquid);
            }
        }
    }

    fn in_container(&self, x: f64, y: f64) -> bool {
        x.abs() <= 0.5 * self.wall_width * self.shape.width_fraction(y)
    }
}

/// Pixel-index range `[lo, hi]` (continuous pixel coords, centres at
/// `n + ½`) clipped to `0..n`, or `None` if it misses.
fn span(lo: f32, hi: f32, n: usize) -> Option<(usize, usize)> {
    let first = (lo - 0.5).ceil().max(0.0);
    let last = (hi - 0.5).floor().min(n as f32 - 1.0);
    (first <= last).then_some((first as usize, last as usize))
}

#[inline]
fn add_kernel(s: &mut Sample, k: &Kernel, q2: f32) {
    if q2 < 1.0 {
        let falloff = 1.0 - q2;
        let w = PEAK * falloff * falloff;
        s.density += w;
        s.temp += w * k.temp; // weighted sum until `finish`
    }
}

/// `depth` = how far below the pool surface (negative above it).
#[inline]
fn add_pool(s: &mut Sample, depth: f32) {
    let t = (depth / POOL_BAND * 0.5 + 0.5).clamp(0.0, 1.0);
    let w = t * t * (3.0 - 2.0 * t);
    s.density += w;
    s.temp += w * super::POOL_TEMP as f32;
}

#[inline]
fn finish(s: &mut Sample, liquid: f32) {
    s.temp = (s.temp + LIQUID_WEIGHT * liquid) / (s.density + LIQUID_WEIGHT);
}
