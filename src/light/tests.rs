use super::*;

const N: usize = 64;

/// One lone blob (the sim's kernel shape) of `radius` world units, centred
/// on an `N × N` canvas (world height 1), at wax temperature `temp`.
fn circle(radius: f32, temp: f32) -> Vec<Sample> {
    const SUPPORT: f32 = 1.8;
    let peak = SURFACE / (1.0 - 1.0 / (SUPPORT * SUPPORT)).powi(2);
    let mut out = Vec::with_capacity(N * N);
    for j in 0..N {
        for i in 0..N {
            let (x, y) = (
                (i as f32 + 0.5) / N as f32 - 0.5,
                (j as f32 + 0.5) / N as f32 - 0.5,
            );
            let q2 = (x * x + y * y) / (SUPPORT * radius).powi(2);
            let density = if q2 < 1.0 {
                peak * (1.0 - q2).powi(2)
            } else {
                0.0
            };
            out.push(Sample { density, temp });
        }
    }
    out
}

fn shade(samples: &[Sample]) -> Vec<f32> {
    let mut out = vec![1.0; samples.len()];
    Lamplight.shade(samples, N, samples.len() / N, &mut out);
    out
}

/// Pixel at `(dx, dy)` world units from the canvas centre (`dy` up).
fn at(buf: &[f32], dx: f32, dy: f32) -> f32 {
    let x = ((0.5 + dx) * N as f32) as usize;
    let y = ((0.5 - dy) * N as f32) as usize;
    buf[y * N + x]
}

/// The normal [`tilt`] describes.
fn normal(g: [f32; 2], bend: [f32; 2], rise: f32, flat: f32) -> [f32; 3] {
    let (k, z) = tilt(g, bend, rise, flat);
    [-g[0] * k, -g[1] * k, z]
}

#[test]
fn normals_point_outward_on_a_circle() {
    let samples = circle(0.25, 0.4);
    let c = N as f32 / 2.0;
    let flat = Rig::new(N, N).flat_diff;
    let mut checked = 0;
    for y in 1..N - 1 {
        for x in 1..N - 1 {
            let d = |x: usize, y: usize| samples[y * N + x].density;
            if d(x, y) < SURFACE {
                continue;
            }
            // Same differences as `shade`, y up.
            let (left, right) = (d(x - 1, y), d(x + 1, y));
            let (up, down, here) = (d(x, y - 1), d(x, y + 1), d(x, y));
            let n = normal(
                [right - left, up - down],
                [left + right - 2.0 * here, up + down - 2.0 * here],
                here - SURFACE,
                flat,
            );
            assert!((dot(n, n) - 1.0).abs() < 1e-4, "unit length: {n:?}");
            assert!(n[2] >= 0.0, "faces the viewer: {n:?}");
            let (rx, ry) = (x as f32 + 0.5 - c, c - (y as f32 + 0.5));
            let r = (rx * rx + ry * ry).sqrt();
            if r > 4.0 {
                let outward = (n[0] * rx + n[1] * ry) / r;
                let tilt = (n[0] * n[0] + n[1] * n[1]).sqrt();
                assert!(outward >= 0.99 * tilt, "outward at ({x}, {y}): {n:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 100);
    // Rim is near edge-on, centre faces the viewer.
    let rim = normal([-5.0, 0.0], [0.0, 0.0], 0.0, FLAT);
    assert!(rim[0] > 0.8, "{rim:?}");
    assert_eq!(normal([0.0, 0.0], [0.0, 0.0], 0.0, FLAT), [0.0, 0.0, 1.0]);
}

#[test]
fn lit_side_is_brighter_than_the_shadow_side() {
    let light = shade(&circle(0.25, 0.4));
    let r = 0.25 * 0.85;
    let (lit, shadow) = (at(&light, -r * 0.7, r * 0.7), at(&light, r * 0.7, -r * 0.7));
    assert!(lit > shadow + 0.2, "lit {lit} vs shadow {shadow}");
    assert!(shadow < 1.0 && shadow > 0.5, "gentle shadow: {shadow}");
    // Left beats right, top beats bottom.
    assert!(at(&light, -r, 0.0) > at(&light, r, 0.0));
    assert!(at(&light, 0.0, r) > at(&light, 0.0, -r));
}

#[test]
fn highlight_sits_up_left_of_centre() {
    let light = shade(&circle(0.25, 0.4));
    let (mut best, mut best_at) = (0.0, (0, 0));
    for y in 0..N / 2 + 8 {
        for x in 0..N {
            if light[y * N + x] > best {
                (best, best_at) = (light[y * N + x], (x, y));
            }
        }
    }
    let (x, y) = best_at;
    assert!(x < N / 2 && y < N / 2, "highlight at {best_at:?}");
    assert!(best > 1.15 && best < 1.6, "subtle highlight: {best}");
}

#[test]
fn hot_wax_glows_into_the_liquid() {
    let (hot, cool) = (shade(&circle(0.15, 0.9)), shade(&circle(0.15, 0.25)));
    // Just outside the surface, in the kernel's tail.
    let (dx, dy) = (0.15 * 1.2, 0.0);
    assert!(at(&hot, dx, dy) > 1.1, "glow {}", at(&hot, dx, dy));
    assert_eq!(at(&cool, dx, dy), 1.0);
    // Far from any wax, nothing.
    assert_eq!(at(&hot, 0.45, 0.45), 1.0);
}

#[test]
fn base_lights_the_bottom_third() {
    let light = shade(&vec![Sample::default(); N * N]);
    for y in 0..N {
        let row = &light[y * N..(y + 1) * N];
        assert!(row.iter().all(|&l| l == row[0]), "uniform across");
        if y < N * 2 / 3 {
            assert_eq!(row[0], 1.0, "row {y}");
        } else {
            assert!(row[0] >= light[(y - 1) * N], "rises toward the base");
        }
    }
    assert!(light[N * N - 1] > 1.3);
}

#[test]
fn degenerate_fields_stay_finite() {
    let flat = |density| vec![Sample { density, temp: 0.6 }; N * N];
    for samples in [flat(0.0), flat(0.5), flat(0.8), flat(40.0)] {
        let light = shade(&samples);
        assert!(
            light
                .iter()
                .all(|l| l.is_finite() && (0.5..=2.5).contains(l))
        );
    }
    // Flat wax faces the viewer: barely touched above the base light.
    assert!((shade(&flat(0.8))[N] - 1.0).abs() < 0.03);
    for (w, h) in [(1, 1), (1, 7), (7, 1), (0, 0), (0, 5)] {
        let samples = vec![
            Sample {
                density: 0.7,
                temp: 0.5
            };
            w * h
        ];
        let mut out = vec![1.0; w * h];
        Lamplight.shade(&samples, w, h, &mut out);
        assert!(out.iter().all(|l| l.is_finite()));
    }
    let nan = Sample {
        density: f32::NAN,
        temp: f32::NAN,
    };
    let mut out = [1.0; 9];
    Lamplight.shade(&[nan; 9], 3, 3, &mut out);
}

/// The live sim at `w × h` (world height 1), mid-run.
fn live(w: usize, h: usize) -> Vec<Sample> {
    use crate::sim::{Field, Shape, World};
    let mut world = World::new(7, w as f64 / h as f64, Shape::Tank);
    world.prewarm(1200, 1.0 / 120.0);
    let mut field = Field::default();
    field.prepare(&world, 1.0);
    let mut samples = vec![Sample::default(); w * h];
    field.fill(&mut samples, w, h);
    samples
}

/// Pixels where coarse shading is off the exact by more than a light
/// step with no pixel-scale feature in the exact light to explain it: no
/// second difference of a step or more (light is quantised to steps, so
/// past three quarters of one), across or down, at or next to the pixel.
fn unexplained_pixels(exact: &[f32], coarse: &[f32], w: usize, h: usize) -> Vec<(usize, usize)> {
    let step = 1.0 / STEPS;
    let at = |x: usize, y: usize| exact[y.min(h - 1) * w + x.min(w - 1)];
    let bend = |x: usize, y: usize| {
        let (l, r) = (at(x.saturating_sub(1), y), at(x + 1, y));
        let (u, d) = (at(x, y.saturating_sub(1)), at(x, y + 1));
        let c = at(x, y);
        (c - 0.5 * (l + r)).abs().max((c - 0.5 * (u + d)).abs())
    };
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if (exact[y * w + x] - coarse[y * w + x]).abs() <= 1.5 * step {
                continue;
            }
            let near = (y.saturating_sub(1)..=y + 1)
                .flat_map(|y| (x.saturating_sub(1)..=x + 1).map(move |x| (x, y)))
                .any(|(x, y)| bend(x, y) > 0.75 * step);
            if !near {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn coarse_dome_matches_exact() {
    // Braille at 200×60, which shades coarse, at a few moments of a few
    // lamps.
    use crate::sim::{Field, Shape, World};
    let (w, h) = (400, 240);
    assert!(h >= FINE);
    let rig = Rig::new(w, h);
    let step = 1.0 / STEPS;
    for seed in [7, 42, 99] {
        let mut world = World::new(seed, w as f64 / h as f64, Shape::Tank);
        world.prewarm(600, 1.0 / 120.0);
        for _ in 0..3 {
            world.prewarm(360, 1.0 / 120.0);
            let mut field = Field::default();
            field.prepare(&world, 1.0);
            let mut samples = vec![Sample::default(); w * h];
            field.fill(&mut samples, w, h);
            let (mut exact, mut coarse) = (vec![0.0; w * h], vec![0.0; w * h]);
            rig.shade_exact(&samples, &mut exact);
            rig.shade_coarse(&samples, &mut coarse);
            // Off by a step at most, except where the exact light turns
            // within a pixel or two (a small blob's highlight, a neck's
            // saddle, see `FINE`), which nodes two pixels apart can't
            // hold. The dome itself has no creases, so those are few and
            // only a few steps off.
            let (mut moved, mut far, mut most) = (0, 0, 0.0f32);
            for (e, c) in exact.iter().zip(&coarse) {
                let d = (e - c).abs();
                moved += usize::from(d > 0.0);
                far += usize::from(d > 1.5 * step);
                most = most.max(d);
            }
            assert!(moved * 100 < w * h, "seed {seed}: {moved} pixels moved");
            assert!(
                far * 3_000 < w * h,
                "seed {seed}: {far} pixels moved > a step"
            );
            assert!(
                most <= 4.5 * step,
                "seed {seed}: off by {} steps",
                most / step
            );
            let lost = unexplained_pixels(&exact, &coarse, w, h);
            assert!(
                lost.is_empty(),
                "seed {seed}: smooth shading off at {lost:?}"
            );
            // The edge, glow and base light are exact: liquid is untouched.
            for ((e, c), s) in exact.iter().zip(&coarse).zip(&samples) {
                if s.density <= WAX_FROM {
                    assert_eq!(e, c);
                }
            }
        }
    }
}

#[test]
fn every_pixel_is_lit_at_any_size() {
    // Both paths, odd sizes, partial runs, both canvas edges, and the
    // widest coarse canvas and one past it.
    let sizes = [
        (1, 1),
        (2, 3),
        (17, 9),
        (1, FINE),
        (3, FINE + 1),
        (RUN + 1, FINE + 3),
        (2 * RUN, FINE),
        (61, 2 * FINE - 1),
        (COARSE_WIDTH, FINE),
        (COARSE_WIDTH + 1, FINE),
    ];
    for (w, h) in sizes {
        let samples = live(w, h);
        let mut out = vec![f32::NAN; w * h];
        Lamplight.shade(&samples, w, h, &mut out);
        assert!(
            out.iter().all(|l| (0.5..=2.5).contains(l)),
            "{w}x{h}: {:?}",
            out.iter().find(|l| !(0.5..=2.5).contains(*l))
        );
    }
}

#[test]
fn coarse_degenerate_fields_stay_finite() {
    let (w, h) = (40, FINE);
    let flat = |density| vec![Sample { density, temp: 0.6 }; w * h];
    for samples in [flat(0.0), flat(0.5), flat(0.8), flat(40.0)] {
        let mut out = vec![f32::NAN; w * h];
        Lamplight.shade(&samples, w, h, &mut out);
        assert!(out.iter().all(|l| l.is_finite() && (0.5..=2.5).contains(l)));
    }
}

/// Cost of the pass alone on the live sim, against the field fill it
/// follows. `cargo test --release -- --ignored --nocapture bench_light`
#[test]
#[ignore = "benchmark"]
fn bench_light() {
    use crate::sim::{Field, Shape, World};
    use std::time::{Duration, Instant};

    let best_of = |mut f: Box<dyn FnMut() + '_>| {
        (0..5)
            .map(|_| {
                let t = Instant::now();
                for _ in 0..100 {
                    f();
                }
                t.elapsed() / 100
            })
            .min()
            .unwrap_or(Duration::ZERO)
    };
    // Half-block and braille grids at 200x60.
    for (w, h) in [(200usize, 120usize), (400, 240)] {
        let mut world = World::new(7, w as f64 / h as f64, Shape::Tank);
        world.prewarm(1200, 1.0 / 120.0);
        let mut field = Field::default();
        field.prepare(&world, 1.0);
        let mut samples = vec![Sample::default(); w * h];
        let fill = best_of(Box::new(|| field.fill(&mut samples, w, h)));
        let mut out = vec![1.0; w * h];
        let shade = best_of(Box::new(|| Lamplight.shade(&samples, w, h, &mut out)));
        println!("{w}x{h}: shade {shade:>8.2?}  (field fill {fill:>8.2?})");
    }
}
