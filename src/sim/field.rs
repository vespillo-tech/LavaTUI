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
//!
//! A blob is drawn as a main bump plus two or three smaller lobes whose
//! offsets slowly orbit and breathe (fixed by the blob's id, so it costs no
//! sim state), so outlines are lumpy and keep changing while the union
//! still covers about the blob's area. Moving blobs are drawn as teardrops:
//! the half of each ellipse ahead of the motion is shortened and the half
//! behind lengthened by the same amount (`TAPER`), which keeps the area, so
//! a rising blob trails a tail. Buds and melting blobs get a skirt joining
//! them to the pool: a broad bulge that draws in to a neck as a bud lets
//! go. The pool is coloured hot where it is deep and cooler at its skin.

use super::{Phase, Shape, World, ambient_temp, pool_surface};

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
/// The pool's colour: a cooler skin at the surface, glowing up to the
/// pool's own temperature this deep (lamp heights).
const POOL_SKIN_TEMP: f32 = 0.66;
const POOL_GLOW_DEPTH: f32 = 0.12;
/// Blob shape: a main bump plus satellite lobes (radius multiples), all
/// scaled by `LOBE_SCALE` so the union covers about the blob's own area.
const MAIN_LOBE: f64 = 0.8;
const LOBE_SIZE: (f64, f64) = (0.55, 0.75);
const LOBE_DIST: (f64, f64) = (0.28, 0.45);
const LOBE_SCALE: f64 = 0.955;
/// Skirt joining a bud to the pool: its half-width in blob radii when the
/// bud starts and when it lets go, and how far grown a melting blob's is.
const SKIRT: (f64, f64) = (0.9, 0.35);
const MELT_SKIRT: f64 = 0.3;
/// Teardrop taper per unit of vertical speed (lamp heights / s), and its
/// cap: a hot blob at full speed is about 0.4 / 1.6 front / back.
const TAPER: f64 = 10.0;
const MAX_TAPER: f64 = 0.5;
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
    /// `1 / (SUPPORT · half-extent)²` across, and above / below the centre.
    inv_x2: f32,
    inv_up2: f32,
    inv_down2: f32,
    /// Support half-extents (bounding box for culling).
    reach_x: f32,
    reach_up: f32,
    reach_down: f32,
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
    /// Width of the container's floor, under the pool.
    floor: f64,
    time: f64,
}

impl Field {
    /// Snapshot `world`, interpolated `alpha` (0..=1) of the way from the
    /// previous step to the current one (`FixedStep::alpha`).
    pub fn prepare(&mut self, world: &World, alpha: f64) {
        let alpha = alpha.clamp(0.0, 1.0);
        self.kernels.clear();
        let time = world.time - (1.0 - alpha) * world.last_dt;
        let pool_level =
            world.prev_pool_level + (world.pool_level() - world.prev_pool_level) * alpha;
        let floor = world.bottom_width();
        for blob in &world.blobs {
            let pose = blob.prev.lerp(blob.pose(), alpha);
            // Rising: the tail hangs below; sinking: it trails above.
            let taper = (TAPER * blob.vy).clamp(-MAX_TAPER, MAX_TAPER);
            let temp = blob.temp as f32;
            let r = pose.radius;
            let kernel =
                |x: f64, y: f64, radius: f64| Kernel::new(x, y, radius, pose.stretch, taper, temp);
            self.kernels
                .push(kernel(pose.x, pose.y, LOBE_SCALE * MAIN_LOBE * r));
            for lobe in lobes(blob.id, time) {
                let (dx, dy) = (lobe.dx * r / pose.stretch, lobe.dy * r * pose.stretch);
                self.kernels
                    .push(kernel(pose.x + dx, pose.y + dy, LOBE_SCALE * lobe.size * r));
            }
            // Attached to the pool: a skirt of wax joins the two, a broad
            // bulge while a bud swells that draws in to a neck as it lets go.
            let grown = match blob.phase {
                Phase::Budding { target } => (r / target).min(1.0),
                Phase::Melting => MELT_SKIRT,
                Phase::Free => continue,
            };
            let surface = pool_surface(pool_level, pose.x, floor, time);
            let bottom = pose.y - r * pose.stretch;
            let width = r * (SKIRT.0 + (SKIRT.1 - SKIRT.0) * grown);
            let height = (0.5 * (bottom - surface) + 0.5 * width).max(0.6 * width);
            let (x, y) = (pose.x, surface.max(0.5 * (surface + bottom)));
            let radius = (width * height).sqrt();
            let skirt = Kernel::new(x, y, radius, height / radius, 0.0, temp);
            self.kernels.push(skirt);
        }
        self.shape = world.shape;
        self.view_width = world.view_width as f32;
        self.wall_width = world.wall_width;
        self.pool_level = pool_level;
        self.time = time;
        self.floor = floor;
    }

    /// Viewport width in world units (the lamp's visual aspect).
    #[cfg(test)]
    pub fn aspect(&self) -> f32 {
        self.view_width
    }

    /// Container shape, for masking the glass.
    pub fn shape(&self) -> Shape {
        self.shape
    }

    /// Sample one point (`u`, `v` normalised, `v` down).
    #[cfg(test)]
    pub fn sample(&self, u: f32, v: f32) -> Sample {
        let x = (u - 0.5) * self.view_width;
        let y = 1.0 - v;
        let mut acc = Sample::default();
        for k in &self.kernels {
            let (dx, dy) = (x - k.x, y - k.y);
            if dx.abs() < k.reach_x && -k.reach_down < dy && dy < k.reach_up {
                add_kernel(&mut acc, k, dx * dx * k.inv_x2 + k.qy(dy));
            }
        }
        if self.in_container(f64::from(x), f64::from(y)) {
            let surface = pool_surface(self.pool_level, f64::from(x), self.floor, self.time) as f32;
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
                (1.0 - k.y - k.reach_up) / px_h,
                (1.0 - k.y + k.reach_down) / px_h,
                rows,
            ) else {
                continue;
            };
            for j in j0..=j1 {
                let qy = k.qy(y_at(j) - k.y);
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
        let top = ((self.pool_level + 1.3 * super::POOL_MOUND * super::MOUND_DEPTH)
            + 1.6 * super::POOL_WAVE) as f32
            + POOL_BAND;
        if let Some((j0, _)) = span((1.0 - top) / px_h, rows as f32, rows) {
            for i in 0..cols {
                let x = x_at(i);
                let surface =
                    pool_surface(self.pool_level, f64::from(x), self.floor, self.time) as f32;
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

impl Kernel {
    /// A bump `radius` across (equal-area), `stretch`ed and `taper`ed.
    fn new(x: f64, y: f64, radius: f64, stretch: f64, taper: f64, temp: f32) -> Self {
        let reach = f64::from(SUPPORT) * radius;
        let (reach_x, reach_y) = (reach / stretch, reach * stretch);
        let (reach_up, reach_down) = (reach_y * (1.0 - taper), reach_y * (1.0 + taper));
        Kernel {
            x: x as f32,
            y: y as f32,
            inv_x2: (1.0 / (reach_x * reach_x)) as f32,
            inv_up2: (1.0 / (reach_up * reach_up)) as f32,
            inv_down2: (1.0 / (reach_down * reach_down)) as f32,
            reach_x: reach_x as f32,
            reach_up: reach_up as f32,
            reach_down: reach_down as f32,
            temp,
        }
    }

    /// Vertical part of the squared elliptical distance at offset `dy`.
    #[inline]
    fn qy(&self, dy: f32) -> f32 {
        dy * dy
            * if dy > 0.0 {
                self.inv_up2
            } else {
                self.inv_down2
            }
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

/// `depth` = how far below the pool surface (negative above it). The pool
/// glows hot where it is deep, over the heater, and shows a cooler skin.
#[inline]
fn add_pool(s: &mut Sample, depth: f32) {
    let w = smooth01(depth / POOL_BAND * 0.5 + 0.5);
    let hot = super::POOL_TEMP as f32;
    let temp = POOL_SKIN_TEMP + (hot - POOL_SKIN_TEMP) * smooth01(depth / POOL_GLOW_DEPTH);
    s.density += w;
    s.temp += w * temp;
}

#[inline]
fn smooth01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One satellite of a blob's main bump, in units of the blob's radius
/// (before stretch).
struct Lobe {
    dx: f64,
    dy: f64,
    size: f64,
}

/// A blob's lobes at `time`: two or three smaller bumps around the main one
/// whose offsets slowly orbit and breathe, so the outline is lumpy and
/// keeps changing. Fixed by the blob's id: no sim state, deterministic.
fn lobes(id: u64, time: f64) -> impl Iterator<Item = Lobe> {
    let mut h = hash(id);
    let count = 2 + (h % 2) as usize;
    (0..count).map(move |_| {
        h = hash(h);
        let unit = |bits: u32| f64::from((h >> bits) as u16) / 65536.0;
        let spin = (0.03 + 0.07 * unit(0)) * if h & (1 << 63) != 0 { 1.0 } else { -1.0 };
        let angle = 2.0 * std::f64::consts::PI * unit(16) + spin * time;
        let breathe = 1.0 + 0.2 * (time * (0.1 + 0.15 * unit(32)) + 6.0 * unit(48)).sin();
        let dist = LOBE_DIST.0 + (LOBE_DIST.1 - LOBE_DIST.0) * unit(40);
        Lobe {
            dx: angle.cos() * dist * breathe,
            dy: angle.sin() * dist * breathe,
            size: LOBE_SIZE.0 + (LOBE_SIZE.1 - LOBE_SIZE.0) * unit(24),
        }
    })
}

/// SplitMix64 finaliser.
fn hash(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
fn finish(s: &mut Sample, liquid: f32) {
    s.temp = (s.temp + LIQUID_WEIGHT * liquid) / (s.density + LIQUID_WEIGHT);
}
