//! Pop harness: plays the lamp frame by frame the way the app does (fixed
//! sim steps fed by frame times, `Field::prepare(alpha)`, a fill at the
//! style's grid, optionally the reduced grid of adaptive quality) and flags
//! frames where the wax outline jumps instead of moving smoothly, with what
//! happened in the sim that frame.
//!
//! Wax may move fast (a neck forming between two blobs fills in quickly),
//! so speed is no test. A pop is a *discontinuity*: at a pixel near the
//! surface, the density's change this frame differs from what last frame's
//! change predicts (scaled by the two frame times). That surprise, divided
//! by the steepness of the field there, is how far the surface jumped, in
//! sample pixels. Smooth motion, however fast, stays far below
//! [`JUMP_PX`]; a frame with at least [`POP_PIXELS`] such pixels is a pop.
//!
//! `cargo test --release -- --ignored --nocapture pop_harness` prints the
//! long report (`POP_SECS` seconds per run, default 600);
//! `wax_does_not_pop` is the short version in the gate.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use super::LampState;
use crate::sim::{Field, Phase, SURFACE, Sample, World};
use crate::timing::FixedStep;

/// Unexplained surface motion (sample pixels in one frame) that reads as
/// a jump.
const JUMP_PX: f32 = 0.5;
/// Jumping surface pixels a frame needs to count as a pop.
const POP_PIXELS: usize = 4;
/// Only pixels this close to the surface (density) are measured.
const BAND: f32 = 0.2;
/// Steepness floor, so a flat patch near the surface (a saddle) doesn't
/// turn a tiny change into a huge jump.
const MIN_GRADIENT: f32 = 0.03;

/// How the frames are timed.
#[derive(Debug, Clone, Copy)]
enum Pacing {
    /// Exactly 60 fps.
    Steady,
    /// 60 fps with ±4 ms jitter and a late frame (30-80 ms) every ~50.
    Jittered,
}

/// How the lamp is sampled: the full grid, or switching to the reduced
/// grid (adaptive quality) and back every 3 s.
#[derive(Debug, Clone, Copy)]
enum Grid {
    Full,
    Flipping,
}

#[derive(Debug, Default)]
struct Report {
    frames: usize,
    pops: usize,
    /// Pops by what happened in the sim that frame.
    causes: BTreeMap<String, usize>,
    /// The worst pops: (jumping pixels, largest jump, when and why).
    worst: Vec<(usize, f32, String)>,
}

impl Report {
    fn summary(&self) -> String {
        let mut s = format!("{} pops / {} frames", self.pops, self.frames);
        let mut causes: Vec<_> = self.causes.iter().collect();
        causes.sort_by(|a, b| b.1.cmp(a.1));
        for (cause, n) in causes {
            let _ = write!(s, "\n    {n:5} {cause}");
        }
        for (px, max, at) in &self.worst {
            let _ = write!(s, "\n    worst: {px} px (up to {max:.1} px) at {at}");
        }
        s
    }
}

/// LCG for frame jitter (the sim has its own seeded rng).
struct Lcg(u64);

impl Lcg {
    fn unit(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Each blob's phase (and layer), by id.
fn phases(world: &World) -> BTreeMap<u64, u8> {
    let code = |b: &crate::sim::Blob| match (b.phase, b.at_top()) {
        (Phase::Budding { .. }, false) => 0,
        (Phase::Free, _) => 1,
        (Phase::Melting { .. }, false) => 2,
        (Phase::Budding { .. }, true) => 3,
        (Phase::Melting { left }, true) if left > 0.0 => 4,
        (Phase::Melting { .. }, true) => 5,
    };
    world.blobs().iter().map(|b| (b.id, code(b))).collect()
}

/// What changed between two [`phases`] snapshots (`merged` / `split`:
/// how many of each the stats counted meanwhile).
fn events(
    before: &BTreeMap<u64, u8>,
    now: &BTreeMap<u64, u8>,
    merged: u64,
    split: u64,
) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (id, &phase) in now {
        out.push(match (before.get(id), phase) {
            (None, 0) => "bud",
            (None, 3) => "drip",
            (None, _) if split > 0 => "split",
            (None, _) => "new",
            (Some(0), 1) => "detach",
            (Some(1), 2) => "melt-start",
            (Some(0), 2) => "bud-dry",
            (Some(3), 1) => "drip-off",
            (Some(3), 4 | 5) => "drip-dry",
            (Some(1), 4) => "cap-join",
            (Some(4), 5) => "pull-away",
            (Some(5), 1) => "pinch-off",
            (Some(&b), p) if b != p => "phase",
            _ => continue,
        });
    }
    for (id, &phase) in before {
        if !now.contains_key(id) {
            out.push(match (merged > 0, phase) {
                (true, _) => "merge",
                (false, 2) => "melted",
                (false, 4) => "capped",
                _ => "gone",
            });
        }
    }
    if merged > 0 {
        out.push("merge");
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// How much of the surface jumped this frame.
#[derive(Debug, Default)]
struct Jumps {
    /// Pixels that jumped, and the largest jump (sample pixels).
    count: usize,
    max: f32,
    /// Where, on average (`u`, `v` across the lamp, `v` down).
    at: (f32, f32),
}

/// Find the jumps between three consecutive fills `older → prev → cur`,
/// `older_dt` and `dt` the frame times between them.
fn jumps(
    older: &[Sample],
    prev: &[Sample],
    cur: &[Sample],
    cols: usize,
    rows: usize,
    older_dt: f32,
    dt: f32,
) -> Jumps {
    let mut out = Jumps::default();
    // Extrapolating last frame's change is only as good as the frames are
    // even: a late frame shows real curvature as surprise, in proportion.
    let threshold = JUMP_PX * (dt / older_dt).max(older_dt / dt);
    let d = |s: &[Sample], i: usize, j: usize| s[j * cols + i].density;
    let steep = |s: &[Sample], i: usize, j: usize| {
        let gx = 0.5 * (d(s, i + 1, j) - d(s, i - 1, j));
        let gy = 0.5 * (d(s, i, j + 1) - d(s, i, j - 1));
        gx.hypot(gy)
    };
    for j in 1..rows.saturating_sub(1) {
        for i in 1..cols.saturating_sub(1) {
            let (a, b) = (d(prev, i, j), d(cur, i, j));
            if (a - SURFACE).abs() > BAND && (b - SURFACE).abs() > BAND {
                continue;
            }
            let expected = (a - d(older, i, j)) * dt / older_dt;
            let change = b - a;
            // Only the frame the jump starts: the next one sees less change
            // than the jump predicts.
            if change.abs() <= expected.abs() {
                continue;
            }
            let g = steep(prev, i, j).max(steep(cur, i, j)).max(MIN_GRADIENT);
            let jump = (change - expected).abs() / g;
            if jump > threshold {
                out.count += 1;
                out.at.0 += (i as f32 + 0.5) / cols as f32;
                out.at.1 += (j as f32 + 0.5) / rows as f32;
            }
            out.max = out.max.max(jump);
        }
    }
    if out.count > 0 {
        out.at = (out.at.0 / out.count as f32, out.at.1 / out.count as f32);
    }
    out
}

/// Whether the top layer (`lamp.top_wax`) is on.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Top {
    Off,
    On,
    /// On, but switched off for 10 s every 30 s.
    Toggling,
}

impl Top {
    fn on_at(self, t: f64) -> bool {
        match self {
            Top::Off => false,
            Top::On => true,
            Top::Toggling => t % 30.0 < 20.0,
        }
    }
}

/// Play `secs` seconds of a `cols × rows` half-block lamp and report pops.
fn play(
    seed: u64,
    (cols, rows): (usize, usize),
    secs: f64,
    (pacing, grid): (Pacing, Grid),
    top: Top,
) -> Report {
    let (w, h) = (cols, rows * 2);
    let mut world = World::new(seed, w as f64 / h as f64);
    world.set_top_wax(top.on_at(0.0));
    world.prewarm(1200, 1.0 / 120.0);
    let mut clock = FixedStep::new(120);
    let mut field = Field::default();
    let mut lcg = Lcg(seed ^ 0x5eed);
    let mut fills = [(); 3].map(|()| vec![Sample::default(); w * h]);
    let mut lamp = LampState::default();
    let mut report = Report::default();
    let mut seen = phases(&world);
    let (mut t, mut reduced) = (0.0, false);
    let mut dts = [1.0 / 60.0_f32; 2];
    for frame in 0.. {
        if t >= secs {
            break;
        }
        let dt = match pacing {
            Pacing::Steady => 1.0 / 60.0,
            Pacing::Jittered if lcg.unit() < 0.02 => 0.03 + 0.05 * lcg.unit(),
            Pacing::Jittered => 1.0 / 60.0 + 0.008 * (lcg.unit() - 0.5),
        };
        t += dt;
        let toggled = top.on_at(t) != top.on_at(t - dt);
        world.set_top_wax(top.on_at(t));
        let before = world.stats();
        let steps = clock.advance(Duration::from_secs_f64(dt), 1.0);
        for _ in 0..steps {
            world.step(clock.dt_secs());
        }
        let was = reduced;
        if let Grid::Flipping = grid {
            reduced = (t / 3.0) as u64 % 2 == 1;
        }
        field.prepare(&world, clock.alpha());
        fills.rotate_left(1);
        lamp.sample(&field, reduced, w, h);
        fills[2].copy_from_slice(&lamp.samples);
        dts = [dts[1], dt as f32];

        let now = phases(&world);
        let stats = world.stats();
        let mut ev = events(
            &seen,
            &now,
            stats.merged - before.merged,
            stats.split - before.split,
        );
        seen = now;
        if was != reduced {
            ev.push("grid-switch");
        }
        if toggled {
            ev.push("top-toggle");
        }
        if frame < 2 {
            continue;
        }
        report.frames += 1;
        let [older, prev, cur] = &fills;
        let jumped = jumps(older, prev, cur, w, h, dts[0], dts[1]);
        if jumped.count >= POP_PIXELS {
            report.pops += 1;
            let mut label = if ev.is_empty() {
                "none".to_owned()
            } else {
                ev.join("+")
            };
            if steps != 2 {
                let _ = write!(label, " ({steps} steps)");
            }
            *report.causes.entry(label.clone()).or_default() += 1;
            let (u, v) = jumped.at;
            let at = format!("t={t:.2}s at ({u:.2}, {v:.2}) {label}");
            report.worst.push((jumped.count, jumped.max, at));
        }
    }
    report.worst.sort_by_key(|w| std::cmp::Reverse(w.0));
    report.worst.truncate(4);
    report
}

/// `cargo test --release -- --ignored --nocapture pop_harness`
/// (`POP_TOP=on` or `POP_TOP=toggling` for the top layer).
#[test]
#[ignore = "long report"]
fn pop_harness() {
    let secs = std::env::var("POP_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600.0);
    let top = match std::env::var("POP_TOP").as_deref() {
        Ok("on") => Top::On,
        Ok("toggling") => Top::Toggling,
        _ => Top::Off,
    };
    for size in [(80, 24), (160, 45), (250, 70)] {
        for timing in [
            (Pacing::Steady, Grid::Full),
            (Pacing::Jittered, Grid::Full),
            (Pacing::Steady, Grid::Flipping),
        ] {
            let r = play(7, size, secs, timing, top);
            println!(
                "{}x{} {:?} {:?} top {top:?}: {}",
                size.0,
                size.1,
                timing.0,
                timing.1,
                r.summary()
            );
        }
    }
}

#[test]
fn wax_does_not_pop() {
    let timing = (Pacing::Jittered, Grid::Flipping);
    for (seed, size, top) in [
        (3, (80, 24), Top::Off),
        (11, (160, 45), Top::Off),
        (5, (120, 35), Top::Toggling),
    ] {
        let r = play(seed, size, 60.0, timing, top);
        assert_eq!(r.pops, 0, "{size:?} top {top:?}: {}", r.summary());
    }
}
