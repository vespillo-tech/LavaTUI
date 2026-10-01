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

#[test]
fn normals_point_outward_on_a_circle() {
    let samples = circle(0.25, 0.4);
    let c = N as f32 / 2.0;
    let mut checked = 0;
    for y in 1..N - 1 {
        for x in 1..N - 1 {
            let d = |x: usize, y: usize| samples[y * N + x].density;
            if d(x, y) < SURFACE {
                continue;
            }
            // Same differences as `shade`, y up.
            let gx = (d(x + 1, y) - d(x - 1, y)) * 0.5 * N as f32;
            let gy = (d(x, y - 1) - d(x, y + 1)) * 0.5 * N as f32;
            let n = normal(gx, gy, (d(x, y) - SURFACE) / DOME);
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
    let rim = normal(-5.0, 0.0, 0.0);
    assert!(rim[0] > 0.8, "{rim:?}");
    assert_eq!(normal(0.0, 0.0, 0.0), [0.0, 0.0, 1.0]);
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
