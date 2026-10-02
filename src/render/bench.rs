//! Opt-in stage timings, including cold colour caches and evolving wax.
//! Run alone in release mode so parallel tests do not warm the caches:
//! `cargo test --release bench_compute -- --ignored --nocapture --test-threads=1`

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;

use super::*;
use crate::sim::World;
use crate::theme::{ColorDepth, Palette};

// Test-only native CPU clock: unlike Instant, this excludes descheduling
// on a busy macOS host. Other platforms still report wall stage timings.
#[cfg(target_os = "macos")]
fn cpu_ns() -> u64 {
    unsafe extern "C" {
        fn clock_gettime_nsec_np(clock_id: u32) -> u64;
    }
    // CLOCK_THREAD_CPUTIME_ID from the macOS SDK's _time.h.
    unsafe { clock_gettime_nsec_np(16) }
}

#[cfg(not(target_os = "macos"))]
fn cpu_ns() -> u64 {
    0
}

#[derive(Default)]
struct Times(Vec<u64>);

impl Times {
    fn record(&mut self, start: Instant) {
        self.0.push(start.elapsed().as_nanos() as u64);
    }

    fn report(&self) -> String {
        let mut sorted = self.0.clone();
        sorted.sort_unstable();
        let mean = sorted.iter().sum::<u64>() as f64 / sorted.len() as f64 / 1000.0;
        let p99 = sorted[(sorted.len() - 1) * 99 / 100] as f64 / 1000.0;
        let max = *sorted.last().unwrap() as f64 / 1000.0;
        format!("{mean:.2}/{p99:.2}/{max:.2}")
    }
}

#[test]
#[ignore = "benchmark: mean/p99/max microseconds per stage and output fingerprint"]
fn bench_compute() {
    let frames = std::env::var("LAVA_BENCH_FRAMES")
        .ok()
        .map(|n| n.parse::<usize>().unwrap())
        .unwrap_or(1800);
    assert!(frames > 0);
    println!(
        "size depth style step2 prepare reset fill draw dither diff total cpu_total (mean/p99/max us) fingerprint worst_frame max_blobs events first_dither_us"
    );
    for (cols, rows) in [(80, 24), (160, 40), (250, 70), (300, 90)] {
        let size = format!("{cols}x{rows}");
        if std::env::var("LAVA_BENCH_SIZE").is_ok_and(|s| s != size) {
            continue;
        }
        for (depth, depth_name) in [
            (ColorDepth::TrueColor, "truecolor"),
            (ColorDepth::Ansi256, "256"),
        ] {
            if std::env::var("LAVA_BENCH_DEPTH").is_ok_and(|s| s != depth_name) {
                continue;
            }
            let theme = Theme::new(&Palette::all()[0], depth);
            for id in StyleId::all() {
                let style = id.style();
                if std::env::var("LAVA_BENCH_STYLE").is_ok_and(|s| s != style.name()) {
                    continue;
                }
                let mut world = World::new(7, f64::from(cols) / (2.0 * f64::from(rows)));
                world.prewarm(1200, 1.0 / 120.0);
                let mut field = Field::default();
                let area = Rect::new(0, 0, cols, rows);
                let (mut prev, mut next) = (Buffer::empty(area), Buffer::empty(area));
                let width = usize::from(cols) * usize::from(style.grid().x);
                let height = usize::from(rows) * usize::from(style.grid().y);
                let mut samples = vec![Sample::default(); width * height];
                let mut times: [Times; 8] =
                    std::array::from_fn(|_| Times(Vec::with_capacity(frames)));
                let mut fingerprint = DefaultHasher::new();
                let mut cpu = Times(Vec::with_capacity(frames));
                let mut max_blobs = world.blobs().len();
                let events = world.stats();
                for frame in 0..frames {
                    let cpu_start = cpu_ns();
                    let total = Instant::now();
                    let start = Instant::now();
                    world.step(1.0 / 120.0);
                    world.step(1.0 / 120.0);
                    times[0].record(start);
                    let start = Instant::now();
                    field.prepare(&world, 1.0);
                    times[1].record(start);
                    let start = Instant::now();
                    next.reset();
                    times[2].record(start);
                    let start = Instant::now();
                    field.fill(&mut samples, width, height);
                    times[3].record(start);
                    let start = Instant::now();
                    let dithering = theme.dithering();
                    let paint = dithering.as_ref().unwrap_or(&theme);
                    style.draw(
                        &Canvas {
                            area,
                            samples: &samples,
                            width,
                            height,
                            theme: paint,
                            time: frame as f64 / 60.0,
                            translucent: false,
                        },
                        &mut next,
                    );
                    times[4].record(start);
                    let start = Instant::now();
                    if let Some(paint) = &dithering {
                        dither256::resolve(paint, area, &mut next, false);
                    }
                    times[5].record(start);
                    let start = Instant::now();
                    // Terminal::flush uses the streaming iterator; avoid
                    // adding the legacy diff() Vec allocation to this stage.
                    for change in prev.diff_iter(&next) {
                        std::hint::black_box(change);
                    }
                    times[6].record(start);
                    times[7].record(total);
                    cpu.0.push(cpu_ns() - cpu_start);
                    max_blobs = max_blobs.max(world.blobs().len());
                    // Hash outside the timing: exact field bits and every cell,
                    // including foreground/background and glyph attributes.
                    for sample in &samples {
                        sample.density.to_bits().hash(&mut fingerprint);
                        sample.temp.to_bits().hash(&mut fingerprint);
                    }
                    next.hash(&mut fingerprint);
                    std::mem::swap(&mut prev, &mut next);
                }
                let worst = times[7]
                    .0
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, n)| *n)
                    .unwrap()
                    .0;
                println!(
                    "{size} {depth_name} {} {} {} {:016x} {worst} {max_blobs} {:?} {:.2}",
                    style.name(),
                    times
                        .iter()
                        .map(Times::report)
                        .collect::<Vec<_>>()
                        .join(" "),
                    cpu.report(),
                    fingerprint.finish(),
                    (
                        world.stats().budded - events.budded,
                        world.stats().merged - events.merged,
                        world.stats().split - events.split,
                        world.stats().melted - events.melted
                    ),
                    times[5].0[0] as f64 / 1000.0,
                );
            }
        }
    }
}
