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
//! with a neck instead of just overlapping. The pool adds a soft-surfaced
//! layer along the base that follows its mounds.
//!
//! A blob is drawn as a main bump plus two or three smaller lobes whose
//! offsets slowly orbit and breathe (fixed by the blob's id, so it costs no
//! sim state), so outlines are lumpy and keep changing while the union
//! still covers about the blob's area. Lobes need resolution to read as
//! lumps rather than noise: [`Field::fill`] fades them out (to one round
//! bump) for blobs only a few sample pixels across, and draws the pool at
//! least [`MIN_POOL_PIXELS`] deep. Moving blobs are drawn as teardrops:
//! the half of each ellipse ahead of the motion is shortened and the half
//! behind lengthened by the same amount (the blob's eased `taper`), which
//! keeps the area, so a rising blob trails a tail. Buds and melting blobs get a skirt joining
//! them to the pool: a broad bulge that draws in to a neck as a bud lets
//! go. The pool is coloured hot where it is deep and cooler at its skin.
//! The optional top layer is a thin soft-surfaced band of cool wax under
//! the top edge, drawn at least [`MIN_CAP_PIXELS`] deep and at most
//! [`MAX_CAP_PIXELS`] (scaled by how far it has eased in); what hangs from
//! it or melts into it gets a skirt reaching up to it.
//!
//! Nothing snaps from one frame to the next: a merge or split crossfades
//! from the blobs it replaced (the world's ghosts) to the new ones, skirts
//! grow in and fade out, a blob only a little over `MELTED_RADIUS` fades
//! with its size, and lobe detail follows the grid the lamp is shown at,
//! not the one it is sampled at.

use super::blob::End;
use super::rng::hash;
use super::{
    Blob, CAP_LUMP, CAP_TEMP, LumpShape, MELTED_RADIUS, World, ambient_temp, cap_underside,
    pool_ceiling, pool_surface,
};

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
/// Lobe detail by blob radius in sample pixels: none (one round bump)
/// below the first, full from the second.
const LOBE_PIXELS: (f64, f64) = (2.5, 5.0);
/// The pool is drawn at least this many sample pixels deep, so a small
/// lamp's pool never thins to a stray row.
pub const MIN_POOL_PIXELS: f32 = 2.0;
/// The top layer is drawn this many sample pixels deep at least (so it
/// shows in a small lamp) and at most (so it stays a thin layer in a huge
/// one), and its soft surface is this thick (half, lamp heights).
pub const MIN_CAP_PIXELS: f64 = 1.5;
pub const MAX_CAP_PIXELS: f64 = 5.0;
/// Its soft surface is the pool's, so a blob nearing it fuses with it the
/// same way.
const CAP_BAND: f32 = POOL_BAND;
/// A blob fades out over its last `FADE_RADIUS × MELTED_RADIUS` of
/// radius before it melts away (and a bud fades in over its first).
const FADE_RADIUS: f64 = 1.0;
/// Most lobes a blob has.
const MAX_LOBES: usize = 3;
/// Skirt joining a bud to the pool: its half-width in blob radii when the
/// bud starts and when it lets go (`Blob::neck` 0 … 1).
const SKIRT: (f64, f64) = (0.9, 0.35);
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
struct Kernel {
    x: f32,
    y: f32,
    /// `1 / (SUPPORT · half-extent)²` across, and above / below the centre.
    inv_x2: f32,
    inv_up2: f32,
    inv_down2: f32,
    /// Support half-extents (bounding box for culling).
    reach_x: f32,
    reach_up: f32,
    reach_down: f32,
    /// Density at the centre (lobes fade by lowering it).
    peak: f32,
    temp: f32,
}

/// One blob's pose for this frame, before it is turned into kernels for a
/// particular sample resolution.
#[derive(Debug, Clone, Copy)]
pub(super) struct BlobSnap {
    x: f64,
    pub(super) y: f64,
    radius: f64,
    stretch: f64,
    taper: f64,
    temp: f32,
    /// How fully it is drawn, 0 … 1 (kernel strength).
    weight: f64,
    lobes: [Lobe; MAX_LOBES],
    lobe_count: usize,
    /// Joins a bud or melting blob to the pool.
    skirt: Option<Kernel>,
}

/// A frame's snapshot of the wax, ready to sample. Keep one around and call
/// [`Field::prepare`] each frame: it reuses its buffers, so steady-state
/// sampling never allocates.
#[derive(Debug, Default)]
pub struct Field {
    pub(super) blobs: Vec<BlobSnap>,
    view_width: f32,
    wall_width: f64,
    pool_level: f64,
    /// Width of the container's floor, under the pool.
    floor: f64,
    time: f64,
    /// How far the top layer is shown (0 … 1, eased), its mean depth,
    /// and its bulges.
    cap: f64,
    cap_depth: f64,
    lumps: Vec<LumpShape>,
}

impl Field {
    /// Snapshot `world`, interpolated `alpha` (0..=1) of the way from the
    /// previous step to the current one (`FixedStep::alpha`).
    pub fn prepare(&mut self, world: &World, alpha: f64) {
        let alpha = alpha.clamp(0.0, 1.0);
        self.blobs.clear();
        let time = world.time - (1.0 - alpha) * world.last_dt;
        let pool_level =
            world.prev_pool_level + (world.pool_level() - world.prev_pool_level) * alpha;
        let floor = world.bottom_width();
        let cap_on = world.prev_cap_on + (world.cap_on - world.prev_cap_on) * alpha;
        let cap = smooth(cap_on);
        let cap_depth =
            world.prev_cap_depth + (world.cap_mean_depth() - world.prev_cap_depth) * alpha;
        let mut lumps = std::mem::take(&mut self.lumps);
        lumps.clear();
        lumps.extend(world.lumps.iter().map(|l| l.prev.lerp(l.shape, alpha)));
        let snap = |blob: &Blob| {
            let pose = blob.prev.lerp(blob.pose(), alpha);
            let taper = pose.taper;
            let temp = blob.temp as f32;
            let r = pose.radius;
            // A bud just out of the pool, or a melting blob almost gone,
            // fades with its size rather than popping in or out.
            // Fades ease in and out, so the outline never starts or stops
            // moving with a jerk.
            let size = smooth((r - MELTED_RADIUS) / (FADE_RADIUS * MELTED_RADIUS));
            let weight = smooth(pose.weight) * size;
            // Attached to the pool: a skirt of wax joins the two, a broad
            // bulge while a bud swells that draws in to a neck as it lets
            // go (and fades as it rises away).
            let grown = pose.neck;
            // Hanging from (or melting into) the top layer: the same,
            // upside down.
            let skirt = (pose.attach > 0.0).then(|| {
                let width = r * (SKIRT.0 + (SKIRT.1 - SKIRT.0) * grown);
                let (height, y) = if blob.end == End::Top {
                    let under = cap_underside(cap * cap_depth, pose.x, time)
                        - cap * lump_depth(&lumps, pose.x);
                    let top = pose.y + r * pose.stretch;
                    let height = (0.5 * (under - top) + 0.5 * width).max(0.6 * width);
                    (height, under.min(0.5 * (under + top)))
                } else {
                    let surface = pool_surface(pool_level, pose.x, floor, time);
                    let bottom = pose.y - r * pose.stretch;
                    let height = (0.5 * (bottom - surface) + 0.5 * width).max(0.6 * width);
                    (height, surface.max(0.5 * (surface + bottom)))
                };
                let x = pose.x;
                let radius = (width * height).sqrt();
                let weight = weight * smooth(pose.attach);
                Kernel::new(x, y, radius, height / radius, 0.0, weight, temp)
            });
            let mut snap = BlobSnap {
                x: pose.x,
                y: pose.y,
                radius: r,
                stretch: pose.stretch,
                taper,
                temp,
                weight,
                lobes: [Lobe::default(); MAX_LOBES],
                lobe_count: 0,
                skirt,
            };
            for (slot, lobe) in snap.lobes.iter_mut().zip(lobes(blob.id, time)) {
                *slot = lobe;
                snap.lobe_count += 1;
            }
            snap
        };
        let ghosts = world.ghosts.iter().map(|g| &g.blob);
        self.blobs
            .extend(world.blobs.iter().chain(ghosts).map(snap));
        self.lumps = lumps;
        self.blobs.retain(|b| b.weight > 0.0);
        self.view_width = world.view_width as f32;
        self.wall_width = world.wall_width;
        self.pool_level = pool_level;
        self.time = time;
        self.floor = floor;
        self.cap = cap;
        self.cap_depth = cap_depth;
    }

    /// Viewport width in world units (the lamp's visual aspect).
    #[cfg(test)]
    pub fn aspect(&self) -> f32 {
        self.view_width
    }

    /// Sample one point (`u`, `v` normalised, `v` down) at full detail.
    #[cfg(test)]
    pub fn sample(&self, u: f32, v: f32) -> Sample {
        self.sample_at(u, v, Pixel::default())
    }

    /// Sample one point as [`Field::fill`] would on a `cols × rows` grid.
    #[cfg(test)]
    pub fn sample_on_grid(&self, u: f32, v: f32, cols: usize, rows: usize) -> Sample {
        self.sample_at(u, v, self.pixel(cols, rows))
    }

    #[cfg(test)]
    fn sample_at(&self, u: f32, v: f32, px: Pixel) -> Sample {
        let x = (u - 0.5) * self.view_width;
        let y = 1.0 - v;
        let mut acc = Sample::default();
        self.for_each_kernel(px, |k| {
            let (dx, dy) = (x - k.x, y - k.y);
            if dx.abs() < k.reach_x && -k.reach_down < dy && dy < k.reach_up {
                add_kernel(&mut acc, k, dx * dx * k.inv_x2 + k.qy(dy));
            }
        });
        if self.in_container(f64::from(x)) {
            add_pool(
                &mut acc,
                self.pool_surface_at(x, px, self.pool_lift(px)) - y,
            );
            if self.cap > 0.0 {
                let (under, temp) = self.cap_at(self.cap_drawn(px), f64::from(x));
                add_cap(&mut acc, y - under as f32, self.cap as f32, temp as f32);
            }
        }
        finish(&mut acc, ambient_temp(f64::from(y)) as f32);
        acc
    }

    /// Sample pixel size for a `cols × rows` grid over the viewport.
    fn pixel(&self, cols: usize, rows: usize) -> Pixel {
        let (w, h) = (self.view_width / cols as f32, 1.0 / rows as f32);
        Pixel {
            size: f64::from(w.max(h)),
            height: h,
        }
    }

    /// How far the pool is drawn above its true surface on pixels `px`
    /// tall: enough to bring its mean level to [`MIN_POOL_PIXELS`], so a
    /// small lamp's pool keeps its mounds on a base at least that deep.
    fn pool_lift(&self, px: Pixel) -> f32 {
        (MIN_POOL_PIXELS * px.height - self.pool_level as f32).max(0.0)
    }

    /// The top layer's drawn mean depth on pixels `px` tall: its own,
    /// kept between [`MIN_CAP_PIXELS`] and [`MAX_CAP_PIXELS`] deep, times
    /// how far it is shown.
    fn cap_drawn(&self, px: Pixel) -> f64 {
        let h = f64::from(px.height);
        self.cap * self.cap_depth.clamp(MIN_CAP_PIXELS * h, MAX_CAP_PIXELS * h)
    }

    /// The top layer's drawn underside at `x`, `depth` deep on average,
    /// bulges and all, and its temperature there: the layer's own, warmer
    /// where wax has just melted in.
    #[inline]
    fn cap_at(&self, depth: f64, x: f64) -> (f64, f64) {
        let plain = 1.0 - cap_underside(depth, x, self.time);
        let (mut bulge, mut heat) = (0.0, 0.0);
        for lump in &self.lumps {
            let d = self.cap * lump.depth(x);
            bulge += d;
            heat += d * lump.temp;
        }
        let total = plain + bulge;
        let temp = if total > 0.0 {
            (plain * CAP_TEMP + heat) / total
        } else {
            CAP_TEMP
        };
        (1.0 - total, temp)
    }

    /// The pool's drawn surface at `x`: lifted by `lift` (see
    /// [`Field::pool_lift`]) and never less than [`MIN_POOL_PIXELS`] deep.
    #[inline]
    fn pool_surface_at(&self, x: f32, px: Pixel, lift: f32) -> f32 {
        let surface = pool_surface(self.pool_level, f64::from(x), self.floor, self.time) as f32;
        (surface + lift).max(MIN_POOL_PIXELS * px.height)
    }

    /// Every kernel for pixels `px` across: per blob a main bump, its lobes
    /// faded in by how many pixels the blob spans, and any skirt.
    fn for_each_kernel(&self, px: Pixel, mut f: impl FnMut(&Kernel)) {
        for b in &self.blobs {
            let detail = lobe_detail(b.radius / px.size);
            let r = b.radius;
            let kernel = |x: f64, y: f64, radius: f64, weight: f64| {
                Kernel::new(x, y, radius, b.stretch, b.taper, b.weight * weight, b.temp)
            };
            // At full detail the main bump shrinks to share the blob with
            // its lobes; with none it is the whole, round blob.
            let main = 1.0 + (LOBE_SCALE * MAIN_LOBE - 1.0) * detail;
            f(&kernel(b.x, b.y, main * r, 1.0));
            if detail > 0.0 {
                for lobe in &b.lobes[..b.lobe_count] {
                    let (dx, dy) = (
                        detail * lobe.dx * r / b.stretch,
                        detail * lobe.dy * r * b.stretch,
                    );
                    f(&kernel(
                        b.x + dx,
                        b.y + dy,
                        LOBE_SCALE * lobe.size * r,
                        detail,
                    ));
                }
            }
            if let Some(skirt) = &b.skirt {
                f(skirt);
            }
        }
    }

    /// Fill `out` (row-major, `cols × rows`) with samples at pixel centres:
    /// pixel `(i, j)` is `sample((i + ½) / cols, (j + ½) / rows)`. Cost
    /// scales with the wax area on screen, not blobs × pixels.
    pub fn fill(&self, out: &mut [Sample], cols: usize, rows: usize) {
        self.fill_detailed(out, cols, rows, (cols, rows));
    }

    /// [`Field::fill`] for a grid that will be shown at `detail` (columns,
    /// rows) pixels, e.g. a reduced grid upsampled to the full one: shapes
    /// (lobes, the pool's least depth) follow the shown grid, so the wax
    /// keeps its shape whatever grid samples it.
    pub fn fill_detailed(
        &self,
        out: &mut [Sample],
        cols: usize,
        rows: usize,
        detail: (usize, usize),
    ) {
        assert_eq!(out.len(), cols * rows, "fill: buffer is not cols × rows");
        out.fill(Sample::default());
        if cols == 0 || rows == 0 {
            return;
        }
        let px = self.pixel(detail.0.max(1), detail.1.max(1));
        let px_w = self.view_width / cols as f32;
        let px_h = 1.0 / rows as f32;
        let left = -0.5 * self.view_width;
        let x_at = |i: usize| left + (i as f32 + 0.5) * px_w;
        let y_at = |j: usize| 1.0 - (j as f32 + 0.5) * px_h;

        self.for_each_kernel(px, |k| {
            let Some((i0, i1)) = span(
                (k.x - k.reach_x - left) / px_w,
                (k.x + k.reach_x - left) / px_w,
                cols,
            ) else {
                return;
            };
            let Some((j0, j1)) = span(
                (1.0 - k.y - k.reach_up) / px_h,
                (1.0 - k.y + k.reach_down) / px_h,
                rows,
            ) else {
                return;
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
        });

        // Pool: only the rows its surface can reach.
        let lift = self.pool_lift(px);
        let ceiling =
            (pool_ceiling(self.pool_level) as f32 + lift).max(MIN_POOL_PIXELS * px.height);
        let top = ceiling + POOL_BAND;
        if let Some((j0, _)) = span((1.0 - top) / px_h, rows as f32, rows) {
            for i in 0..cols {
                let x = x_at(i);
                if !self.in_container(f64::from(x)) {
                    continue;
                }
                let surface = self.pool_surface_at(x, px, lift);
                for j in j0..rows {
                    add_pool(&mut out[j * cols + i], surface - y_at(j));
                }
            }
        }

        if self.cap > 0.0 {
            self.fill_cap(out, (cols, rows), px, (left, px_w, px_h));
        }

        for (j, row) in out.chunks_exact_mut(cols).enumerate() {
            let liquid = ambient_temp(f64::from(y_at(j))) as f32;
            for s in row {
                finish(s, liquid);
            }
        }
    }

    /// The top layer's part of [`Field::fill_detailed`]: only the rows its
    /// underside can reach. Kept out of line, so the fill loop is the same
    /// code with no top layer.
    #[inline(never)]
    fn fill_cap(
        &self,
        out: &mut [Sample],
        (cols, rows): (usize, usize),
        px: Pixel,
        (left, px_w, px_h): (f32, f32, f32),
    ) {
        let depth = self.cap_drawn(px);
        let shown = self.cap as f32;
        let bulge: f64 = self.lumps.iter().map(|l| l.peak()).sum();
        let reach = (depth * (1.0 + CAP_LUMP) + self.cap * bulge) as f32 + CAP_BAND;
        let Some((_, j1)) = span(0.0, reach / px_h, rows) else {
            return;
        };
        for i in 0..cols {
            let x = left + (i as f32 + 0.5) * px_w;
            if !self.in_container(f64::from(x)) {
                continue;
            }
            let (under, temp) = self.cap_at(depth, f64::from(x));
            let (under, temp) = (under as f32, temp as f32);
            for j in 0..=j1 {
                let y = 1.0 - (j as f32 + 0.5) * px_h;
                add_cap(&mut out[j * cols + i], y - under, shown, temp);
            }
        }
    }

    /// Whether `x` is between the walls (which lag the view on a resize).
    fn in_container(&self, x: f64) -> bool {
        x.abs() <= 0.5 * self.wall_width
    }
}

impl Kernel {
    /// A bump `radius` across (equal-area), `stretch`ed and `taper`ed, its
    /// centre `weight` × full strength.
    fn new(x: f64, y: f64, radius: f64, stretch: f64, taper: f64, weight: f64, temp: f32) -> Self {
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
            peak: PEAK * weight as f32,
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
        let w = k.peak * falloff * falloff;
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

/// How far the top layer's bulges hang below it at `x`.
#[inline]
fn lump_depth(lumps: &[LumpShape], x: f64) -> f64 {
    lumps.iter().map(|l| l.depth(x)).sum()
}

/// `height` = how far above the top layer's underside (negative below);
/// `shown` = how far it has eased in (its soft surface fades with it);
/// `temp` its temperature there.
#[inline]
fn add_cap(s: &mut Sample, height: f32, shown: f32, temp: f32) {
    let w = shown * smooth01(height / CAP_BAND * 0.5 + 0.5);
    s.density += w;
    s.temp += w * temp;
}

/// [`smooth01`] in `f64`.
pub(super) fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn smooth01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of its lobes a blob `pixels` sample pixels in radius shows,
/// 0 (one round bump) … 1.
fn lobe_detail(pixels: f64) -> f64 {
    smooth((pixels - LOBE_PIXELS.0) / (LOBE_PIXELS.1 - LOBE_PIXELS.0))
}

/// Sample pixel size, in world units: across (the larger side, for lobe
/// detail) and tall (for the pool's depth in rows).
/// The default (zero) is an infinitely fine grid: full detail.
#[derive(Debug, Clone, Copy, Default)]
struct Pixel {
    size: f64,
    height: f32,
}

/// One satellite of a blob's main bump, in units of the blob's radius
/// (before stretch).
#[derive(Debug, Clone, Copy, Default)]
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

#[inline]
fn finish(s: &mut Sample, liquid: f32) {
    s.temp = (s.temp + LIQUID_WEIGHT * liquid) / (s.density + LIQUID_WEIGHT);
}
