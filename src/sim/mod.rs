//! Wax simulation: a pure, seeded, fixed-step model of a lava lamp. No
//! terminal code.
//!
//! # World units
//!
//! The lamp is always 1.0 tall: `y` runs from 0 (base, the heater) to 1
//! (top). Width is the lamp's *visual* aspect (on-screen width ÷ height),
//! centred on `x = 0`, so a blob that is round in world units is round on
//! screen whatever the window size. In the tank (bleed) a terminal resize
//! only changes the view width; the walls then ease to it over ~250 ms and
//! push blobs along, so nothing teleports (docs/design.md §2.2). The bottle
//! (glass) has a fixed aspect: resizing it only changes the sampling.
//! Switching between the two keeps every blob that fits where it is (see
//! [`World::set_shape`]).
//!
//! # Model
//!
//! - A thin **pool** of molten wax sits on the heater. It buds: a wide bulge
//!   swells out of it, necks off and rises. The deeper the pool, the sooner
//!   it buds, so wax never piles up into a slab.
//! - A few big blobs of quite different sizes, not many equal ones. Moving
//!   blobs stretch along their motion, and the field draws them as
//!   teardrops trailing a tail (see `field.rs`).
//! - Free blobs exchange heat with the liquid, which is warm at the base and
//!   cool at the top (smaller blobs change temperature faster). Buoyancy is
//!   proportional to `temp - NEUTRAL_TEMP`; strong, implicit viscous drag
//!   makes the motion slow and heavy. Drag is relative to a slow convection
//!   flow, which adds gentle swirl.
//! - Similar-temperature blobs attract weakly and merge when they overlap
//!   enough; dissimilar ones squeeze past each other. Big, hot, rising blobs
//!   occasionally split in two.
//! - Cooled blobs sink, settle on the pool and melt back into it.
//! - Total wax area is conserved by every step (pool + blobs). Only a width
//!   change adjusts it, slowly, through the pool.
//!
//! User controls (heat, reseed, heat pulse, speed) live in `controls.rs`.
//!
//! Rendering goes through [`Field`]: `prepare` it from the world once per
//! frame (with the fixed-step `alpha` for smooth interpolation), then sample.

mod blob;
mod controls;
mod field;
mod rng;

use std::f64::consts::PI;

use crate::silhouette::{self, BOTTLE_ASPECT};
pub use blob::{Blob, Phase};
pub use controls::SimSpeed;
pub use controls::{DEFAULT_HEAT, HEAT_LEVELS};
use controls::{Pulse, Reseed};
pub use field::{Field, SURFACE, Sample};
use rng::Rng;

/// Buoyancy is zero at this temperature: hotter rises, colder sinks.
pub const NEUTRAL_TEMP: f64 = 0.5;
/// Liquid temperature at the base and at the top. Both are below neutral,
/// so only wax fresh off the heater rises; everything eventually sinks.
const AMBIENT_BOTTOM: f64 = 0.42;
const AMBIENT_TOP: f64 = 0.18;
/// Temperature of the pool on the heater, and of buds leaving it.
const POOL_TEMP: f64 = 0.92;
/// The span wax temperatures show across, cool to hot: from a little
/// above the top liquid (wax that has cooled there) to a little below
/// the pool (fresh off the heater). Renderers map it onto the wax colours.
pub const WAX_TEMP: (f32, f32) = (0.25, 0.9);

/// Upward acceleration per unit of `temp - NEUTRAL_TEMP` for a
/// reference-size blob (world units / s²).
const BUOYANCY: f64 = 0.2;
/// Viscous drag rate (1/s). Terminal velocity = buoyancy / drag, so the
/// hottest blobs top out near 0.06 lamp-heights per second.
const DRAG: f64 = 1.5;
/// Heat exchange rate (1/s) with the liquid for a reference-size blob.
/// Exchange scales with surface / volume, i.e. `1 / radius`.
const COOL_RATE: f64 = 0.045;
const REF_RADIUS: f64 = 0.08;
/// Free blobs just above the pool are warmed by the heater.
const HEATER_BAND: f64 = 0.04;
const HEATER_RATE: f64 = 0.25;
/// Buds and melting blobs approach the pool temperature at this rate (1/s).
const POOL_HEAT_RATE: f64 = 0.6;

/// Wax as a fraction of the container's area.
const FILL: f64 = 0.26;
/// Pool depth (lamp heights) the world aims for, and the least it keeps.
const POOL_DEPTH: f64 = 0.045;
const MIN_POOL_DEPTH: f64 = 0.022;
/// Bud sizes, as a range of multiples of the typical radius: with merges
/// and splits on top, the lamp shows a 3:1 spread or more.
const BUD_SIZE: (f64, f64) = (0.35, 1.5);
/// Largest bud, as a fraction of the largest blob.
const MAX_BUD: f64 = 0.95;
/// Pool surface ripple amplitude.
const POOL_WAVE: f64 = 0.006;
/// Seconds a bud takes to grow to full size.
const BUD_TIME: f64 = 6.0;
/// Seconds between bud attempts (random in range). A pool deeper than
/// `POOL_DEPTH` shortens the gap in proportion, and past `DEEP_POOL ×
/// POOL_DEPTH` it buds even when the lamp already has its blob count
/// (a cooler lamp lets more wax lie: see `heat_deep_pool`).
const SPAWN_GAP: (f64, f64) = (1.5, 5.0);
const DEEP_POOL: f64 = 1.5;
/// Melting blobs lose this fraction of their area per second.
const MELT_RATE: f64 = 0.5;
/// A melting blob smaller than this is gone.
const MELTED_RADIUS: f64 = 0.012;

/// Cohesion: similar blobs within `COHESION_RANGE × (r1 + r2)` attract.
const COHESION: f64 = 0.02;
const COHESION_RANGE: f64 = 1.6;
/// Overlapping blobs that may not merge push apart this hard.
const REPULSION: f64 = 0.4;
/// Merge when centres are closer than this fraction of `r1 + r2`.
const MERGE_DIST: f64 = 0.6;
/// Blobs whose temperatures differ by more than this don't merge.
const MERGE_TEMP_GAP: f64 = 0.2;
/// How elongated a merged blob starts, per unit of alignment.
const MERGE_STRETCH: f64 = 0.35;
/// Splits: blobs above `SPLIT_FRACTION × max radius`, hot and rising, split
/// at up to `SPLIT_RATE` per second (scaled by how far over they are). Hot
/// blobs never merge past that size, so split halves don't just re-fuse.
const SPLIT_FRACTION: f64 = 0.85;
const HOT: f64 = 0.6;
const SPLIT_RATE: f64 = 0.15;
const SPLIT_KICK: f64 = 0.008;
/// Split halves start this far apart (× parent half-height) and overlap,
/// so the field shows a neck that thins as they part.
const SPLIT_REACH: f64 = 0.55;
/// No merging/melting for this long after a split or detaching.
const COOLDOWN: f64 = 3.0;

/// Largest blob radius (lamp heights), as a fraction of the narrowest width
/// (it must fit the bottle's neck), and of the widest (two must fit side by
/// side, or a narrow lamp jams).
const MAX_RADIUS: f64 = 0.16;
const MAX_RADIUS_OF_WIDTH: f64 = 0.48;
const MAX_RADIUS_OF_WIDEST: f64 = 0.25;
const MAX_BLOBS: usize = 40;

/// Wall spring stiffness (1/s²).
const WALL: f64 = 12.0;
/// Hard caps that keep the sim sane whatever happens.
const MAX_SPEED: f64 = 0.3;
const STRETCH_RANGE: (f64, f64) = (0.65, 1.9);
/// Stretch follows velocity: tall when moving vertically, wide when moving
/// sideways, relaxing at `STRETCH_RELAX` per second. A hot blob at full
/// speed (~0.06/s) aims for about 1.4.
const STRETCH_GAIN: f64 = 7.0;
const STRETCH_RELAX: f64 = 0.8;

/// Lateral meander acceleration amplitude, and frequency range (rad/s).
const WANDER: f64 = 0.008;
const WANDER_FREQ: (f64, f64) = (0.08, 0.25);
/// Background convection: peak liquid speed, and preferred cell width.
const FLOW: f64 = 0.012;
const FLOW_CELL: f64 = 0.8;

/// Wall easing time constant on resize (≈ 95 % in 250 ms).
const WALL_EASE: f64 = 0.08;
/// Pool area eases toward the volume target at this rate (1/s).
const POOL_EASE: f64 = 0.5;
/// Accepted lamp aspect range.
const ASPECT_RANGE: (f64, f64) = (0.05, 20.0);

/// Container shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    /// Straight walls; width follows the window ("bleed" frame).
    #[default]
    Tank,
    /// The glass lamp's bottle profile (docs/design.md §2.1): the world is
    /// the bottle's bounding box and the walls follow its curve.
    Bottle,
}

impl Shape {
    /// Container width at height `y` (0 base … 1 top), as a fraction of the
    /// world width. Renderers use this to mask the bottle.
    pub fn width_fraction(self, y: f64) -> f64 {
        match self {
            Shape::Tank => 1.0,
            Shape::Bottle => silhouette::bottle_width(y),
        }
    }

    /// World width for a lamp whose view is `aspect` wide: the bottle's is
    /// fixed, the tank's follows the view.
    fn world_width(self, aspect: f64) -> f64 {
        match self {
            Shape::Tank => aspect.clamp(ASPECT_RANGE.0, ASPECT_RANGE.1),
            Shape::Bottle => BOTTLE_ASPECT,
        }
    }

    /// Container area for a world `width` wide.
    fn area(self, width: f64) -> f64 {
        match self {
            Shape::Tank => width,
            Shape::Bottle => width * silhouette::bottle_area(),
        }
    }
}

/// Liquid temperature at height `y`.
pub fn ambient_temp(y: f64) -> f64 {
    AMBIENT_BOTTOM + (AMBIENT_TOP - AMBIENT_BOTTOM) * y.clamp(0.0, 1.0)
}

/// Pool surface height at `x`: the mean `level` plus a slow ripple.
fn pool_surface(level: f64, x: f64, time: f64) -> f64 {
    level + POOL_WAVE * ((x * 7.0 + time * 0.35).sin() + 0.5 * (x * 17.0 - time * 0.6).sin())
}

/// Event counters, for tests and a debug HUD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub budded: u64,
    pub merged: u64,
    pub split: u64,
    pub melted: u64,
}

#[derive(Debug)]
pub struct World {
    time: f64,
    /// Duration of the last step (for interpolating time).
    last_dt: f64,
    shape: Shape,
    /// Width of the viewport (what the renderer maps onto the screen).
    view_width: f64,
    /// Width of the walls; eases toward `view_width`.
    wall_width: f64,
    pool_area: f64,
    /// Pool level at the start of the last step, for interpolation.
    prev_pool_level: f64,
    /// Total wax the world aims to hold; changes only with the width.
    wax_target: f64,
    blobs: Vec<Blob>,
    /// Per-blob acceleration scratch, reused every step.
    accel: Vec<(f64, f64)>,
    rng: Rng,
    next_id: u64,
    spawn_timer: f64,
    stats: Stats,
    /// Chosen heat level, and the continuous level the sim uses (eases
    /// toward the chosen one).
    heat_target: u8,
    heat_level: f64,
    pulses: Vec<Pulse>,
    reseed: Option<Reseed>,
}

impl World {
    /// A fresh lamp `aspect` wide (visual width ÷ height; ignored for the
    /// bottle, see [`BOTTLE_ASPECT`]), seeded so the same seed always plays
    /// out the same way. Starts with some wax already
    /// afloat; run [`World::prewarm`] to skip the opening entirely.
    pub fn new(seed: u64, aspect: f64, shape: Shape) -> Self {
        let width = shape.world_width(aspect);
        let mut world = Self {
            time: 0.0,
            last_dt: 0.0,
            shape,
            view_width: width,
            wall_width: width,
            pool_area: 0.0,
            prev_pool_level: 0.0,
            wax_target: FILL * shape.area(width),
            blobs: Vec::with_capacity(MAX_BLOBS + 1),
            accel: Vec::with_capacity(MAX_BLOBS + 1),
            rng: Rng::new(seed),
            next_id: 0,
            spawn_timer: 0.0,
            stats: Stats::default(),
            heat_target: DEFAULT_HEAT,
            heat_level: f64::from(DEFAULT_HEAT),
            pulses: Vec::new(),
            reseed: None,
        };
        world.scatter_initial_blobs();
        world.prev_pool_level = world.pool_level();
        for blob in &mut world.blobs {
            blob.prev = blob.pose();
        }
        world
    }

    /// Run `steps` steps of `dt` so the first frame already looks alive.
    pub fn prewarm(&mut self, steps: u32, dt: f64) {
        for _ in 0..steps {
            self.step(dt);
        }
    }

    #[cfg(test)]
    pub fn blobs(&self) -> &[Blob] {
        &self.blobs
    }

    #[cfg(test)]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Total wax: pool plus blobs.
    pub fn wax_area(&self) -> f64 {
        self.pool_area + self.blobs.iter().map(Blob::area).sum::<f64>()
    }

    /// Mean height of the pool surface.
    pub fn pool_level(&self) -> f64 {
        self.pool_area / self.bottom_width()
    }

    /// Single field sample at the current state. Convenience for tests and
    /// one-offs; renderers should reuse a [`Field`].
    #[cfg(test)]
    pub fn sample(&self, u: f64, v: f64) -> Sample {
        let mut field = Field::default();
        field.prepare(self, 1.0);
        field.sample(u as f32, v as f32)
    }

    /// The lamp's on-screen aspect changed. The view follows at once; walls
    /// ease over ~250 ms, pushing blobs, and the pool slowly adjusts so wax
    /// stays at [`FILL`] of the container. A no-op for the bottle, whose
    /// world never resizes.
    pub fn set_aspect(&mut self, aspect: f64) {
        let width = self.shape.world_width(aspect);
        if (width - self.view_width).abs() > 1e-9 {
            self.view_width = width;
            self.wax_target = FILL * self.shape.area(width);
        }
    }

    /// Advance by exactly `dt` seconds (always the fixed step).
    pub fn step(&mut self, dt: f64) {
        self.last_dt = dt;
        self.prev_pool_level = self.pool_level();
        for blob in &mut self.blobs {
            blob.prev = blob.pose();
            blob.cooldown = (blob.cooldown - dt).max(0.0);
        }
        self.time += dt;

        self.update_controls(dt);
        self.ease_walls(dt);
        self.move_free(dt);
        self.move_attached(dt);
        self.exchange_heat(dt);
        self.merge();
        self.split(dt);
        self.spawn(dt);
        self.balance_pool(dt);
        self.contain();
    }

    // --- geometry --------------------------------------------------------

    fn half_width_at(&self, y: f64) -> f64 {
        0.5 * self.wall_width * self.shape.width_fraction(y)
    }

    fn bottom_width(&self) -> f64 {
        self.wall_width * self.shape.width_fraction(0.0)
    }

    fn max_radius(&self) -> f64 {
        let narrowest = self.wall_width * self.shape.width_fraction(1.0);
        MAX_RADIUS
            .min(MAX_RADIUS_OF_WIDTH * narrowest)
            .min(MAX_RADIUS_OF_WIDEST * self.wall_width)
    }

    /// Pool depth, in `POOL_DEPTH`s, past which the pool buds regardless
    /// of the blob count.
    fn deep_pool(&self) -> f64 {
        DEEP_POOL * self.heat_deep_pool()
    }

    /// How deep the pool is relative to [`POOL_DEPTH`] (which a cooler lamp
    /// lets grow), 1..=4: a deep pool buds this much more often and its buds
    /// grow this much faster.
    fn pool_surplus(&self) -> f64 {
        (self.pool_level() / (POOL_DEPTH * self.heat_deep_pool())).clamp(1.0, 4.0)
    }

    fn min_pool_area(&self) -> f64 {
        MIN_POOL_DEPTH * self.bottom_width()
    }

    /// How many blobs the world aims for (docs/design.md §2.4).
    /// Scales with heat, which eases, so the count changes gradually.
    fn target_blobs(&self) -> usize {
        let base = match self.shape {
            Shape::Tank => (3.5 * self.wall_width).clamp(3.0, 20.0),
            Shape::Bottle => 5.0,
        };
        ((base * self.heat_blobs()).round() as usize).clamp(2, MAX_BLOBS)
    }

    /// Typical blob radius: the wax not in a full-depth pool, shared out.
    fn typical_radius(&self) -> f64 {
        let afloat = self.wax_target - POOL_DEPTH * self.bottom_width();
        (afloat.max(0.0) / (self.target_blobs() as f64 * PI)).sqrt()
    }

    /// Liquid velocity: a slow, gently pulsing row of convection cells that
    /// never flows through the walls.
    fn flow(&self, x: f64, y: f64) -> (f64, f64) {
        let width = self.wall_width;
        let cells = (width / FLOW_CELL).round().max(1.0);
        let kx = PI * cells / width;
        let s = (x / width + 0.5) * PI * cells;
        let amp = FLOW * (0.75 + 0.25 * (self.time * 0.05).sin()) / PI;
        // Stream function ψ = amp · sin(πy) · sin(s); u = ∂ψ/∂y, v = −∂ψ/∂x.
        let y = y.clamp(0.0, 1.0);
        (
            amp * PI * (PI * y).cos() * s.sin(),
            -amp * kx * (PI * y).sin() * s.cos(),
        )
    }

    // --- step stages -------------------------------------------------------

    fn ease_walls(&mut self, dt: f64) {
        let gap = self.view_width - self.wall_width;
        self.wall_width = if gap.abs() < 1e-6 {
            self.view_width
        } else {
            self.wall_width + gap * (1.0 - (-dt / WALL_EASE).exp())
        };
    }

    fn move_free(&mut self, dt: f64) {
        let max_radius = self.max_radius();
        let blobs = &self.blobs;
        let accel = &mut self.accel;
        accel.clear();
        accel.resize(blobs.len(), (0.0, 0.0));

        for i in 0..blobs.len() {
            for j in i + 1..blobs.len() {
                let (a, b) = (&blobs[i], &blobs[j]);
                if a.phase != Phase::Free || b.phase != Phase::Free {
                    continue;
                }
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let dist = dx.hypot(dy).max(1e-9);
                let reach = a.radius + b.radius;
                if dist > COHESION_RANGE * reach {
                    continue;
                }
                let force = if can_merge(a, b, max_radius) {
                    COHESION * (1.0 - dist / (COHESION_RANGE * reach))
                } else if dist < reach {
                    -REPULSION * (reach - dist) / reach
                } else {
                    continue;
                };
                // Split the pull by mass: the smaller blob moves more.
                let (ma, mb) = (a.area(), b.area());
                let (ux, uy) = (dx / dist, dy / dist);
                let (fa, fb) = (force * mb / (ma + mb), force * ma / (ma + mb));
                accel[i].0 += ux * fa;
                accel[i].1 += uy * fa;
                accel[j].0 -= ux * fb;
                accel[j].1 -= uy * fb;
            }
        }

        let damp = 1.0 / (1.0 + DRAG * dt);
        let level = self.pool_level();
        let buoyancy = BUOYANCY * self.heat_buoyancy();
        for i in 0..self.blobs.len() {
            let blob = &self.blobs[i];
            if blob.phase != Phase::Free {
                continue;
            }
            let (mut ax, mut ay) = self.accel[i];
            let (hx, hy) = blob.half_extents();
            // Stokes-ish: bigger blobs rise and sink a little faster.
            let size = (blob.radius / REF_RADIUS).sqrt().clamp(0.6, 1.4);
            ay += buoyancy * size * (blob.temp - NEUTRAL_TEMP);
            ax += WANDER * (blob.wander_freq * self.time + blob.wander_phase).sin();

            let half = self.half_width_at(blob.y);
            ax += WALL * ((-half + hx - blob.x).max(0.0) - (blob.x + hx - half).max(0.0));
            ay -= WALL * (blob.y + hy - 1.0).max(0.0);

            // Implicit drag toward the liquid's own velocity.
            let (fx, fy) = self.flow(blob.x, blob.y);
            let blob = &mut self.blobs[i];
            blob.vx = (blob.vx + (ax + DRAG * fx) * dt) * damp;
            blob.vy = (blob.vy + (ay + DRAG * fy) * dt) * damp;
            blob.x += blob.vx * dt;
            blob.y += blob.vy * dt;

            let target = 1.0 + STRETCH_GAIN * (blob.vy.abs() - blob.vx.abs());
            relax(&mut blob.stretch, target, STRETCH_RELAX, dt);

            // Settled onto the pool while sinking: start melting in.
            let bottom = blob.y - blob.radius * blob.stretch;
            let surface = pool_surface(level, blob.x, self.time);
            if bottom < surface + 0.004 && blob.vy < 0.0 && blob.cooldown <= 0.0 {
                blob.phase = Phase::Melting;
            }
        }
    }

    /// Buds and melting blobs are attached to the pool and move with it.
    fn move_attached(&mut self, dt: f64) {
        let level = self.pool_level();
        let min_pool = self.min_pool_area();
        let (melt_rate, bud_time) = match self.reseed {
            Some(Reseed::Melting) => (controls::RESEED_MELT_RATE, BUD_TIME),
            Some(Reseed::Refill { .. }) => (MELT_RATE, BUD_TIME / controls::REFILL_BUD_SPEEDUP),
            None => (MELT_RATE, BUD_TIME / self.pool_surplus()),
        };
        for blob in &mut self.blobs {
            if blob.phase == Phase::Free {
                continue;
            }
            // Walls that moved in on an attached blob nudge it along the pool.
            let inside =
                (0.5 * self.wall_width * self.shape.width_fraction(blob.y) - blob.radius).max(0.0);
            let excess = blob.x.abs() - inside;
            if excess > 0.0 {
                blob.x -= blob.x.signum() * excess.min(MAX_SPEED * dt);
            }
            let surface = pool_surface(level, blob.x, self.time);
            let old_y = blob.y;
            match blob.phase {
                Phase::Free => unreachable!("skipped above"),
                Phase::Budding { target } => {
                    let full = PI * target * target;
                    let grow = (full / bud_time * dt).min(self.pool_area - min_pool);
                    if grow <= 0.0 {
                        // The pool ran dry: let go if it's worth it, else sink back.
                        blob.phase = if blob.radius > 0.5 * target {
                            Phase::Free
                        } else {
                            Phase::Melting
                        };
                        blob.cooldown = COOLDOWN;
                        continue;
                    }
                    self.pool_area -= grow;
                    blob.set_area(blob.area() + grow);
                    let g = (blob.radius / target).min(1.0);
                    // A wide, low bulge on the pool that rises as it swells
                    // and draws up tall at the neck before letting go.
                    blob.y = surface + blob.radius * (1.6 * g - 0.7);
                    blob.stretch = 0.7 + 0.65 * g * g;
                    if g >= 1.0 {
                        blob.phase = Phase::Free;
                        blob.cooldown = COOLDOWN;
                        self.stats.budded += 1;
                    }
                }
                Phase::Melting => {
                    let drain = blob.area() * melt_rate * dt;
                    blob.set_area(blob.area() - drain);
                    self.pool_area += drain;
                    let rest = surface - 0.4 * blob.radius;
                    blob.y += (rest - blob.y) * (1.0 - (-2.0 * dt).exp());
                    blob.x += blob.vx * dt;
                    blob.vx *= 1.0 / (1.0 + DRAG * dt);
                    relax(&mut blob.stretch, 0.85, STRETCH_RELAX, dt);
                    if blob.radius < MELTED_RADIUS {
                        self.pool_area += blob.area();
                        blob.radius = 0.0;
                        self.stats.melted += 1;
                    }
                }
            }
            blob.vy = (blob.y - old_y) / dt;
        }
        self.blobs.retain(|b| b.radius > 0.0);
    }

    fn exchange_heat(&mut self, dt: f64) {
        let level = self.pool_level();
        for i in 0..self.blobs.len() {
            let blob = &self.blobs[i];
            let (target, rate) = match blob.phase {
                Phase::Free => {
                    let bottom = blob.y - blob.radius * blob.stretch;
                    let near = 1.0 - ((bottom - level) / HEATER_BAND).clamp(0.0, 1.0);
                    let cool = COOL_RATE * (REF_RADIUS / blob.radius.max(0.01)).min(4.0);
                    // Blend the liquid's pull with the heater's below the band
                    // and any heat pulses (which heat toward fully hot).
                    let heat = HEATER_RATE * near;
                    let pulse = self.pulse_heat(blob.x, blob.y, blob.radius);
                    let rate = cool + heat + pulse;
                    let target = (cool * ambient_temp(blob.y) + heat * POOL_TEMP + pulse) / rate;
                    (target, rate)
                }
                Phase::Budding { .. } | Phase::Melting => (POOL_TEMP, POOL_HEAT_RATE),
            };
            let blob = &mut self.blobs[i];
            blob.temp += (target - blob.temp) * (1.0 - (-rate * dt).exp());
        }
    }

    fn merge(&mut self) {
        let max_radius = self.max_radius();
        let mut i = 0;
        while i < self.blobs.len() {
            let mut j = i + 1;
            while j < self.blobs.len() {
                let (a, b) = (&self.blobs[i], &self.blobs[j]);
                let dist = (b.x - a.x).hypot(b.y - a.y);
                if can_merge(a, b, max_radius) && dist < MERGE_DIST * (a.radius + b.radius) {
                    let other = self.blobs.remove(j);
                    self.blobs[i].absorb(&other, MERGE_STRETCH);
                    self.stats.merged += 1;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }

    fn split(&mut self, dt: f64) {
        let max_radius = self.max_radius();
        let split_radius = SPLIT_FRACTION * max_radius;
        for i in 0..self.blobs.len() {
            let blob = &self.blobs[i];
            if blob.phase != Phase::Free || blob.cooldown > 0.0 {
                continue;
            }
            // Too big for the lamp (it narrowed): always split.
            let forced = blob.radius > max_radius * 1.02;
            let eager = blob.radius > split_radius && blob.temp > HOT && blob.vy > 0.0;
            let chance = if eager {
                SPLIT_RATE * (blob.radius - split_radius) / (max_radius - split_radius)
            } else {
                0.0
            };
            if !forced && !(chance > 0.0 && self.rng.unit() < chance * dt) {
                continue;
            }

            // Pinch vertically into top and bottom parts whose combined
            // footprint matches the parent; the top one is a touch hotter
            // and faster, so they drift apart.
            let top_share = self.rng.range(0.4, 0.6);
            let parent = self.blobs[i].clone();
            let reach = SPLIT_REACH * parent.radius * parent.stretch;
            let mut top = parent.clone();
            top.id = self.next_id;
            self.next_id += 1;
            top.set_area(parent.area() * top_share);
            top.y += reach * (1.0 - top_share);
            top.vy += SPLIT_KICK;
            top.temp = (top.temp + 0.03).min(1.0);

            let bottom = &mut self.blobs[i];
            bottom.set_area(parent.area() * (1.0 - top_share));
            bottom.y -= reach * top_share;
            bottom.vy -= SPLIT_KICK;
            bottom.temp -= 0.03;

            for part in [&mut top, &mut self.blobs[i]] {
                part.stretch = 1.15;
                part.cooldown = COOLDOWN;
                part.prev = part.pose();
            }
            self.blobs.push(top);
            self.stats.split += 1;
        }
    }

    fn spawn(&mut self, dt: f64) {
        let gap_scale = match self.reseed {
            Some(Reseed::Melting) => return,
            Some(Reseed::Refill { .. }) => 1.0 / controls::REFILL_SPAWN_SPEEDUP,
            None => self.heat_spawn_gap(),
        };
        self.spawn_timer -= dt;
        if self.spawn_timer > 0.0 {
            return;
        }
        let surplus = self.pool_surplus();
        self.spawn_timer = self.rng.range(SPAWN_GAP.0, SPAWN_GAP.1) * gap_scale / surplus;

        let buds = self
            .blobs
            .iter()
            .filter(|b| matches!(b.phase, Phase::Budding { .. }))
            .count();
        let deep = self.pool_level() > self.deep_pool() * POOL_DEPTH;
        let max_buds = (1.5 * self.wall_width).round().max(1.0) as usize + usize::from(deep);
        if (self.blobs.len() >= self.target_blobs() && !deep) || buds >= max_buds {
            return;
        }
        self.bud_at(None);
    }

    /// Start a bud on the pool, near `x` or anywhere, if there's room and
    /// wax for it.
    fn bud_at(&mut self, x: Option<f64>) {
        if self.blobs.len() >= MAX_BLOBS {
            return;
        }
        let target = (self.typical_radius() * self.rng.range(BUD_SIZE.0, BUD_SIZE.1))
            .min(MAX_BUD * self.max_radius())
            .max(2.0 * MELTED_RADIUS);
        if self.pool_area - self.min_pool_area() < 0.5 * PI * target * target {
            return;
        }
        let half = (self.half_width_at(0.0) - target).max(0.0);
        let x = match x {
            Some(x) => x.clamp(-half, half),
            None => self.rng.range(-half, half),
        };
        // Just under the surface, where the bud's first step puts it.
        let y = pool_surface(self.pool_level(), x, self.time) - 0.7 * MELTED_RADIUS;
        let blob = self.new_blob(x, y, MELTED_RADIUS, POOL_TEMP, Phase::Budding { target });
        self.pool_area -= blob.area();
        self.blobs.push(blob);
    }

    /// After a width change, nudge the pool so total wax matches the target.
    fn balance_pool(&mut self, dt: f64) {
        let gap = self.wax_target - self.wax_area();
        if gap.abs() > 1e-12 {
            let eased = gap * (1.0 - (-POOL_EASE * dt).exp());
            self.pool_area = (self.pool_area + eased).max(self.min_pool_area().min(self.pool_area));
        }
    }

    /// Keep everything finite and inside the lamp, whatever happened.
    fn contain(&mut self) {
        for blob in &mut self.blobs {
            let sane = [
                blob.x,
                blob.y,
                blob.vx,
                blob.vy,
                blob.radius,
                blob.stretch,
                blob.temp,
            ]
            .iter()
            .all(|v| v.is_finite());
            if !sane {
                *blob = Blob {
                    x: 0.0,
                    y: 0.5,
                    vx: 0.0,
                    vy: 0.0,
                    radius: REF_RADIUS,
                    stretch: 1.0,
                    temp: NEUTRAL_TEMP,
                    phase: Phase::Free,
                    ..blob.clone()
                };
            }
            // Walls are springs, so blobs left outside by a fast resize are
            // pushed back in at a capped speed rather than snapped. This
            // clamp is only a runaway guard.
            let bound = 0.5 * ASPECT_RANGE.1;
            blob.x = blob.x.clamp(-bound, bound);
            blob.y = blob.y.clamp(0.0, 1.0);
            let speed = blob.vx.hypot(blob.vy);
            if speed > MAX_SPEED {
                blob.vx *= MAX_SPEED / speed;
                blob.vy *= MAX_SPEED / speed;
            }
            blob.stretch = blob.stretch.clamp(STRETCH_RANGE.0, STRETCH_RANGE.1);
            blob.temp = blob.temp.clamp(0.0, 1.0);
        }
        if !self.pool_area.is_finite() {
            self.pool_area = self.min_pool_area();
        }
    }

    // --- setup -----------------------------------------------------------

    fn new_blob(&mut self, x: f64, y: f64, radius: f64, temp: f64, phase: Phase) -> Blob {
        let id = self.next_id;
        self.next_id += 1;
        let pose = blob::Pose {
            x,
            y,
            radius,
            stretch: 1.0,
        };
        Blob {
            id,
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            radius,
            stretch: 1.0,
            temp,
            phase,
            prev: pose,
            cooldown: 0.0,
            wander_phase: self.rng.range(0.0, 2.0 * PI),
            wander_freq: self.rng.range(WANDER_FREQ.0, WANDER_FREQ.1),
        }
    }

    /// Opening state: about three quarters of the target blob count afloat
    /// at mixed temperatures, the rest of the wax in the pool.
    fn scatter_initial_blobs(&mut self) {
        let count = (self.target_blobs() * 3).div_ceil(4);
        let typical = self.typical_radius();
        let max_radius = MAX_BUD * self.max_radius();
        let mut afloat = 0.0;
        for _ in 0..count {
            let radius = (typical * self.rng.range(BUD_SIZE.0, BUD_SIZE.1)).min(max_radius);
            // A few tries at a spot that doesn't overlap anything.
            let (mut x, mut y) = (0.0, 0.0);
            for _ in 0..8 {
                y = self.rng.range(0.25, 0.85);
                let half = (self.half_width_at(y) - radius).max(0.0);
                x = self.rng.range(-half, half);
                let clear = self
                    .blobs
                    .iter()
                    .all(|b| (b.x - x).hypot(b.y - y) > 1.2 * (b.radius + radius));
                if clear {
                    break;
                }
            }
            let temp = self.rng.range(0.35, 0.7);
            let blob = self.new_blob(x, y, radius, temp, Phase::Free);
            afloat += blob.area();
            self.blobs.push(blob);
        }
        self.pool_area = (self.wax_target - afloat).max(self.min_pool_area());
        self.wax_target = self.wax_area();
        self.spawn_timer = self.rng.range(0.0, SPAWN_GAP.0);
    }
}

fn can_merge(a: &Blob, b: &Blob, max_radius: f64) -> bool {
    let cap = if a.temp.max(b.temp) > HOT {
        SPLIT_FRACTION * max_radius
    } else {
        max_radius
    };
    a.phase == Phase::Free
        && b.phase == Phase::Free
        && a.cooldown <= 0.0
        && b.cooldown <= 0.0
        && (a.temp - b.temp).abs() < MERGE_TEMP_GAP
        && a.radius.hypot(b.radius) <= cap
}

/// Exponential approach of `value` toward `target` at `rate` per second.
fn relax(value: &mut f64, target: f64, rate: f64, dt: f64) {
    *value += (target - *value) * (1.0 - (-rate * dt).exp());
}

#[cfg(test)]
mod tests;
