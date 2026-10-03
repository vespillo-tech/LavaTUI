//! Wax simulation: a pure, seeded, fixed-step model of a lava lamp. No
//! terminal code.
//!
//! # World units
//!
//! The lamp is always 1.0 tall: `y` runs from 0 (base, the heater) to 1
//! (top). Width is the lamp's *visual* aspect (on-screen width ÷ height),
//! so a blob that is round in world units is round on screen whatever the
//! window size; a new world is centred on `x = 0`. The container is a
//! straight-walled tank that fills the lamp's area.
//!
//! When the lamp changes size or place on screen ([`World::set_frame`]),
//! the [`View`] (the part of the world it shows) first moves with it, so
//! every cell shows the wax it showed before; then it glides (~0.45 s) to
//! the whole lamp height and the new width. The edge that stayed put keeps
//! its place in the world, so the tank grows or shrinks on the side that
//! moved, and the walls glide there (~1.2 s), pushing blobs along: nothing
//! on screen jumps (docs/design.md §2.2).
//!
//! # Model
//!
//! - A **pool** of molten wax sits on the heater, heaped into soft, slowly
//!   breathing mounds. It buds, mostly off the mound tops: a wide bulge
//!   swells out of it, necks off and rises. The deeper the pool, the sooner
//!   it buds, so wax never piles up into a slab.
//! - A few blobs of quite different sizes (now and then a big, slow one),
//!   not many equal ones. Moving blobs stretch along their motion, and the
//!   field draws each as a lumpy cluster of bumps, teardropped to trail a
//!   tail (see `field.rs`).
//! - Free blobs exchange heat with the liquid, which is warm at the base and
//!   cool at the top (smaller blobs change temperature faster). Buoyancy is
//!   proportional to `temp - NEUTRAL_TEMP`; strong, implicit viscous drag
//!   makes the motion slow and heavy. Drag is relative to a slow convection
//!   flow, which adds gentle swirl.
//! - Similar-temperature blobs attract weakly and merge when they overlap
//!   enough; dissimilar ones squeeze past each other. Big, hot, rising blobs
//!   occasionally split in two.
//! - Cooled blobs sink, settle on the pool and melt back into it.
//! - Optionally (`lamp.top_wax`), a thin **top layer** of cool wax rests
//!   under the top of the tank, as in a real lamp: the pool, upside down
//!   and much thinner. Blobs bud from it (drops that hang, swell and let
//!   go) and melt into it by the same code as the pool's, mirrored (a
//!   blob's [`End`]). About a third of the blobs that rise to it melt in;
//!   a big one melts in only a share, then pulls away and sinks. What
//!   melts in shows as a bulge where it joined that slowly spreads out
//!   and evens. It never drips below [`CAP_KEEP`] deep, and the pool tops
//!   it up to that (when it is turned on, or the lamp widens). Turned
//!   off, it thins away into the pool.
//! - Total wax area is conserved by every step (pool + blobs + top layer).
//!   Only a width change adjusts it, slowly, through the pool.
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

pub use blob::{Blob, Phase};
use blob::{End, Ghost};
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
const BUOYANCY: f64 = 0.21;
/// Viscous drag rate (1/s). Terminal velocity = buoyancy / drag, so the
/// hottest blobs top out near 0.065 lamp-heights per second.
const DRAG: f64 = 1.5;
/// Heat exchange rate (1/s) with the liquid for a reference-size blob.
/// Exchange scales with surface / volume, `1 / radius`, softened to this
/// power so the biggest blobs still cool enough at the top to come down
/// rather than pile up under it.
const COOL_RATE: f64 = 0.06;
const COOL_SIZE: f64 = 0.6;
const REF_RADIUS: f64 = 0.08;
/// Free blobs just above the pool are warmed by the heater, big ones more
/// slowly (so a big blob sinking onto the pool settles and melts in rather
/// than bobbing just above it).
const HEATER_BAND: f64 = 0.04;
const HEATER_RATE: f64 = 0.25;
/// Buds and melting blobs approach the pool temperature at this rate (1/s).
const POOL_HEAT_RATE: f64 = 0.6;

/// Wax as a fraction of the container's area.
const FILL: f64 = 0.30;
/// Pool depth (lamp heights) the world aims for, and the least it keeps.
const POOL_DEPTH: f64 = 0.07;
const MIN_POOL_DEPTH: f64 = 0.045;
/// Bud sizes, as ranges of multiples of the typical radius: mostly
/// middling, a quarter big, a fifth small. With merges and splits on top,
/// the lamp shows a 3:1 spread or more.
const BUD_SIZE: (f64, f64) = (0.7, 1.1);
const BIG_BUD: (f64, f64) = (1.6, 2.2);
const BIG_BUD_CHANCE: f64 = 0.25;
const SMALL_BUD: (f64, f64) = (0.3, 0.5);
const SMALL_BUD_CHANCE: f64 = 0.2;
/// While the lamp has no big blob (or no small one) afloat, a new bud is
/// that size this often instead, so a still frame rarely shows a row of
/// look-alikes. Big means at least `BIG_BLOB` typical radii (or most of
/// the largest size), small at most `SMALL_BLOB`.
const MISSING_SIZE_CHANCE: f64 = 0.85;
const BIG_BLOB: f64 = 1.3;
const SMALL_BLOB: f64 = 0.6;
/// The typical radius is at most this fraction of the largest, so in a
/// narrow lamp the big buds still stand out from the rest.
const TYPICAL_OF_MAX: f64 = 0.5;
/// Smallest bud radius (lamp heights): a droplet a couple of pixels across
/// even in a short lamp.
const MIN_BUD: f64 = 0.025;
/// Buds start this much of the way from a random spot to the nearest
/// mound top.
const BUD_CENTRING: f64 = 0.5;
/// Largest bud, as a fraction of the largest blob.
const MAX_BUD: f64 = 0.95;
/// Pool surface ripple amplitude.
const POOL_WAVE: f64 = 0.005;
/// The pool heaps into mounds about this wide (lamp heights), each rising
/// `POOL_MOUND` times the mean depth above it in the middle and thinning by
/// as much toward its edges.
const POOL_HUMP: f64 = 0.9;
const POOL_MOUND: f64 = 0.55;
/// A pool deeper than this (lamp heights) heaps no higher.
const MOUND_DEPTH: f64 = 0.08;
/// Seconds a bud takes to grow to full size. Below `BUD_START` of its
/// full radius it grows in proportion to its size, so it swells steadily
/// out of the pool instead of ballooning in its first frames.
const BUD_TIME: f64 = 6.0;
const BUD_START: f64 = 0.3;
/// How squat a bud starts (its stretch), rounding out as it grows.
const BUD_STRETCH: f64 = 0.72;
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
/// A melting blob sinks into the pool toward its rest depth at this rate
/// (1/s), no faster than `MELT_SINK` (lamp heights / s), and takes
/// `MELT_SINK_EASE` (1/s) to get up to that speed: a big blob settles
/// in rather than dropping through the pool (and its teardrop, which
/// follows its speed, eases too).
const MELT_SINK_RATE: f64 = 2.0;
const MELT_SINK: f64 = 0.06;
const MELT_SINK_EASE: f64 = 1.0;
/// Teardrop taper per unit of vertical speed (lamp heights / s), its cap
/// (a hot blob at full speed is about 0.4 / 1.6 front / back), and how
/// fast (1/s) a blob's taper follows its speed.
const TAPER: f64 = 10.0;
const MAX_TAPER: f64 = 0.5;
const TAPER_EASE: f64 = 3.0;
/// Seconds a merge or split takes to crossfade (see [`Ghost`]), and a
/// skirt to grow in or fade out.
const TOPOLOGY_FADE: f64 = 0.4;
const SKIRT_FADE: f64 = 0.4;
/// How far a melting blob's skirt is drawn in (see `Blob::neck`).
const MELT_SKIRT: f64 = 0.3;

/// The top layer (`lamp.top_wax`). The pool tops it up to `CAP_KEEP`
/// mean depth (lamp heights) and it never drips below that; rising blobs
/// stop joining it past `CAP_FULL` deep. It is drawn at least
/// `field::MIN_CAP_PIXELS` deep, and at most `field::MAX_CAP_PIXELS`.
const CAP_KEEP: f64 = 0.01;
const CAP_FULL: f64 = 0.03;
/// Its temperature, and that of what melts into it or drips from it: the
/// cool end of [`WAX_TEMP`].
const CAP_TEMP: f64 = 0.27;
/// Seconds it takes to appear or go when turned on or off, and the rate
/// (1/s) wax moves between it and the pool meanwhile.
const CAP_FADE: f64 = 1.5;
const CAP_FILL_RATE: f64 = 3.0;
/// How uneven its underside is, as a share of its depth.
const CAP_LUMP: f64 = 0.35;
/// A rising blob whose top comes this close to the underside touches it.
/// While it presses there it sticks (starts melting in) at up to
/// `STICK_RATE` per second: not while it is still warm (above
/// `STICK_TEMP.1`; it cools as it presses), fully once it has cooled to
/// `STICK_TEMP.0`, and less the smaller it is (`STICK_SIZE`, in typical
/// radii: none below the first, full from the second). A small warm blob
/// touches, flattens a little and turns back; a big, cooling one sticks.
const CAP_TOUCH: f64 = 0.004;
const STICK_RATE: f64 = 0.08;
const STICK_TEMP: (f64, f64) = (0.52, 0.62);
const STICK_SIZE: (f64, f64) = (0.4, 1.2);
/// A sticking blob melts in all of it if the layer has room for it (up
/// to `CAP_FULL`), else up to `CAP_SHARE` of it, then pulls away.
const CAP_SHARE: f64 = 0.4;
/// The top layer is cold, stiff wax: what melts into it seeps in slowly
/// (`TOP_MELT_RATE`, a fraction of its area per second, against the hot
/// pool's [`MELT_RATE`]), rising into it no faster than `TOP_MELT_SINK`
/// (lamp heights / s), flattening and spreading under it as it goes
/// (to `TOP_FLATTEN`), only `TOP_MELT_DEPTH` radii into it.
const TOP_MELT_RATE: f64 = 0.3;
const TOP_MELT_SINK: f64 = 0.03;
const TOP_FLATTEN: f64 = 0.6;
const TOP_MELT_DEPTH: f64 = 0.25;
/// A blob pulling away from the top layer lets go once this far clear of
/// it; one that lets go still pressed into it (the layer was switched
/// off) flattens out of it at this rate (1/s).
const PULL_GAP: f64 = 0.01;
const TOP_FLATTEN_RATE: f64 = 4.0;
/// What melts into the top layer bulges it where it joined: a bump
/// `LUMP_WIDTH` × the blob's radius across (half-width) that spreads at
/// `LUMP_SPREAD` (lamp heights / s) while its wax evens out into the rest
/// of the layer at `LUMP_EVEN` (1/s). It holds the warmth of the wax that
/// melted in, cooling to [`CAP_TEMP`] at `LUMP_COOL` (1/s).
const LUMP_WIDTH: f64 = 3.0;
const LUMP_SPREAD: f64 = 0.03;
const LUMP_EVEN: f64 = 0.06;
const LUMP_COOL: f64 = 0.35;
/// Cool wax is heavier than the liquid: a bulge, `SAG_TIME` seconds after
/// the last wax melted into it, sags into a hanging drop with this chance
/// if it holds enough for one; the drop takes `SAG_SHARE` of it.
const SAG_TIME: f64 = 2.0;
const SAG_CHANCE: f64 = 0.85;
const SAG_SHARE: f64 = 0.8;
/// Now and then a drop forms anywhere, while the layer holds more than
/// `DRIP_SPARE` × `CAP_KEEP`: seconds between tries (random in range).
/// Drop sizes (multiples of the typical radius), and the seconds one
/// takes to grow.
const DRIP_GAP: (f64, f64) = (40.0, 90.0);
const DRIP_SPARE: f64 = 2.0;
const DRIP_SIZE: (f64, f64) = (0.3, 0.6);
const DRIP_TIME: f64 = 5.0;
/// A drop hangs squat (`DROP_STRETCH`) and grows long (by `DROP_LENGTH`)
/// as it swells, a teardrop pointing up (its taper eases to `DROP_TAPER`).
const DROP_STRETCH: f64 = 0.8;
const DROP_LENGTH: f64 = 0.55;
const DROP_TAPER: f64 = -0.45;

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
const SPLIT_FRACTION: f64 = 0.8;
const HOT: f64 = 0.6;
const SPLIT_RATE: f64 = 0.1;
/// The top part of a split takes this share of the wax: rarely an even
/// split, so a split adds to the size mix rather than evening it out.
const SPLIT_SHARE: (f64, f64) = (0.25, 0.75);
const SPLIT_KICK: f64 = 0.008;
/// Split halves start this far apart (× parent half-height) and overlap,
/// so the field shows a neck that thins as they part.
const SPLIT_REACH: f64 = 0.55;
/// No merging/melting for this long after a split or detaching.
const COOLDOWN: f64 = 3.0;

/// Largest blob radius (lamp heights): a big blob is about half the lamp
/// tall, a third of an 80x24 lamp across. And as a fraction of the width
/// (two must fit side by side, or a narrow lamp jams).
const MAX_RADIUS: f64 = 0.24;
const MAX_RADIUS_OF_WIDTH: f64 = 0.25;
const MAX_BLOBS: usize = 40;

/// Wall spring stiffness (1/s²), and the flattest a blob pressed on the
/// top wall gets (as a stretch): past that the wall pushes it instead.
const WALL: f64 = 12.0;
const WALL_FLATTEN: f64 = 0.85;
/// Hard caps that keep the sim sane whatever happens.
const MAX_SPEED: f64 = 0.3;
const STRETCH_RANGE: (f64, f64) = (0.6, 2.3);
/// Stretch follows velocity: tall when moving vertically, wide when moving
/// sideways, relaxing at `STRETCH_RELAX` per second. A hot blob at full
/// speed (~0.07/s) aims for about 1.8.
const STRETCH_GAIN: f64 = 12.0;
const STRETCH_RELAX: f64 = 0.6;

/// Lateral meander acceleration amplitude, and frequency range (rad/s).
const WANDER: f64 = 0.008;
const WANDER_FREQ: (f64, f64) = (0.08, 0.25);
/// Background convection: peak liquid speed, and preferred cell width.
const FLOW: f64 = 0.008;
const FLOW_CELL: f64 = 0.8;

/// How fast (1/s) the view and the walls glide to a new lamp size:
/// critically damped, so a glide starts from rest and settles without
/// overshooting. The walls take 800 ms (95 %), so they meet the blobs
/// they push gently; the view glides in two such stages, one chasing the
/// other, so it even starts without a jerk (450 ms: a zoom moves the
/// whole lamp).
const VIEW_GLIDE: f64 = 17.0;
const WALL_GLIDE: f64 = 4.0;
/// Seconds the pool's mounds take to change to a resized lamp's.
const HUMP_FADE: f64 = 0.8;
/// While the walls glide, and `SOFT_SQUEEZE` seconds after, a blob they
/// reach is squeezed at this rate (1/s) rather than at once, so it never
/// jumps; it is drawn past the wall meanwhile, which is off the lamp
/// (walls only close in from outside it).
const SQUEEZE_RATE: f64 = 4.0;
const SOFT_SQUEEZE: f64 = 2.5;
/// Pool area eases toward the volume target at this rate (1/s).
const POOL_EASE: f64 = 0.5;
/// Accepted lamp aspect range.
const ASPECT_RANGE: (f64, f64) = (0.05, 20.0);

/// World width for a lamp whose view is `aspect` wide.
fn world_width(aspect: f64) -> f64 {
    aspect.clamp(ASPECT_RANGE.0, ASPECT_RANGE.1)
}

/// Where the lamp is on screen, in row heights: columns ÷ the cell aspect
/// across (`x`, `width`), rows down (`y`, `height`). Only differences
/// between frames matter, so any origin will do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    fn near(self, other: Frame) -> bool {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        close(self.x, other.x)
            && close(self.y, other.y)
            && close(self.width, other.width)
            && close(self.height, other.height)
    }
}

/// The part of the world the lamp shows: its left edge and bottom, its
/// width and height (world units). Settled, it is the whole lamp height
/// (`y` 0, `height` 1) and the lamp's aspect wide.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct View {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl View {
    /// The settled view `width` wide, centred on `x = 0`.
    fn centred(width: f64) -> Self {
        View {
            x: -0.5 * width,
            y: 0.0,
            width,
            height: 1.0,
        }
    }

    fn top(self) -> f64 {
        self.y + self.height
    }

    fn centre(self) -> f64 {
        self.x + 0.5 * self.width
    }

    fn lerp(self, to: View, t: f64) -> View {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        View {
            x: mix(self.x, to.x),
            y: mix(self.y, to.y),
            width: mix(self.width, to.width),
            height: mix(self.height, to.height),
        }
    }

    /// The view a lamp moved from `from` to `to` on screen shows if every
    /// cell keeps showing the world point it showed. Linear in the view,
    /// so it commutes with [`View::lerp`].
    fn pinned(self, from: Frame, to: Frame) -> View {
        let (kx, ky) = (self.width / from.width, self.height / from.height);
        let top = self.top() - (to.y - from.y) * ky;
        let height = ky * to.height;
        View {
            x: self.x + (to.x - from.x) * kx,
            y: top - height,
            width: kx * to.width,
            height,
        }
    }

    /// One step `dt` of a [`glide`] toward `aim` at `rate`, `speed` per
    /// component.
    fn glide(&mut self, speed: &mut View, aim: View, rate: f64, dt: f64) {
        glide(&mut self.x, &mut speed.x, aim.x, rate, dt);
        glide(&mut self.y, &mut speed.y, aim.y, rate, dt);
        glide(&mut self.width, &mut speed.width, aim.width, rate, dt);
        glide(&mut self.height, &mut speed.height, aim.height, rate, dt);
    }
}

/// One step `dt` of `value` (moving at `speed`) gliding to `to`: a
/// critically damped spring at `rate`, solved exactly. Lands on `to` once
/// within 1e-6 (far below a pixel) and all but still.
fn glide(value: &mut f64, speed: &mut f64, to: f64, rate: f64, dt: f64) {
    let off = *value - to;
    if off.abs() < 1e-6 && speed.abs() < 1e-6 {
        (*value, *speed) = (to, 0.0);
        return;
    }
    let fade = (-rate * dt).exp();
    let c = *speed + rate * off;
    *value = to + (off + c * dt) * fade;
    *speed = (*speed - rate * c * dt) * fade;
}

/// The pool's floor: the walls' middle and width, and the mounds it heaps
/// into, `from`'s giving way to `to`'s (`blend` of the way, 0 … 1) after
/// a resize.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Floor {
    pub centre: f64,
    pub width: f64,
    pub from: Mounds,
    pub to: Mounds,
    pub blend: f64,
}

impl Floor {
    /// `t` of the way to `to`, whose mounds both are counted in.
    fn lerp(self, to: Floor, t: f64) -> Floor {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        Floor {
            centre: mix(self.centre, to.centre),
            width: mix(self.width, to.width),
            blend: mix(self.blend, to.blend),
            ..to
        }
    }
}

/// Mounds laid out over a floor `width` wide around `centre` (where the
/// walls are once they settle): one per [`POOL_HUMP`] of it, lowest at
/// the walls. They stay put while the walls glide.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Mounds {
    pub centre: f64,
    pub width: f64,
    pub humps: f64,
}

impl Mounds {
    /// The mounds of a lamp settled at `view`.
    fn of(view: View) -> Self {
        Mounds {
            centre: view.centre(),
            width: view.width,
            humps: (view.width / POOL_HUMP).round().max(1.0),
        }
    }
}

/// Liquid temperature at height `y`.
#[inline]
pub fn ambient_temp(y: f64) -> f64 {
    AMBIENT_BOTTOM + (AMBIENT_TOP - AMBIENT_BOTTOM) * y.clamp(0.0, 1.0)
}

/// Pool surface height at `x` over `floor`: the mean `level`, heaped into
/// soft mounds (one per [`POOL_HUMP`] of floor, lowest at the walls) that
/// slowly breathe and drift, plus a slow ripple. The mounds average out
/// to about `level`; they are shape only and move no wax.
fn pool_surface(level: f64, x: f64, floor: Floor, time: f64) -> f64 {
    let heap = |m: Mounds| {
        let s = ((x - m.centre) / m.width + 0.5) * m.humps;
        (2.0 * PI * s).cos() - MOUND_SKEW * (2.0 * PI * 1.7 * s + time * 0.04).sin()
    };
    let breathe = 1.0 + MOUND_BREATHE * (time * 0.09).sin();
    let mound = POOL_MOUND * breathe * level.min(MOUND_DEPTH);
    let heap = if floor.blend >= 1.0 {
        heap(floor.to)
    } else {
        let from = heap(floor.from);
        from + (heap(floor.to) - from) * field::smooth(floor.blend)
    };
    let ripple = (x * 9.0 + time * 0.21).sin() + RIPPLE_OVERTONE * (x * 23.0 - time * 0.37).sin();
    level - mound * heap + POOL_WAVE * ripple
}

/// How far the mounds can breathe, and the skew that makes them uneven.
const MOUND_BREATHE: f64 = 0.3;
const MOUND_SKEW: f64 = 0.3;
const RIPPLE_OVERTONE: f64 = 0.6;
/// The most a mound can heap above (or dip below) the mean level, in
/// multiples of `level.min(MOUND_DEPTH)`: full breath times full heap.
const MAX_MOUND: f64 = (1.0 + MOUND_BREATHE) * (1.0 + MOUND_SKEW) * POOL_MOUND;

/// Highest the [`pool_surface`] at mean `level` can reach anywhere.
fn pool_ceiling(level: f64) -> f64 {
    level + MAX_MOUND * level.min(MOUND_DEPTH) + (1.0 + RIPPLE_OVERTONE) * POOL_WAVE
}

/// Underside of the top layer at `x`: `depth` (its mean) below the top,
/// slightly uneven, slowly drifting. Exactly 1 with no layer.
fn cap_underside(depth: f64, x: f64, time: f64) -> f64 {
    let lumps = 0.6 * (x * 5.3 + time * 0.05).sin() + 0.4 * (x * 13.7 - time * 0.083 + 1.3).sin();
    1.0 - depth * (1.0 + CAP_LUMP * lumps)
}

/// A bulge in the top layer where a blob melted in (see [`LUMP_WIDTH`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Lump {
    pub shape: LumpShape,
    /// Its shape at the start of the last step, for interpolation.
    pub prev: LumpShape,
    /// The blob melting into it, or the drop it sags into (it follows
    /// that blob along the layer).
    pub follow: u64,
    /// Seconds since wax last melted into it, and whether it has had its
    /// chance to sag into a drop.
    pub since_fed: f64,
    pub sagged: bool,
}

/// The part of a [`Lump`] the field draws.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct LumpShape {
    pub x: f64,
    pub area: f64,
    /// Half-width.
    pub width: f64,
    pub temp: f64,
}

impl LumpShape {
    pub fn lerp(self, to: LumpShape, t: f64) -> LumpShape {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        LumpShape {
            x: mix(self.x, to.x),
            area: mix(self.area, to.area),
            width: mix(self.width, to.width),
            temp: mix(self.temp, to.temp),
        }
    }

    /// How far it hangs below the layer at `x`: a smooth, compact bump
    /// holding `area`.
    #[inline]
    pub fn depth(self, x: f64) -> f64 {
        let u = (x - self.x) / self.width;
        if u.abs() >= 1.0 {
            return 0.0;
        }
        let bump = (1.0 - u * u) * (1.0 - u * u);
        bump * self.area * (15.0 / 16.0) / self.width
    }

    /// Its deepest point.
    pub fn peak(self) -> f64 {
        self.area * (15.0 / 16.0) / self.width
    }
}

/// Event counters, for tests and a debug HUD.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub budded: u64,
    pub merged: u64,
    pub split: u64,
    pub melted: u64,
    /// Drops that let go of the top layer, blobs that melted into it, and
    /// blobs that melted into it in part and pulled away.
    pub dripped: u64,
    pub capped: u64,
    pub pinched: u64,
}

#[derive(Debug)]
pub struct World {
    time: f64,
    /// Duration of the last step (for interpolating time).
    last_dt: f64,
    /// What the lamp shows (see [`View`]), the view it glides to, the
    /// first stage of that glide (`chase`), their speeds, and the view at
    /// the start of the last step (for interpolation).
    view: View,
    aim: View,
    chase: View,
    speeds: (View, View),
    prev_view: View,
    /// Where the lamp was on screen last ([`World::set_frame`]).
    frame: Option<Frame>,
    /// Width and centre of the walls, and their speeds; they glide to the
    /// aim's.
    wall_width: f64,
    wall_centre: f64,
    wall_speed: (f64, f64),
    /// Whether a resize is still settling (the view, walls or mounds on
    /// their way), and seconds since the walls last moved.
    resizing: bool,
    walls_still: f64,
    /// The pool's mounds and their change (see [`Floor`]), and the floor
    /// at the start of the last step, for interpolation (its blend counted
    /// between the mounds of now).
    mounds: (Mounds, Mounds),
    mound_blend: f64,
    prev_floor: Floor,
    pool_area: f64,
    /// Pool level at the start of the last step, for interpolation.
    prev_pool_level: f64,
    /// Total wax the world aims to hold; changes only with the width.
    wax_target: f64,
    blobs: Vec<Blob>,
    /// What merges and splits replaced, fading out (drawn only).
    ghosts: Vec<Ghost>,
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
    /// Whether the top layer is wanted, how far it is shown (0 … 1, eases
    /// toward `top_wax`), and the wax in it.
    top_wax: bool,
    cap_on: f64,
    cap_area: f64,
    /// `cap_on` and the layer's mean depth at the start of the last step.
    prev_cap_on: f64,
    prev_cap_depth: f64,
    drip_timer: f64,
    /// Bulges in the top layer where blobs melted in.
    lumps: Vec<Lump>,
}

impl World {
    /// A fresh lamp `aspect` wide (visual width ÷ height), seeded so the
    /// same seed always plays out the same way. Starts with some wax
    /// already afloat; run [`World::prewarm`] to skip the opening entirely.
    pub fn new(seed: u64, aspect: f64) -> Self {
        let width = world_width(aspect);
        let mut world = Self {
            time: 0.0,
            last_dt: 0.0,
            view: View::centred(width),
            aim: View::centred(width),
            chase: View::centred(width),
            speeds: (View::default(), View::default()),
            prev_view: View::centred(width),
            frame: None,
            wall_width: width,
            wall_centre: 0.0,
            wall_speed: (0.0, 0.0),
            resizing: false,
            walls_still: SOFT_SQUEEZE,
            mounds: (
                Mounds::of(View::centred(width)),
                Mounds::of(View::centred(width)),
            ),
            mound_blend: 1.0,
            prev_floor: Floor::default(),
            pool_area: 0.0,
            prev_pool_level: 0.0,
            wax_target: FILL * width,
            blobs: Vec::with_capacity(MAX_BLOBS + 1),
            ghosts: Vec::new(),
            accel: Vec::with_capacity(MAX_BLOBS + 1),
            rng: Rng::new(seed),
            next_id: 0,
            spawn_timer: 0.0,
            stats: Stats::default(),
            heat_target: DEFAULT_HEAT,
            heat_level: f64::from(DEFAULT_HEAT),
            pulses: Vec::new(),
            reseed: None,
            top_wax: false,
            cap_on: 0.0,
            cap_area: 0.0,
            prev_cap_on: 0.0,
            prev_cap_depth: 0.0,
            drip_timer: 0.0,
            lumps: Vec::new(),
        };
        world.scatter_initial_blobs();
        world.prev_floor = world.floor();
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

    /// Total wax: pool, blobs and top layer.
    pub fn wax_area(&self) -> f64 {
        self.pool_area
            + self.cap_area
            + self.lumps.iter().map(|l| l.shape.area).sum::<f64>()
            + self.blobs.iter().map(Blob::area).sum::<f64>()
    }

    /// How deep the top layer is shown now (lamp heights).
    #[cfg(test)]
    pub fn top_wax_depth(&self) -> f64 {
        self.cap_depth()
    }

    /// Show the thin layer of wax under the top, or let it go. Either
    /// eases over [`CAP_FADE`].
    pub fn set_top_wax(&mut self, on: bool) {
        self.top_wax = on;
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

    /// The lamp is at `frame` on screen. If it moved or changed size,
    /// the view moves with it so every cell shows what it showed, then
    /// glides to the whole lamp: the lamp's height, and its aspect wide,
    /// keeping in place the side edge that didn't move (both or neither:
    /// the middle). The walls glide after it, pushing blobs, and the pool
    /// slowly adjusts so wax stays at [`FILL`] of the container. The first
    /// frame only says where the view is (centred on it).
    pub fn set_frame(&mut self, frame: Frame) {
        let sane = [frame.x, frame.y, frame.width, frame.height]
            .iter()
            .all(|v| v.is_finite());
        if !sane || frame.width <= 0.0 || frame.height <= 0.0 {
            return;
        }
        let from = self.frame.unwrap_or_else(|| {
            let width = self.view.width / self.view.height * frame.height;
            Frame {
                x: frame.x + 0.5 * (frame.width - width),
                width,
                ..frame
            }
        });
        self.frame = Some(frame);
        if from.near(frame) {
            return;
        }
        self.resizing = true;
        self.view = self.view.pinned(from, frame);
        self.chase = self.chase.pinned(from, frame);
        self.speeds.0 = self.speeds.0.pinned(from, frame);
        self.speeds.1 = self.speeds.1.pinned(from, frame);
        self.prev_view = self.prev_view.pinned(from, frame);
        let stays = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let pivot = match (
            stays(from.x, frame.x),
            stays(from.x + from.width, frame.x + frame.width),
        ) {
            (true, false) => 0.0,
            (false, true) => 1.0,
            _ => 0.5,
        };
        let width = world_width(frame.width / frame.height);
        if (width - self.aim.width).abs() > 1e-9 {
            self.wax_target = FILL * width;
        }
        let at = self.view.x + pivot * self.view.width;
        self.aim = View {
            x: at - pivot * width,
            y: 0.0,
            width,
            height: 1.0,
        };
    }

    /// A lamp `aspect` wide in place of this one: the same height, its
    /// left edge where it was.
    #[cfg(test)]
    pub fn set_aspect(&mut self, aspect: f64) {
        let from = self.frame.unwrap_or(Frame {
            x: 0.0,
            y: 0.0,
            width: self.view.width / self.view.height,
            height: 1.0,
        });
        self.frame = Some(from);
        self.set_frame(Frame {
            width: aspect * from.height,
            ..from
        });
    }

    /// Advance by exactly `dt` seconds (always the fixed step).
    pub fn step(&mut self, dt: f64) {
        self.last_dt = dt;
        self.prev_view = self.view;
        self.prev_floor = self.floor();
        self.prev_pool_level = self.pool_level();
        self.prev_cap_on = self.cap_on;
        self.prev_cap_depth = self.cap_mean_depth();
        for lump in &mut self.lumps {
            lump.prev = lump.shape;
        }
        for blob in &mut self.blobs {
            blob.prev = blob.pose();
            blob.cooldown = (blob.cooldown - dt).max(0.0);
        }
        self.time += dt;
        self.fade(dt);

        self.update_controls(dt);
        self.ease_cap(dt);
        self.even_lumps(dt);
        self.ease_walls(dt);
        self.move_free(dt);
        self.move_attached(dt);
        self.exchange_heat(dt);
        self.merge();
        self.split(dt);
        self.spawn(dt);
        self.drip(dt);
        self.balance_pool(dt);
        self.contain();
        self.carry_ghosts();
    }

    // --- geometry --------------------------------------------------------

    fn half_width(&self) -> f64 {
        0.5 * self.wall_width
    }

    fn bottom_width(&self) -> f64 {
        self.wall_width
    }

    /// The pool's floor, as [`pool_surface`] takes it.
    fn floor(&self) -> Floor {
        Floor {
            centre: self.wall_centre,
            width: self.wall_width,
            from: self.mounds.0,
            to: self.mounds.1,
            blend: self.mound_blend,
        }
    }

    fn max_radius(&self) -> f64 {
        MAX_RADIUS.min(MAX_RADIUS_OF_WIDTH * self.wall_width)
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

    /// The top layer's mean depth (lamp heights), however far it is shown.
    fn cap_mean_depth(&self) -> f64 {
        self.cap_area / self.wall_width
    }

    /// The top layer's depth as shown now: 0 with none, easing in and out.
    fn cap_depth(&self) -> f64 {
        field::smooth(self.cap_on) * self.cap_mean_depth()
    }

    /// The top layer's underside at `x` as it is now, bulges and all.
    fn cap_under(&self, x: f64) -> f64 {
        let lumps: f64 = self.lumps.iter().map(|l| l.shape.depth(x)).sum();
        cap_underside(self.cap_depth(), x, self.time) - field::smooth(self.cap_on) * lumps
    }

    /// All the wax in the top layer, bulges included.
    fn cap_total(&self) -> f64 {
        self.cap_area + self.lumps.iter().map(|l| l.shape.area).sum::<f64>()
    }

    /// The least the top layer keeps (the pool tops it up to this).
    fn cap_keep_area(&self) -> f64 {
        CAP_KEEP * self.wall_width
    }

    /// How many blobs the world aims for (docs/design.md §2.4).
    /// Scales with heat, which eases, so the count changes gradually.
    fn target_blobs(&self) -> usize {
        let base = (2.8 * self.wall_width).clamp(3.0, 16.0);
        ((base * self.heat_blobs()).round() as usize).clamp(2, MAX_BLOBS)
    }

    /// Typical blob radius: the wax not in a full-depth pool, shared out.
    fn typical_radius(&self) -> f64 {
        let cap = field::smooth(self.cap_on) * CAP_KEEP;
        let afloat = self.wax_target - (POOL_DEPTH + cap) * self.bottom_width();
        (afloat.max(0.0) / (self.target_blobs() as f64 * PI))
            .sqrt()
            .min(TYPICAL_OF_MAX * self.max_radius())
    }

    /// Liquid velocity: a slow, gently pulsing row of convection cells that
    /// never flows through the walls.
    fn flow(&self, x: f64, y: f64) -> (f64, f64) {
        let width = self.wall_width;
        let cells = (width / FLOW_CELL).round().max(1.0);
        let kx = PI * cells / width;
        let s = ((x - self.wall_centre) / width + 0.5) * PI * cells;
        let amp = FLOW * (0.75 + 0.25 * (self.time * 0.05).sin()) / PI;
        // Stream function ψ = amp · sin(πy) · sin(s); u = ∂ψ/∂y, v = −∂ψ/∂x.
        let y = y.clamp(0.0, 1.0);
        (
            amp * PI * (PI * y).cos() * s.sin(),
            -amp * kx * (PI * y).sin() * s.cos(),
        )
    }

    // --- step stages -------------------------------------------------------

    /// Fade blobs born of a merge or split in and their ghosts out, and
    /// skirts in or out with the phase.
    fn fade(&mut self, dt: f64) {
        for blob in &mut self.blobs {
            blob.weight = (blob.weight + dt / TOPOLOGY_FADE).min(1.0);
            let attach = if blob.phase == Phase::Free { 0.0 } else { 1.0 };
            let step = dt / SKIRT_FADE;
            blob.attach += (attach - blob.attach).clamp(-step, step);
            match blob.phase {
                Phase::Budding { target } => blob.neck = (blob.radius / target).min(1.0),
                // Pulling away: its skirt draws in to a neck, as a bud's.
                Phase::Melting { left } if left <= 0.0 => {
                    relax(&mut blob.neck, 1.0, 1.0 / SKIRT_FADE, dt);
                }
                // A skirt not shown yet starts as it should be; one that is
                // (a bud whose pool ran dry) eases there.
                Phase::Melting { .. } if blob.prev.attach <= 0.0 => blob.neck = MELT_SKIRT,
                Phase::Melting { .. } => relax(&mut blob.neck, MELT_SKIRT, 1.0 / SKIRT_FADE, dt),
                Phase::Free => {}
            }
            // Rising: the tail hangs below; sinking: it trails above. A
            // drop hangs as a teardrop pointing up.
            let taper = match (blob.phase, blob.end) {
                (Phase::Budding { .. }, End::Top) => DROP_TAPER * blob.neck,
                _ => (TAPER * blob.vy).clamp(-MAX_TAPER, MAX_TAPER),
            };
            relax(&mut blob.taper, taper, TAPER_EASE, dt);
        }
        for ghost in &mut self.ghosts {
            ghost.blob.prev = ghost.blob.pose();
            ghost.blob.weight -= dt / TOPOLOGY_FADE;
        }
        self.ghosts
            .retain(|g| g.blob.weight > 0.0 || g.blob.prev.weight > 0.0);
    }

    /// Fade `blob` out, carried along with the blob `follow`.
    fn add_ghost(&mut self, blob: Blob, follow: u64) {
        if blob.weight > 0.0 || blob.prev.weight > 0.0 {
            self.ghosts.push(Ghost { blob, follow });
        }
    }

    /// Ghosts move with the blob that took their place.
    fn carry_ghosts(&mut self) {
        for ghost in &mut self.ghosts {
            if let Some(b) = self.blobs.iter().find(|b| b.id == ghost.follow) {
                ghost.blob.x += b.x - b.prev.x;
                ghost.blob.y += b.y - b.prev.y;
            }
        }
    }

    /// The view and the walls glide to the aim (after a resize), and the
    /// pool's mounds fade to the aim's.
    fn ease_walls(&mut self, dt: f64) {
        if !self.resizing {
            self.walls_still += dt;
            return;
        }
        self.chase
            .glide(&mut self.speeds.0, self.aim, VIEW_GLIDE, dt);
        self.view
            .glide(&mut self.speeds.1, self.chase, VIEW_GLIDE, dt);
        let was = (self.wall_centre, self.wall_width);
        let (centre, width) = &mut self.wall_speed;
        glide(&mut self.wall_width, width, self.aim.width, WALL_GLIDE, dt);
        glide(
            &mut self.wall_centre,
            centre,
            self.aim.centre(),
            WALL_GLIDE,
            dt,
        );
        let moved = was != (self.wall_centre, self.wall_width);
        self.walls_still = if moved { 0.0 } else { self.walls_still + dt };

        // One change at a time: new mounds wait for the last change to
        // finish, unless they are the ones it started from.
        let (from, to) = self.mounds;
        let want = Mounds::of(self.aim);
        if want == from && self.mound_blend < 1.0 {
            self.mounds = (to, from);
            self.mound_blend = 1.0 - self.mound_blend;
            self.prev_floor.blend = 1.0 - self.prev_floor.blend;
        } else if want != to && self.mound_blend >= 1.0 {
            self.mounds = (to, want);
            (self.mound_blend, self.prev_floor.blend) = (0.0, 0.0);
        }
        (self.prev_floor.from, self.prev_floor.to) = self.mounds;
        if self.mound_blend < 1.0 {
            self.mound_blend = (self.mound_blend + dt / HUMP_FADE).min(1.0);
        }
        self.resizing = self.view != self.aim
            || self.chase != self.aim
            || self.speeds != (View::default(), View::default())
            || self.wall_speed != (0.0, 0.0)
            || (self.wall_centre, self.wall_width) != (self.aim.centre(), self.aim.width)
            || self.mounds.1 != want
            || self.mound_blend < 1.0;
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
        let floor = self.floor();
        let buoyancy = BUOYANCY * self.heat_buoyancy();
        let cap = self.cap_depth();
        // Rising blobs may join the top layer once it is fully in, as far
        // as it has room.
        let room = CAP_FULL * self.wall_width - self.cap_total();
        let joinable = self.top_wax && self.cap_on >= 1.0 && self.reseed.is_none() && room > 0.0;
        let typical = self.typical_radius();
        let squeeze = (self.walls_still < SOFT_SQUEEZE).then(|| 1.0 - (-SQUEEZE_RATE * dt).exp());
        for i in 0..self.blobs.len() {
            let blob = &self.blobs[i];
            if blob.phase != Phase::Free {
                continue;
            }
            let (mut ax, mut ay) = self.accel[i];
            let (hx, hy) = blob.half_extents();
            // Stokes-ish: bigger blobs rise and sink a little faster, but
            // not much: the big ones should still look heavy.
            let size = (blob.radius / REF_RADIUS).powf(0.25).clamp(0.7, 1.1);
            ay += buoyancy * size * (blob.temp - NEUTRAL_TEMP);
            ax += WANDER * (blob.wander_freq * self.time + blob.wander_phase).sin();

            let half = self.half_width();
            let ceiling = if cap > 0.0 {
                self.cap_under(blob.x)
            } else {
                1.0
            };
            // Across from the walls' middle.
            let centre = self.wall_centre;
            let across = blob.x - centre;
            ax += WALL * ((-half + hx - across).max(0.0) - (across + hx - half).max(0.0));
            ay -= WALL * (blob.y + hy - ceiling).max(0.0);

            // Implicit drag toward the liquid's own velocity.
            let (fx, fy) = self.flow(blob.x, blob.y);
            let blob = &mut self.blobs[i];
            blob.vx = (blob.vx + (ax + DRAG * fx) * dt) * damp;
            blob.vy = (blob.vy + (ay + DRAG * fy) * dt) * damp;
            blob.x += blob.vx * dt;
            blob.y += blob.vy * dt;

            let target = 1.0 + STRETCH_GAIN * (blob.vy.abs() - blob.vx.abs());
            relax(&mut blob.stretch, target, STRETCH_RELAX, dt);
            // Walls flatten a blob pressed on them (down to `WALL_FLATTEN`)
            // before they push it: one left stretched into a wall is shoved
            // off it hard, and stretches further with that speed.
            let tallest = (ceiling - blob.y) / blob.radius;
            let widest = (half - (blob.x - centre).abs()) / blob.radius;
            let squeezed = blob.stretch.max(1.0 / widest.max(1e-3));
            let stretch = match squeeze {
                Some(k) => blob.stretch + (squeezed - blob.stretch) * k,
                None => squeezed,
            };
            let flattest = tallest.max(WALL_FLATTEN);
            if blob.end == End::Top && stretch > flattest {
                // Let go of the top layer still pressed into it (it was
                // switched off): flattens out of it rather than snapping.
                blob.stretch = stretch;
                relax(&mut blob.stretch, flattest, TOP_FLATTEN_RATE, dt);
            } else {
                blob.stretch = stretch.min(flattest);
            }

            // Settled onto the pool while sinking: start melting in.
            let bottom = blob.y - blob.radius * blob.stretch;
            let surface = pool_surface(level, blob.x, floor, self.time);
            if bottom < surface + 0.004 && blob.vy < 0.0 && blob.cooldown <= 0.0 {
                blob.phase = Phase::MELTING;
                blob.end = End::Bottom;
            }

            // Risen to the top layer: some melt into it.
            let top = blob.y + blob.radius * blob.stretch;
            if joinable
                && top > ceiling - CAP_TOUCH
                && blob.vy > -0.005
                && blob.cooldown <= 0.0
                && blob.attach <= 0.0
                && blob.prev.attach <= 0.0
                && self.rng.unit() < stick_rate(blob, typical) * dt
            {
                let area = blob.area();
                let left = if area <= room {
                    f64::INFINITY
                } else {
                    (CAP_SHARE * area).min(room)
                };
                blob.phase = Phase::Melting { left };
                blob.end = End::Top;
            }
        }
    }

    /// Buds and melting blobs are attached to their layer (the pool, or
    /// the top layer) and move with it. The top layer's are the pool's
    /// upside down.
    fn move_attached(&mut self, dt: f64) {
        let level = self.pool_level();
        let floor = self.floor();
        let min_pool = self.min_pool_area();
        let keep = self.cap_keep_area();
        let half = self.half_width();
        let squeeze = (self.walls_still < SOFT_SQUEEZE).then(|| 1.0 - (-SQUEEZE_RATE * dt).exp());
        let (melt_rate, bud_time) = match self.reseed {
            Some(Reseed::Melting) => (controls::RESEED_MELT_RATE, BUD_TIME),
            Some(Reseed::Refill { .. }) => (MELT_RATE, BUD_TIME / controls::REFILL_BUD_SPEEDUP),
            None => (MELT_RATE, BUD_TIME / self.pool_surplus()),
        };
        let top_melt_rate = match self.reseed {
            Some(Reseed::Melting) => controls::RESEED_MELT_RATE,
            _ => TOP_MELT_RATE,
        };
        for i in 0..self.blobs.len() {
            let blob = &self.blobs[i];
            if blob.phase == Phase::Free {
                continue;
            }
            let end = blob.end;
            let id = blob.id;
            let out = end.outward();
            // Walls that moved in on an attached blob nudge it along the pool
            // (by its half-width, as for a free one: a tall blob settling to
            // melt beside a wall stays put).
            let inside = (half - blob.half_extents().0).max(0.0);
            let across = blob.x - self.wall_centre;
            let excess = across.abs() - inside;
            // (While walls glide, gently: eased in, not at once at full speed.)
            let x = match squeeze {
                _ if excess <= 0.0 => blob.x,
                Some(k) => blob.x - across.signum() * (excess * k).min(MAX_SPEED * dt),
                None => blob.x - across.signum() * excess.min(MAX_SPEED * dt),
            };
            let surface = match end {
                End::Bottom => pool_surface(level, x, floor, self.time),
                End::Top => self.cap_under(x),
            };
            // A drop draws on the bulge it sags from first.
            let lump = self.lumps.iter().position(|l| l.follow == id);
            let lump_wax = lump.map_or(0.0, |l| self.lumps[l].shape.area);
            let (layer, least) = match end {
                End::Bottom => (self.pool_area, min_pool),
                End::Top => (self.cap_area + lump_wax, keep),
            };
            let blob = &mut self.blobs[i];
            blob.x = x;
            let old_y = blob.y;
            // Wax into (+) or out of (−) the layer, in order.
            let mut moved = [0.0; 2];
            match blob.phase {
                Phase::Free => unreachable!("skipped above"),
                Phase::Budding { target } => {
                    let full = PI * target * target;
                    let start = (blob.radius / (BUD_START * target)).min(1.0);
                    let time = match end {
                        End::Bottom => bud_time,
                        End::Top => DRIP_TIME,
                    };
                    let grow = (full / time * start * dt).min(layer - least);
                    if grow <= 0.0 {
                        // The layer ran dry: let go if it's worth it, else sink back.
                        blob.phase = if blob.radius > 0.5 * target {
                            Phase::Free
                        } else {
                            Phase::MELTING
                        };
                        blob.cooldown = COOLDOWN;
                        continue;
                    }
                    moved[0] = -grow;
                    blob.set_area(blob.area() + grow);
                    let g = (blob.radius / target).min(1.0);
                    match end {
                        // A wide, low bulge on the pool that rises and rounds
                        // out as it swells; the field draws the neck below it.
                        End::Bottom => {
                            blob.y = surface + out * blob.radius * (1.7 * g - 0.75);
                            blob.stretch = BUD_STRETCH + 0.4 * g * g;
                        }
                        // A drop sags lower and longer as it swells, until it
                        // hangs by its tip.
                        End::Top => {
                            blob.stretch = DROP_STRETCH + DROP_LENGTH * g * g;
                            let hang = DROP_STRETCH + DROP_LENGTH + 0.75;
                            blob.y = surface - blob.radius * (hang * g - 0.75);
                        }
                    }
                    if g >= 1.0 {
                        blob.phase = Phase::Free;
                        blob.cooldown = COOLDOWN;
                        match end {
                            End::Bottom => self.stats.budded += 1,
                            End::Top => self.stats.dripped += 1,
                        }
                    }
                }
                Phase::Melting { left } if left > 0.0 => {
                    // The hot pool takes wax in quickly; the cold top layer
                    // slowly, the blob flattening and spreading under it.
                    let (rate, flat, depth, sink_max) = match end {
                        End::Bottom => (melt_rate, 0.85, 0.4, MELT_SINK),
                        End::Top => (top_melt_rate, TOP_FLATTEN, TOP_MELT_DEPTH, TOP_MELT_SINK),
                    };
                    let drain = (blob.area() * rate * dt).min(left);
                    blob.set_area(blob.area() - drain);
                    moved[0] = drain;
                    blob.phase = Phase::Melting { left: left - drain };
                    let rest = surface - out * depth * blob.radius;
                    let sink = (MELT_SINK_RATE * (rest - blob.y)).clamp(-sink_max, sink_max);
                    relax(&mut blob.vy, sink, MELT_SINK_EASE, dt);
                    blob.y += blob.vy * dt;
                    blob.x += blob.vx * dt;
                    blob.vx *= 1.0 / (1.0 + DRAG * dt);
                    relax(&mut blob.stretch, flat, STRETCH_RELAX, dt);
                    if blob.radius < MELTED_RADIUS {
                        moved[1] = blob.area();
                        blob.radius = 0.0;
                        match end {
                            End::Bottom => self.stats.melted += 1,
                            End::Top => self.stats.capped += 1,
                        }
                    }
                }
                Phase::Melting { .. } => {
                    // Given its share: pulls away until clear, then lets go.
                    let reach = blob.radius * blob.stretch + PULL_GAP;
                    let rest = surface + out * reach;
                    let sink =
                        (MELT_SINK_RATE * (rest - blob.y)).clamp(-TOP_MELT_SINK, TOP_MELT_SINK);
                    relax(&mut blob.vy, sink, MELT_SINK_EASE, dt);
                    blob.y += blob.vy * dt;
                    blob.x += blob.vx * dt;
                    blob.vx *= 1.0 / (1.0 + DRAG * dt);
                    relax(&mut blob.stretch, 1.0, STRETCH_RELAX, dt);
                    if out * (blob.y - surface) - blob.radius * blob.stretch >= 0.5 * PULL_GAP {
                        blob.phase = Phase::Free;
                        blob.cooldown = COOLDOWN;
                        self.stats.pinched += 1;
                    }
                }
            }
            if let Phase::Budding { .. } = blob.phase {
                blob.vy = (blob.y - old_y) / dt;
            }
            let (x, radius, temp) = (blob.x, blob.radius, blob.temp);
            for wax in moved.into_iter().filter(|&w| w != 0.0) {
                match end {
                    End::Bottom => self.pool_area += wax,
                    // What melts in bulges the layer where it joined,
                    // bringing its warmth.
                    End::Top if wax > 0.0 => {
                        let lump = self.lump_for(id, x, radius);
                        let s = &mut lump.shape;
                        s.temp = (s.temp * s.area + temp * wax) / (s.area + wax);
                        s.area += wax;
                        lump.since_fed = 0.0;
                    }
                    // A drop draws from the bulge it sags from, then the rest.
                    End::Top => {
                        let from_lump = lump.map_or(0.0, |l| {
                            let take = (-wax).min(self.lumps[l].shape.area);
                            self.lumps[l].shape.area -= take;
                            take
                        });
                        self.cap_area += wax + from_lump;
                    }
                }
            }
        }
        self.blobs.retain(|b| b.radius > 0.0);
    }

    /// The bulge blob `id` melts into at `x` (it follows the blob), new if
    /// it has none yet.
    fn lump_for(&mut self, id: u64, x: f64, radius: f64) -> &mut Lump {
        let at = match self.lumps.iter().position(|l| l.follow == id) {
            Some(at) => at,
            None => {
                let shape = LumpShape {
                    x,
                    area: 0.0,
                    width: LUMP_WIDTH * radius,
                    temp: CAP_TEMP,
                };
                self.lumps.push(Lump {
                    shape,
                    prev: shape,
                    follow: id,
                    since_fed: 0.0,
                    sagged: false,
                });
                self.lumps.len() - 1
            }
        };
        let lump = &mut self.lumps[at];
        lump.shape.x = x;
        lump
    }

    /// Bulges spread out and their wax evens out into the rest of the top
    /// layer; one that has all but gone, with nothing melting into it, goes.
    fn even_lumps(&mut self, dt: f64) {
        let widest = 0.5 * self.wall_width;
        let even = 1.0 - (-LUMP_EVEN * dt).exp();
        for lump in &mut self.lumps {
            lump.since_fed += dt;
            relax(&mut lump.shape.temp, CAP_TEMP, LUMP_COOL, dt);
            lump.shape.width =
                (lump.shape.width + LUMP_SPREAD * dt).min(widest.max(lump.shape.width));
            let moved = lump.shape.area * even;
            lump.shape.area -= moved;
            self.cap_area += moved;
        }
        let blobs = &self.blobs;
        let attached = |id: u64| {
            blobs
                .iter()
                .any(|b| b.id == id && b.end == End::Top && b.phase != Phase::Free)
        };
        let mut gone = 0.0;
        self.lumps.retain(|l| {
            let keep = l.shape.area > 1e-7 || attached(l.follow);
            if !keep {
                gone += l.shape.area;
            }
            keep
        });
        self.cap_area += gone;
    }

    fn exchange_heat(&mut self, dt: f64) {
        for i in 0..self.blobs.len() {
            let blob = &self.blobs[i];
            let (target, rate) = match (blob.phase, blob.end) {
                (Phase::Free, _) => self.free_heat(blob),
                // Pulling away from the top layer: cools as if free.
                (Phase::Melting { left }, End::Top) if left <= 0.0 => self.free_heat(blob),
                (_, End::Bottom) => (POOL_TEMP, POOL_HEAT_RATE),
                (_, End::Top) => (CAP_TEMP, POOL_HEAT_RATE),
            };
            let blob = &mut self.blobs[i];
            blob.temp += (target - blob.temp) * (1.0 - (-rate * dt).exp());
        }
    }

    /// The temperature a free blob heads for, and how fast (1/s): the
    /// liquid's pull, the heater's below its band, and any heat pulses
    /// (which heat toward fully hot).
    fn free_heat(&self, blob: &Blob) -> (f64, f64) {
        let bottom = blob.y - blob.radius * blob.stretch;
        let near = 1.0 - ((bottom - self.pool_level()) / HEATER_BAND).clamp(0.0, 1.0);
        let cool = COOL_RATE
            * (REF_RADIUS / blob.radius.max(0.01))
                .powf(COOL_SIZE)
                .min(4.0);
        let heat = HEATER_RATE * near * (REF_RADIUS / blob.radius.max(0.01)).min(1.0);
        let pulse = self.pulse_heat(blob.x, blob.y, blob.radius);
        let rate = cool + heat + pulse;
        let target = (cool * ambient_temp(blob.y) + heat * POOL_TEMP + pulse) / rate;
        (target, rate)
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
                    let kept = self.blobs[i].clone();
                    self.blobs[i].absorb(&other, MERGE_STRETCH);
                    let id = self.blobs[i].id;
                    for ghost in &mut self.ghosts {
                        if ghost.follow == kept.id || ghost.follow == other.id {
                            ghost.follow = id;
                        }
                    }
                    self.add_ghost(kept, id);
                    self.add_ghost(other, id);
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
            let top_share = self.rng.range(SPLIT_SHARE.0, SPLIT_SHARE.1);
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
                part.weight = 0.0;
                part.prev = part.pose();
            }
            self.blobs.push(top);
            self.add_ghost(parent, self.blobs[i].id);
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
            .filter(|b| matches!(b.phase, Phase::Budding { .. }) && b.end == End::Bottom)
            .count();
        let deep = self.pool_level() > self.deep_pool() * POOL_DEPTH;
        let max_buds = (1.5 * self.wall_width).round().max(1.0) as usize + usize::from(deep);
        // A lamp with its blob count still buds a droplet when it has none
        // afloat: droplets take little wax, and keep the sizes mixed.
        let full = self.blobs.len() >= self.target_blobs() && !deep;
        let droplet = full && !self.sizes_afloat().1 && self.blobs.len() <= self.target_blobs();
        if (full && !droplet) || buds >= max_buds {
            return;
        }
        self.bud_at(None, droplet);
    }

    /// Start a bud on the pool, near `x` or anywhere, if there's room and
    /// wax for it: a small one if `droplet`, else any size.
    fn bud_at(&mut self, x: Option<f64>, droplet: bool) {
        if self.blobs.len() >= MAX_BLOBS {
            return;
        }
        let target = self.bud_radius(droplet);
        if self.pool_area - self.min_pool_area() < 0.5 * PI * target * target {
            return;
        }
        let half = (self.half_width() - target).max(0.0);
        let centre = self.wall_centre;
        let (lo, hi) = (centre - half, centre + half);
        let x = match x {
            Some(x) => x.clamp(lo, hi),
            // Mostly off the tops of the mounds, where the pool is deepest.
            None => {
                let x = centre + self.rng.range(-half, half);
                let Mounds {
                    centre,
                    width: floor,
                    humps,
                } = self.mounds.1;
                let top = (((x - centre) / floor + 0.5) * humps)
                    .floor()
                    .min(humps - 1.0)
                    + 0.5;
                let top = centre + (top / humps - 0.5) * floor;
                (x + BUD_CENTRING * (top - x)).clamp(lo, hi)
            }
        };
        // Just under the surface, where the bud's first step puts it.
        let surface = pool_surface(self.pool_level(), x, self.floor(), self.time);
        let y = surface - 0.7 * MELTED_RADIUS;
        let mut blob = self.new_blob(x, y, MELTED_RADIUS, POOL_TEMP, Phase::Budding { target });
        // Already the flat bulge its first step makes it, fading in.
        blob.stretch = BUD_STRETCH;
        blob.weight = 0.0;
        blob.prev = blob.pose();
        self.pool_area -= blob.area();
        self.blobs.push(blob);
    }

    /// The top layer, fully in, now and then grows a drip where it has wax
    /// to spare.
    fn drip(&mut self, dt: f64) {
        if !self.top_wax || self.cap_on < 1.0 || self.reseed.is_some() {
            return;
        }
        self.sag();
        self.drip_timer -= dt;
        if self.drip_timer > 0.0 {
            return;
        }
        let keep = self.cap_keep_area();
        self.drip_timer = self.rng.range(DRIP_GAP.0, DRIP_GAP.1);
        let drips = (self.blobs.iter())
            .filter(|b| matches!(b.phase, Phase::Budding { .. }) && b.end == End::Top)
            .count();
        let max_drips = self.wall_width.round().max(1.0) as usize;
        let target = (self.typical_radius() * self.rng.range(DRIP_SIZE.0, DRIP_SIZE.1))
            .max(MIN_BUD.min(0.5 * MAX_BUD * self.max_radius()));
        let x =
            self.wall_centre + self.rng.range(-1.0, 1.0) * (self.half_width() - target).max(0.0);
        if self.blobs.len() < MAX_BLOBS
            && drips < max_drips
            && self.cap_area - keep >= 0.5 * PI * target * target
            && self.cap_total() > DRIP_SPARE * keep
        {
            self.cap_area -= self.start_drop(x, target, CAP_TEMP);
        }
    }

    /// Bulges left where blobs melted in sag, a while after: most draw
    /// into a drop that hangs from where the wax joined.
    fn sag(&mut self) {
        // Drops stay drops, smaller than most blobs.
        let largest = (DRIP_SIZE.1 * self.typical_radius()).min(0.5 * MAX_BUD * self.max_radius());
        for i in 0..self.lumps.len() {
            let lump = self.lumps[i];
            if lump.sagged || lump.since_fed < SAG_TIME {
                continue;
            }
            self.lumps[i].sagged = true;
            let target = (SAG_SHARE * lump.shape.area / PI).sqrt().min(largest);
            if target < MIN_BUD || self.blobs.len() >= MAX_BLOBS || self.rng.unit() >= SAG_CHANCE {
                continue;
            }
            let room = (self.half_width() - target).max(0.0);
            let x = lump
                .shape
                .x
                .clamp(self.wall_centre - room, self.wall_centre + room);
            let start = self.start_drop(x, target, lump.shape.temp);
            let lump = &mut self.lumps[i];
            lump.shape.area -= start;
            lump.follow = self.next_id - 1;
        }
    }

    /// Start a drop hanging from the top layer at `x`, growing to `target`
    /// radius; returns the wax it starts with (the caller takes it from
    /// the layer).
    fn start_drop(&mut self, x: f64, target: f64, temp: f64) -> f64 {
        // Just inside the layer, where its first step puts it.
        let y = self.cap_under(x) + 0.75 * MELTED_RADIUS;
        let mut blob = self.new_blob(x, y, MELTED_RADIUS, temp, Phase::Budding { target });
        blob.end = End::Top;
        blob.stretch = DROP_STRETCH;
        blob.weight = 0.0;
        blob.prev = blob.pose();
        let area = blob.area();
        self.blobs.push(blob);
        area
    }

    /// Ease the top layer in or out. Coming in, the pool tops it up to
    /// [`CAP_KEEP`]; going, it thins back into the pool, and whatever
    /// hangs from or melts into it lets go.
    fn ease_cap(&mut self, dt: f64) {
        let (aim, step) = (f64::from(u8::from(self.top_wax)), dt / CAP_FADE);
        self.cap_on += (aim - self.cap_on).clamp(-step, step);
        // Both flows start and stop gently with the fade.
        let shown = field::smooth(self.cap_on);
        if self.top_wax {
            let room = self.pool_area - self.min_pool_area();
            let gap = (self.cap_keep_area() - self.cap_area).min(room);
            // Past full (the lamp narrowed), it thins back into the pool.
            let over = self.cap_area - CAP_FULL * self.wall_width;
            let flow = if gap > 0.0 {
                gap * (CAP_FILL_RATE * shown * dt).min(1.0)
            } else if over > 0.0 {
                -over * (CAP_FILL_RATE * dt).min(1.0)
            } else {
                0.0
            };
            self.pool_area -= flow;
            self.cap_area += flow;
        } else if self.cap_area > 0.0 {
            // The last of it, too little to see, goes at once.
            let flow = if self.cap_area > 1e-9 {
                self.cap_area * (CAP_FILL_RATE * (1.0 - shown) * dt).min(1.0)
            } else {
                self.cap_area
            };
            self.pool_area += flow;
            self.cap_area -= flow;
        }
        if !self.top_wax {
            // Whatever hangs from or melts into it pulls away and lets go.
            for blob in &mut self.blobs {
                if blob.end == End::Top && blob.phase != Phase::Free {
                    blob.phase = Phase::Melting { left: 0.0 };
                }
            }
        }
    }

    /// Whether the lamp has a big blob, and a small one, afloat or budding.
    fn sizes_afloat(&self) -> (bool, bool) {
        let typical = self.typical_radius();
        let big = (BIG_BLOB * typical).min(0.85 * MAX_BUD * self.max_radius());
        let size = |b: &Blob| match b.phase {
            Phase::Budding { target } => target,
            _ => b.radius,
        };
        (
            self.blobs.iter().any(|b| size(b) >= big),
            self.blobs.iter().any(|b| size(b) <= SMALL_BLOB * typical),
        )
    }

    /// A new blob's radius: small if `droplet`, else mostly middling, now
    /// and then a big one or a small one, and more likely whichever size
    /// the lamp is missing.
    fn bud_radius(&mut self, droplet: bool) -> f64 {
        let typical = self.typical_radius();
        let largest = MAX_BUD * self.max_radius();
        let (has_big, has_small) = self.sizes_afloat();
        let pick = self.rng.unit();
        let (lo, hi) = if droplet {
            SMALL_BUD
        } else if !has_big && pick < MISSING_SIZE_CHANCE {
            BIG_BUD
        } else if !has_small && pick < MISSING_SIZE_CHANCE {
            SMALL_BUD
        } else if pick < BIG_BUD_CHANCE {
            BIG_BUD
        } else if pick < BIG_BUD_CHANCE + SMALL_BUD_CHANCE {
            SMALL_BUD
        } else {
            BUD_SIZE
        };
        (typical * self.rng.range(lo, hi))
            .min(largest)
            .max(MIN_BUD.min(0.5 * largest))
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
                    x: self.wall_centre,
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
            blob.x = blob
                .x
                .clamp(self.wall_centre - bound, self.wall_centre + bound);
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
        if !self.cap_area.is_finite() {
            self.cap_area = 0.0;
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
            taper: 0.0,
            weight: 1.0,
            attach: f64::from(u8::from(phase != Phase::Free)),
            neck: 0.0,
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
            weight: pose.weight,
            attach: pose.attach,
            taper: pose.taper,
            neck: pose.neck,
            end: End::Bottom,
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
        let mut afloat = 0.0;
        for _ in 0..count {
            let radius = self.bud_radius(false);
            // A few tries at a spot that doesn't overlap anything.
            let (mut x, mut y) = (0.0, 0.0);
            for _ in 0..8 {
                y = self.rng.range(0.25, 0.85);
                let half = (self.half_width() - radius).max(0.0);
                x = self.wall_centre + self.rng.range(-half, half);
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

/// How readily (per second) free `blob`, pressed against the top layer,
/// sticks to it: a cooled, heavy blob does, a small warm one doesn't.
fn stick_rate(blob: &Blob, typical: f64) -> f64 {
    let cooled = field::smooth((STICK_TEMP.1 - blob.temp) / (STICK_TEMP.1 - STICK_TEMP.0));
    let size = blob.radius / typical.max(1e-6);
    let heavy = field::smooth((size - STICK_SIZE.0) / (STICK_SIZE.1 - STICK_SIZE.0));
    STICK_RATE * cooled * heavy
}

/// Exponential approach of `value` toward `target` at `rate` per second.
fn relax(value: &mut f64, target: f64, rate: f64, dt: f64) {
    *value += (target - *value) * (1.0 - (-rate * dt).exp());
}

#[cfg(test)]
mod tests;
