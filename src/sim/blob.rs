//! One lump of wax.

use std::f64::consts::PI;

/// Where a blob is in its life cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// Swelling out of the pool at the base, still attached. Grows by drawing
    /// wax from the pool until it reaches `target` radius, then detaches.
    Budding { target: f64 },
    /// Moving freely: buoyancy, drag, cohesion, walls.
    Free,
    /// Settled back onto the pool; its wax drains into the pool until it is
    /// gone.
    Melting,
}

/// A wax blob. World units: `y` is 0 at the base and 1 at the top, `x` is 0
/// at the lamp's centre line. Read-only outside the sim.
#[derive(Debug, Clone, PartialEq)]
pub struct Blob {
    /// Stable identity (survives merges as the larger parent's id).
    pub id: u64,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    /// Radius of the equal-area circle.
    pub radius: f64,
    /// Vertical scale of the (area-preserving) ellipse: `> 1` is tall and
    /// thin, `< 1` squashed. Horizontal scale is `1 / stretch`.
    pub stretch: f64,
    /// 0 = cold, 1 = hot. Buoyancy is neutral at [`super::NEUTRAL_TEMP`].
    pub temp: f64,
    pub phase: Phase,
    /// How fully the field draws this blob, 0 … 1: a blob born of a
    /// merge or split fades in while what it replaced fades out (a
    /// [`Ghost`]), so the outline morphs instead of snapping.
    pub(super) weight: f64,
    /// How fully the skirt joining it to the pool is drawn, 0 … 1: it
    /// grows in as a blob settles to melt and fades as a bud lets go.
    pub(super) attach: f64,
    /// How far that skirt has drawn in to a neck, 0 (a broad bulge) … 1:
    /// a bud's grows with it, a melting blob's eases to its own.
    pub(super) neck: f64,
    /// Teardrop taper, `> 0` with the tail below (rising): eases toward
    /// what the blob's speed asks for, so the shape never swings with a
    /// sudden change of speed.
    pub(super) taper: f64,
    /// Pose at the start of the last step, for render interpolation.
    pub(super) prev: Pose,
    /// Seconds before this blob may merge or melt again (after a split or
    /// detaching from the pool).
    pub(super) cooldown: f64,
    /// Per-blob lateral meander: `sin(freq * t + phase)`.
    pub(super) wander_phase: f64,
    pub(super) wander_freq: f64,
}

/// The part of a blob's state that rendering interpolates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Pose {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub stretch: f64,
    pub taper: f64,
    pub weight: f64,
    pub attach: f64,
    pub neck: f64,
}

impl Pose {
    pub fn lerp(self, to: Pose, t: f64) -> Pose {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        Pose {
            x: mix(self.x, to.x),
            y: mix(self.y, to.y),
            radius: mix(self.radius, to.radius),
            stretch: mix(self.stretch, to.stretch),
            taper: mix(self.taper, to.taper),
            weight: mix(self.weight, to.weight),
            attach: mix(self.attach, to.attach),
            neck: mix(self.neck, to.neck),
        }
    }
}

/// What a merge or split replaced, still drawn while it fades out: the
/// blob as it was, carried along with the blob that took its place
/// (`follow`, by id), so the outline morphs from the old shape to the new.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Ghost {
    pub blob: Blob,
    pub follow: u64,
}

impl Blob {
    /// Wax area (the 2D "volume"). Stretch preserves it.
    pub fn area(&self) -> f64 {
        PI * self.radius * self.radius
    }

    /// Half-width and half-height of the blob's ellipse.
    pub fn half_extents(&self) -> (f64, f64) {
        (self.radius / self.stretch, self.radius * self.stretch)
    }

    pub(super) fn pose(&self) -> Pose {
        Pose {
            x: self.x,
            y: self.y,
            radius: self.radius,
            stretch: self.stretch,
            taper: self.taper,
            weight: self.weight,
            attach: self.attach,
            neck: self.neck,
        }
    }

    pub(super) fn set_area(&mut self, area: f64) {
        self.radius = (area.max(0.0) / PI).sqrt();
    }

    /// Fold `other` into `self`, conserving area and momentum. The result is
    /// stretched along the axis the two met on, so the merged ellipse covers
    /// roughly the same footprint as the pair did; the field still
    /// crossfades from the pair (see [`Ghost`]).
    pub(super) fn absorb(&mut self, other: &Blob, merge_stretch: f64) {
        let (a, b) = (self.area(), other.area());
        let (wa, wb) = (a / (a + b), b / (a + b));
        let mix = |p: f64, q: f64| p * wa + q * wb;
        let reach = self.radius + other.radius;
        let along = ((other.y - self.y).abs() - (other.x - self.x).abs()) / reach;

        self.x = mix(self.x, other.x);
        self.y = mix(self.y, other.y);
        self.vx = mix(self.vx, other.vx);
        self.vy = mix(self.vy, other.vy);
        self.temp = mix(self.temp, other.temp);
        self.stretch = mix(self.stretch, other.stretch) * (1.0 + merge_stretch * along);
        self.set_area(a + b);
        // Its pose a step ago is the pair's, merged the same way: the frame
        // drawn between the two steps sees one consistent blob.
        let prev_area = PI * (self.prev.radius.powi(2) + other.prev.radius.powi(2));
        self.prev = Pose {
            x: mix(self.prev.x, other.prev.x),
            y: mix(self.prev.y, other.prev.y),
            radius: (prev_area / PI).sqrt(),
            stretch: self.stretch,
            taper: mix(self.prev.taper, other.prev.taper),
            weight: 0.0,
            attach: self.attach.max(other.attach),
            neck: self.neck,
        };
        self.weight = 0.0;
        self.attach = self.prev.attach;
        self.taper = mix(self.taper, other.taper);
        if b > a {
            self.id = other.id;
            self.wander_phase = other.wander_phase;
            self.wander_freq = other.wander_freq;
        }
    }
}
