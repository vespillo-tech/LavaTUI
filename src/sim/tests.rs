use super::*;

const DT: f64 = 1.0 / 120.0;

impl World {
    /// No blobs, a minimal pool (so nothing buds), wax target = what's there.
    fn bare(aspect: f64) -> Self {
        let mut world = Self::new(1, aspect, Shape::Tank);
        world.blobs.clear();
        world.pool_area = world.min_pool_area();
        world.prev_pool_level = world.pool_level();
        world.wax_target = world.wax_area();
        world
    }

    fn add(&mut self, x: f64, y: f64, radius: f64, temp: f64) -> u64 {
        let blob = self.new_blob(x, y, radius, temp, Phase::Free);
        let id = blob.id;
        self.wax_target += blob.area();
        self.blobs.push(blob);
        id
    }

    fn blob(&self, id: u64) -> &Blob {
        self.blobs()
            .iter()
            .find(|b| b.id == id)
            .expect("blob exists")
    }

    fn run(&mut self, steps: u32) {
        for _ in 0..steps {
            self.step(DT);
        }
    }
}

fn assert_close(a: f64, b: f64, tol: f64) {
    assert!((a - b).abs() <= tol, "{a} vs {b} (tol {tol})");
}

#[test]
fn hot_rises_cold_sinks() {
    let mut world = World::bare(1.0);
    let hot = world.add(-0.25, 0.5, 0.06, 0.9);
    let cold = world.add(0.25, 0.5, 0.06, 0.1);
    world.run(480);
    assert!(
        world.blob(hot).y > 0.55,
        "hot blob rose: {}",
        world.blob(hot).y
    );
    assert!(
        world.blob(cold).y < 0.45,
        "cold blob sank: {}",
        world.blob(cold).y
    );
}

#[test]
fn motion_is_slow_and_heavy() {
    // Even the hottest blob crosses the lamp in tens of seconds.
    let mut world = World::bare(1.0);
    let id = world.add(0.0, 0.3, 0.08, 1.0);
    world.run(120 * 5);
    let blob = world.blob(id);
    assert!(blob.vy > 0.0 && blob.vy < 0.08, "speed {}", blob.vy);
}

#[test]
fn rising_blob_cools_and_comes_back_down() {
    let mut world = World::bare(1.0);
    let id = world.add(0.0, 0.2, 0.06, 0.95);
    let mut peak: f64 = 0.0;
    for _ in 0..120 * 90 {
        world.step(DT);
        let Some(blob) = world.blobs.iter().find(|b| b.id == id) else {
            break; // melted back into the pool: the full cycle
        };
        peak = peak.max(blob.y);
    }
    assert!(peak > 0.6, "rose high: {peak}");
    match world.blobs.iter().find(|b| b.id == id) {
        Some(blob) => assert!(blob.y < peak - 0.3 && blob.temp < NEUTRAL_TEMP),
        None => assert_eq!(world.stats().melted, 1),
    }
}

#[test]
fn similar_neighbours_merge_conserving_wax() {
    let mut world = World::bare(1.0);
    world.add(-0.06, 0.5, 0.06, 0.5);
    world.add(0.06, 0.5, 0.06, 0.5);
    let wax = world.wax_area();
    world.run(120 * 10);
    assert_eq!(world.blobs.len(), 1);
    assert_eq!(world.stats().merged, 1);
    assert_close(world.blobs[0].radius, 0.06 * 2f64.sqrt(), 1e-9);
    assert_close(world.wax_area(), wax, 1e-12);
}

#[test]
fn different_temperatures_squeeze_past_instead_of_merging() {
    let mut world = World::bare(1.0);
    world.add(0.0, 0.5, 0.06, 0.95);
    world.add(0.01, 0.62, 0.06, 0.1);
    world.run(120 * 4);
    assert_eq!(world.stats().merged, 0);
    assert_eq!(world.blobs.len(), 2);
}

#[test]
fn big_hot_blobs_split_conserving_wax() {
    let mut world = World::bare(1.5);
    let max = world.max_radius();
    world.add(0.0, 0.3, max, 1.0);
    let wax = world.wax_area();
    world.run(120 * 20);
    assert!(world.stats().split >= 1, "split: {:?}", world.stats());
    // Hot halves are too big to re-fuse.
    assert_eq!(world.stats().merged, 0);
    assert_close(world.wax_area(), wax, 1e-12);
}

#[test]
fn oversized_blob_splits_when_lamp_narrows() {
    let mut world = World::bare(1.0);
    world.add(0.0, 0.5, 0.15, 0.5);
    world.set_aspect(0.2); // max radius → 0.084
    world.run(120 * 2);
    assert!(world.stats().split >= 1);
    let max = world.max_radius();
    assert!(
        world
            .blobs
            .iter()
            .all(|b| b.radius <= max * 1.02 || b.cooldown > 0.0)
    );
}

#[test]
fn pool_buds_blobs_that_detach_and_rise() {
    let mut world = World::new(3, 1.0, Shape::Tank);
    world.blobs.clear();
    world.pool_area = world.wax_target; // all wax in the pool
    world.run(120 * 30);
    assert!(world.stats().budded >= 1, "{:?}", world.stats());
    assert!(
        world
            .blobs
            .iter()
            .any(|b| b.phase == Phase::Free && b.y > 0.3)
    );
}

#[test]
fn volume_is_conserved_every_step() {
    let mut world = World::new(11, 1.3, Shape::Tank);
    let wax = world.wax_area();
    for _ in 0..120 * 60 {
        world.step(DT);
        assert_close(world.wax_area(), wax, 1e-9);
    }
}

#[test]
fn same_seed_same_lamp() {
    let mut a = World::new(42, 1.2, Shape::Tank);
    let mut b = World::new(42, 1.2, Shape::Tank);
    a.run(3000);
    b.run(3000);
    assert_eq!(a.blobs, b.blobs);
    assert_eq!(a.pool_area.to_bits(), b.pool_area.to_bits());

    let mut c = World::new(43, 1.2, Shape::Tank);
    c.run(3000);
    assert_ne!(a.blobs, c.blobs);
}

fn assert_sane(world: &World) {
    assert!(world.wax_area().is_finite() && world.pool_area >= 0.0);
    assert!(world.blobs.len() <= MAX_BLOBS + 1);
    for b in &world.blobs {
        for v in [b.x, b.y, b.vx, b.vy, b.radius, b.stretch, b.temp] {
            assert!(v.is_finite(), "{b:?}");
        }
        let half = 0.5 * world.wall_width * world.shape.width_fraction(b.y);
        assert!(
            b.x.abs() <= half + 0.02 && (0.0..=1.0).contains(&b.y),
            "{b:?}"
        );
        assert!(b.vx.hypot(b.vy) <= MAX_SPEED + 1e-9);
        assert!(b.radius > 0.0 && b.radius <= MAX_RADIUS * 1.02);
    }
}

#[test]
fn stable_for_10k_steps_at_many_shapes() {
    for (seed, aspect, shape) in [
        (1, 1.0, Shape::Tank),
        (2, 0.15, Shape::Tank),
        (3, 4.0, Shape::Tank),
        (4, 0.5, Shape::Bottle),
    ] {
        let mut world = World::new(seed, aspect, shape);
        let wax = world.wax_area();
        for _ in 0..10_000 {
            world.step(DT);
        }
        assert_sane(&world);
        assert_close(world.wax_area(), wax, 1e-9);
        assert!(
            !world.blobs.is_empty(),
            "wax is afloat for {aspect} {shape:?}"
        );
    }
}

#[test]
fn long_run_shows_the_whole_cycle() {
    let mut world = World::new(5, 1.4, Shape::Tank);
    world.run(120 * 300);
    let stats = world.stats();
    assert!(stats.budded > 0 && stats.melted > 0, "{stats:?}");
    assert!(stats.merged > 0 && stats.split > 0, "{stats:?}");
    assert_sane(&world);
}

#[test]
fn resize_eases_walls_without_teleporting() {
    let mut world = World::new(9, 2.0, Shape::Tank);
    world.run(600);
    world.set_aspect(0.5);
    let before: Vec<_> = world.blobs.iter().map(|b| (b.id, b.x, b.y)).collect();
    for _ in 0..120 * 5 {
        world.step(DT);
        // Per-step movement stays tiny: walls push, nothing jumps.
        for b in &world.blobs {
            assert!((b.x - b.prev.x).abs() < 0.01, "{b:?}");
        }
    }
    assert_close(world.wall_width, 0.5, 1e-6);
    assert_sane(&world);
    assert!(
        before
            .iter()
            .any(|(id, ..)| world.blobs.iter().any(|b| b.id == *id))
    );
    // Excess wax melts into the pool and drains away: back to 22 % of the
    // new, 4× smaller lamp within a minute or two, never by deleting blobs.
    world.run(120 * 90);
    assert_close(world.wax_area(), FILL * 0.5, 0.01);
    assert_sane(&world);
}

#[test]
fn bottle_profile_matches_design() {
    let f = |y| Shape::Bottle.width_fraction(y);
    assert_close(f(0.0), 0.56 / 0.78, 1e-12);
    assert_close(f(0.28), 1.0, 1e-12);
    assert_close(f(1.0), 0.40 / 0.78, 1e-12);
    assert_eq!(Shape::Tank.width_fraction(0.3), 1.0);
}

// --- field -----------------------------------------------------------------

#[test]
fn lone_blob_surface_is_at_its_radius() {
    let mut world = World::bare(1.0);
    world.add(0.0, 0.5, 0.1, 0.8);
    // u maps -0.5..0.5 → 0..1 at aspect 1; v = 1 - y.
    let at = |x: f64| world.sample(x + 0.5, 0.5).density;
    assert_close(f64::from(at(0.1)), f64::from(SURFACE), 1e-4);
    assert!(at(0.0) > 1.0 && at(0.09) > SURFACE && at(0.11) < SURFACE);
    assert_eq!(at(0.19), 0.0, "outside the kernel's support");
    let hot = world.sample(0.5, 0.5);
    assert_close(f64::from(hot.temp), 0.8, 0.02);
}

#[test]
fn liquid_temp_where_there_is_no_wax() {
    let world = World::bare(1.0);
    let top = world.sample(0.5, 0.0);
    assert_eq!(top.density, 0.0);
    assert_close(f64::from(top.temp), AMBIENT_TOP, 1e-5);
}

#[test]
fn pool_is_dense_at_the_base() {
    let world = World::bare(1.0);
    let s = world.sample(0.5, 0.999);
    assert!(s.density > 0.9);
    assert_close(f64::from(s.temp), POOL_TEMP, 0.02);
}

#[test]
fn fill_matches_single_samples() {
    let mut world = World::new(21, 1.6, Shape::Tank);
    world.run(1500);
    let mut field = Field::default();
    field.prepare(&world, 0.4);
    let (cols, rows) = (57, 33);
    let mut grid = vec![Sample::default(); cols * rows];
    field.fill(&mut grid, cols, rows);
    let mut wax = 0;
    for j in 0..rows {
        for i in 0..cols {
            let u = (i as f32 + 0.5) / cols as f32;
            let v = (j as f32 + 0.5) / rows as f32;
            let (a, b) = (grid[j * cols + i], field.sample(u, v));
            assert!(
                (a.density - b.density).abs() < 1e-4,
                "({i},{j}) {a:?} {b:?}"
            );
            assert!((a.temp - b.temp).abs() < 1e-4, "({i},{j}) {a:?} {b:?}");
            wax += usize::from(a.density >= SURFACE);
        }
    }
    assert!(wax > 0 && wax < cols * rows);
}

#[test]
fn bottle_field_stays_inside_the_glass() {
    let world = World::new(8, 0.5, Shape::Bottle);
    let mut field = Field::default();
    field.prepare(&world, 1.0);
    assert_eq!((field.shape(), field.aspect()), (Shape::Bottle, 0.5));
    // Bottom corners are outside the bottle: no pool there.
    assert_eq!(world.sample(0.01, 0.999).density, 0.0);
    assert!(world.sample(0.5, 0.999).density > 0.9);
}

#[test]
fn interpolation_moves_between_steps() {
    let mut world = World::bare(1.0);
    world.add(0.0, 0.5, 0.08, 1.0);
    world.run(240);
    let mut field = Field::default();
    let y_at = |field: &Field| field.kernels[0].y;
    field.prepare(&world, 0.0);
    let start = y_at(&field);
    field.prepare(&world, 1.0);
    let end = y_at(&field);
    field.prepare(&world, 0.5);
    assert!(end > start);
    assert!((y_at(&field) - (start + end) / 2.0).abs() < 1e-6);
}

/// `cargo test --release -- --ignored --nocapture bench_fill`
#[test]
#[ignore = "benchmark"]
fn bench_fill() {
    use std::time::Instant;
    for (aspect, cols, rows) in [(0.83, 200, 120), (1.5, 300, 200), (1.33, 80, 48)] {
        let mut world = World::new(7, aspect, Shape::Tank);
        world.prewarm(1200, DT);
        let mut field = Field::default();
        let mut grid = vec![Sample::default(); cols * rows];
        let n = 500;
        let t0 = Instant::now();
        for i in 0..n {
            field.prepare(&world, f64::from(i % 10) / 10.0);
            field.fill(&mut grid, cols, rows);
        }
        let fill = t0.elapsed() / n;
        let t0 = Instant::now();
        let steps = 1200;
        world.prewarm(steps, DT);
        let step = t0.elapsed() / steps;
        println!(
            "{cols}x{rows} aspect {aspect}: fill {fill:?}/frame, step {step:?} ({} blobs)",
            world.blobs.len()
        );
    }
}

// --- controls ------------------------------------------------------------

/// Mean blob count and mean |vy| of free blobs over `secs` seconds, sampled
/// every half second.
fn activity(world: &mut World, secs: u32) -> (f64, f64) {
    let (mut count, mut speed, mut speed_n, mut samples) = (0.0, 0.0, 0.0_f64, 0.0);
    for _ in 0..secs * 2 {
        world.run(60);
        count += world.blobs.len() as f64;
        samples += 1.0;
        for b in world.blobs.iter().filter(|b| b.phase == Phase::Free) {
            speed += b.vy.abs();
            speed_n += 1.0;
        }
    }
    (count / samples, speed / speed_n.max(1.0))
}

#[test]
fn heat_level_clamps() {
    let mut world = World::new(1, 1.0, Shape::Tank);
    assert_eq!(world.heat(), DEFAULT_HEAT);
    world.set_heat(0);
    assert_eq!(world.heat(), 1);
    world.set_heat(9);
    assert_eq!(world.heat(), *HEAT_LEVELS.end());
}

#[test]
fn more_heat_means_more_faster_blobs() {
    let mut results = Vec::new();
    for heat in [1, 3, 5] {
        let mut world = World::new(21, 1.5, Shape::Tank);
        world.set_heat(heat);
        world.run(120 * 60); // settle into the new regime
        results.push(activity(&mut world, 120));
    }
    let [(cold_n, cold_v), (mid_n, mid_v), (hot_n, hot_v)] = results[..] else {
        unreachable!()
    };
    assert!(cold_n < mid_n && mid_n < hot_n, "blob counts {results:?}");
    assert!(cold_v < mid_v && mid_v < hot_v, "speeds {results:?}");
}

#[test]
fn heat_change_eases_in_without_velocity_jumps() {
    let mut base = World::new(8, 1.2, Shape::Tank);
    base.run(600);
    let mut hot = World::new(8, 1.2, Shape::Tank);
    hot.run(600);
    hot.set_heat(5);

    // One step later the two lamps are still all but identical...
    base.step(DT);
    hot.step(DT);
    for (a, b) in base.blobs.iter().zip(&hot.blobs) {
        assert_eq!(a.id, b.id);
        assert!((a.vy - b.vy).abs() < 1e-5, "velocity jumped: {a:?} {b:?}");
    }

    // ...and the heat the sim uses climbs smoothly to the new level.
    let mut last = hot.heat_level;
    for _ in 0..120 * 6 {
        hot.step(DT);
        let step = hot.heat_level - last;
        assert!((0.0..0.02).contains(&step), "heat stepped by {step}");
        last = hot.heat_level;
    }
    assert!(hot.heat_level > 4.9, "eased to {}", hot.heat_level);
}

#[test]
fn reseed_melts_everything_conserving_wax_then_refills() {
    let mut world = World::new(7, 1.2, Shape::Tank);
    world.prewarm(600, DT);
    let wax = world.wax_area();
    let old_ids = world.next_id;
    assert!(!world.blobs.is_empty());

    world.reseed(99);
    assert!(world.is_reseeding());
    let mut melted_at = None;
    for step in 0..120 * 20 {
        world.step(DT);
        assert_close(world.wax_area(), wax, 1e-9);
        if melted_at.is_none() && world.blobs.iter().all(|b| b.id >= old_ids) {
            melted_at = Some(step as f64 * DT);
        }
        if !world.is_reseeding() {
            break;
        }
    }
    let melted_at = melted_at.expect("old wax melted");
    assert!(melted_at <= 2.5, "melt took {melted_at:.2}s");
    assert!(!world.is_reseeding(), "reseed completed");

    // The new lamp buds and rises.
    world.run(120 * 10);
    let risen = world
        .blobs
        .iter()
        .filter(|b| b.id >= old_ids && b.phase == Phase::Free && b.y > 0.2)
        .count();
    assert!(risen >= 2, "new blobs afloat: {:?}", world.blobs);
    assert_close(world.wax_area(), wax, 1e-9);
    assert_sane(&world);
}

#[test]
fn heat_pulse_makes_nearby_wax_rise() {
    let mut world = World::bare(1.0);
    let warmed = world.add(-0.25, 0.6, 0.06, 0.35);
    let control = world.add(0.25, 0.6, 0.06, 0.35);
    // Field coordinates of the left blob: u across, v down.
    world.heat_pulse(0.5 - 0.25, 1.0 - 0.6);
    world.run(120 * 4);
    let (w, c) = (world.blob(warmed), world.blob(control));
    assert!(w.temp > NEUTRAL_TEMP + 0.1, "warmed to {}", w.temp);
    assert!(c.temp < NEUTRAL_TEMP, "control untouched: {}", c.temp);
    assert!(w.y > 0.65 && w.y > c.y + 0.1, "rose: {} vs {}", w.y, c.y);
}

#[test]
fn heat_pulse_on_the_pool_raises_a_bud() {
    let mut world = World::new(3, 1.0, Shape::Tank);
    world.blobs.clear();
    world.pool_area = world.wax_target;
    world.spawn_timer = 1e9; // no natural budding
    world.heat_pulse(0.7, 0.98);
    let bud = world.blobs.first().expect("a bud started");
    assert!(matches!(bud.phase, Phase::Budding { .. }));
    assert_close(bud.x, 0.2, 0.05);
}

#[test]
fn same_seed_and_controls_same_lamp() {
    let play = |seed| {
        let mut world = World::new(seed, 1.1, Shape::Tank);
        world.run(500);
        world.set_heat(5);
        world.heat_pulse(0.4, 0.5);
        world.run(500);
        world.reseed(seed ^ 0xABCD);
        world.run(1500);
        world.set_heat(2);
        world.run(1500);
        world
    };
    let (a, b, c) = (play(42), play(42), play(43));
    assert_eq!(a.blobs, b.blobs);
    assert_eq!(a.pool_area.to_bits(), b.pool_area.to_bits());
    assert_ne!(a.blobs, c.blobs);
}
