use super::*;

const DT: f64 = 1.0 / 120.0;

impl World {
    /// No blobs, a minimal pool (so nothing buds), wax target = what's there.
    fn bare(aspect: f64) -> Self {
        let mut world = Self::new(1, aspect);
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

/// The wax colour span sits inside the temperatures the sim produces:
/// above the coolest liquid, below the pool.
#[test]
fn wax_temp_span_is_inside_the_sim_range() {
    let (cool, hot) = (f64::from(WAX_TEMP.0), f64::from(WAX_TEMP.1));
    assert!(AMBIENT_TOP < cool && cool < NEUTRAL_TEMP);
    assert!(NEUTRAL_TEMP < hot && hot < POOL_TEMP);
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
    // Close (but not yet merging): cohesion has to pull them in against
    // their meander.
    world.add(-0.045, 0.5, 0.06, 0.5);
    world.add(0.045, 0.5, 0.06, 0.5);
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
    let mut world = World::new(3, 1.0);
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
    let mut world = World::new(11, 1.3);
    let wax = world.wax_area();
    for _ in 0..120 * 60 {
        world.step(DT);
        assert_close(world.wax_area(), wax, 1e-9);
    }
}

#[test]
fn same_seed_same_lamp() {
    let mut a = World::new(42, 1.2);
    let mut b = World::new(42, 1.2);
    a.run(3000);
    b.run(3000);
    assert_eq!(a.blobs, b.blobs);
    assert_eq!(a.pool_area.to_bits(), b.pool_area.to_bits());

    let mut c = World::new(43, 1.2);
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
        let half = 0.5 * world.wall_width;
        assert!(
            b.x.abs() <= half + 0.02 && (0.0..=1.0).contains(&b.y),
            "{b:?}"
        );
        assert!(b.vx.hypot(b.vy) <= MAX_SPEED + 1e-9);
        assert!(b.radius > 0.0 && b.radius <= MAX_RADIUS * 1.02);
    }
}

#[test]
fn stable_for_10k_steps_at_many_aspects() {
    for (seed, aspect) in [(1, 1.0), (2, 0.15), (3, 4.0), (4, 0.5)] {
        let mut world = World::new(seed, aspect);
        let wax = world.wax_area();
        for _ in 0..10_000 {
            world.step(DT);
        }
        assert_sane(&world);
        assert_close(world.wax_area(), wax, 1e-9);
        assert!(!world.blobs.is_empty(), "wax is afloat for {aspect}");
    }
}

#[test]
fn long_run_shows_the_whole_cycle() {
    let mut world = World::new(5, 1.4);
    world.run(120 * 300);
    let stats = world.stats();
    assert!(stats.budded > 0 && stats.melted > 0, "{stats:?}");
    assert!(stats.merged > 0 && stats.split > 0, "{stats:?}");
    assert_sane(&world);
}

/// The tank, at aspects from a narrow side panel to a wide short strip,
/// shows a few blobs of quite different sizes in any still frame, and now
/// and then a big one, without getting busier or faster: free blobs span
/// 3:1 or more over a run and at least 2:1 in most frames, and the widest
/// is a third of the lamp across (or 0.4 of its height, in a wide one) much
/// of the time.
#[test]
fn bleed_lamp_has_big_varied_blobs() {
    for aspect in [0.6, 1.25, 2.0, 3.6] {
        let (mut frames, mut varied, mut big) = (0, 0, 0);
        let (mut count, mut speed, mut moving) = (0.0, 0.0, 0.0);
        for seed in [3, 5, 9] {
            let mut world = World::new(seed, aspect);
            world.prewarm(120 * 20, DT);
            let (mut smallest, mut largest) = (f64::MAX, 0.0_f64);
            for _ in 0..180 {
                world.run(60);
                let free: Vec<f64> = (world.blobs.iter())
                    .filter(|b| b.phase == Phase::Free)
                    .map(|b| b.radius)
                    .collect();
                let lo = free.iter().copied().fold(f64::MAX, f64::min);
                let hi = free.iter().copied().fold(0.0, f64::max);
                (smallest, largest) = (smallest.min(lo), largest.max(hi));
                frames += 1;
                varied += usize::from(free.len() >= 2 && hi >= 2.0 * lo);
                big += usize::from(2.0 * hi >= (aspect / 3.0).min(0.4));
                count += world.blobs.len() as f64;
                for b in world.blobs.iter().filter(|b| b.phase == Phase::Free) {
                    speed += b.vy.abs();
                    moving += 1.0;
                }
            }
            assert!(
                largest >= 3.0 * smallest,
                "aspect {aspect} seed {seed}: sizes {smallest:.3}..{largest:.3}"
            );
            assert_sane(&world);
        }
        let share = |n: usize| n as f64 / f64::from(frames);
        let (count, speed) = (count / f64::from(frames), speed / moving);
        println!(
            "aspect {aspect}: varied {:.2}, big {:.2}, {count:.1} blobs, |vy| {speed:.4}",
            share(varied),
            share(big)
        );
        assert!(
            share(varied) >= 0.45,
            "aspect {aspect}: varied {varied}/{frames}"
        );
        assert!(share(big) >= 0.4, "aspect {aspect}: big {big}/{frames}");
        // About its blob count, give or take a bud and a melting blob.
        let target = World::new(1, aspect).target_blobs() as f64;
        assert!(
            count <= 1.25 * target + 2.0,
            "aspect {aspect}: {count:.1} blobs"
        );
        assert!(speed < 0.022, "aspect {aspect}: mean speed {speed:.4}");
    }
}

#[test]
fn resize_eases_walls_without_teleporting() {
    let mut world = World::new(8, 2.0);
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
    // Excess wax melts into the pool and drains away: back to the fill of
    // the new, 4× smaller lamp within about two minutes (big blobs take a
    // while to cycle back down), never by deleting blobs.
    world.run(120 * 120);
    assert_close(world.wax_area(), FILL * 0.5, 0.01);
    assert_sane(&world);
}

// --- field -----------------------------------------------------------------

/// A lone blob is lumpy (a main bump and lobes), but it draws about its
/// own area, whatever its id and the time, stays near its centre, and the
/// kernels' support is finite.
#[test]
fn lone_blob_covers_its_own_area() {
    let (cols, rows) = (200, 200);
    let mut grid = vec![Sample::default(); cols * rows];
    let mut field = Field::default();
    let mut total = 0.0;
    for id in 0..100 {
        let mut world = World::bare(1.0);
        world.add(0.0, 0.5, 0.1, 0.8);
        world.blobs[0].id = id;
        world.time = id as f64 * 3.7;
        field.prepare(&world, 1.0);
        field.fill(&mut grid, cols, rows);
        // The top three quarters: clear of the pool.
        let wax = grid[..cols * 150].iter().filter(|s| s.density >= SURFACE);
        let ratio = wax.count() as f64 / (cols * rows) as f64 / (PI * 0.01);
        assert!((0.7..1.4).contains(&ratio), "id {id}: area × {ratio:.2}");
        total += ratio;
        let at = |x: f64, y: f64| world.sample(x + 0.5, 1.0 - y).density;
        assert!(at(0.0, 0.5) > SURFACE, "id {id}: centre is wax");
        assert_eq!(at(0.3, 0.5) + at(0.0, 0.85), 0.0, "outside the support");
    }
    assert_close(total / 100.0, 1.0, 0.05);
    let hot = world_with_blob(0.8).sample(0.5, 0.5);
    assert_close(f64::from(hot.temp), 0.8, 0.02);
}

fn world_with_blob(temp: f64) -> World {
    let mut world = World::bare(1.0);
    world.add(0.0, 0.5, 0.1, temp);
    world
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
    let mut world = World::bare(1.0);
    world.pool_area = POOL_DEPTH * world.bottom_width();
    let s = world.sample(0.5, 0.999);
    assert!(s.density > 0.9);
    // Glowing hot over the heater, a cooler skin at the surface.
    assert!(f64::from(s.temp) > 0.85 && f64::from(s.temp) <= POOL_TEMP);
    let skin = world.sample(
        0.5,
        1.0 - pool_surface(world.pool_level(), 0.0, world.bottom_width(), world.time) + 0.01,
    );
    assert!(
        skin.density > SURFACE && skin.temp < s.temp - 0.15,
        "{skin:?}"
    );
}

/// `fill` (which culls by bounding boxes and the pool's ceiling) matches
/// sampling every pixel one by one, over a mound's whole breath, at a few
/// aspects and at grids coarse enough to fade lobes and floor the pool.
#[test]
fn fill_matches_single_samples() {
    let mut field = Field::default();
    for (seed, aspect) in [(21, 1.6), (4, 0.6), (9, 0.5)] {
        let mut world = World::new(seed, aspect);
        world.run(1500);
        // Ten frames 8 s apart span the mounds' 70 s breath.
        for frame in 0..10 {
            world.run(960);
            field.prepare(&world, 0.4);
            for (cols, rows) in [(57, 33), (14, 20), (9, 7)] {
                let mut grid = vec![Sample::default(); cols * rows];
                field.fill(&mut grid, cols, rows);
                let mut wax = 0;
                for j in 0..rows {
                    for i in 0..cols {
                        let u = (i as f32 + 0.5) / cols as f32;
                        let v = (j as f32 + 0.5) / rows as f32;
                        let (a, b) = (grid[j * cols + i], field.sample_on_grid(u, v, cols, rows));
                        let at = format!("seed {seed} frame {frame} {cols}x{rows} ({i},{j})");
                        assert!((a.density - b.density).abs() < 1e-4, "{at}: {a:?} {b:?}");
                        assert!((a.temp - b.temp).abs() < 1e-4, "{at}: {a:?} {b:?}");
                        wax += usize::from(a.density >= SURFACE);
                    }
                }
                assert!(wax > 0 && wax < cols * rows);
            }
        }
    }
}

/// The pool's surface never rises above [`pool_ceiling`], the bound
/// `fill` culls the pool's rows by, at any level, place or time.
#[test]
fn pool_surface_stays_under_its_ceiling() {
    let mut highest: f64 = 0.0;
    for level in [0.01, 0.045, 0.07, MOUND_DEPTH, 0.2] {
        let mound = MAX_MOUND * f64::min(level, MOUND_DEPTH);
        for floor in [0.3, 0.9, 2.5] {
            for t in 0..4000 {
                let time = f64::from(t) * 0.37;
                for k in 0..=64 {
                    let x = (f64::from(k) / 64.0 - 0.5) * floor;
                    let surface = pool_surface(level, x, floor, time);
                    assert!(surface <= pool_ceiling(level), "{level} {x} {time}");
                    highest = highest.max((surface - level - POOL_WAVE * 1.6) / mound);
                }
            }
        }
    }
    // The bound is tight: real mounds come close to it.
    assert!(highest > 0.95, "highest mound {highest:.3} of the bound");
}

/// A small lamp keeps its pool at least [`MIN_POOL_PIXELS`] rows deep,
/// even at a mound's trough and with nearly all the wax afloat.
#[test]
fn coarse_grid_keeps_the_pool_two_rows_deep() {
    let mut world = World::bare(0.6);
    world.time = 30.0;
    let mut field = Field::default();
    field.prepare(&world, 1.0);
    let (cols, rows) = (12, 24);
    let mut grid = vec![Sample::default(); cols * rows];
    field.fill(&mut grid, cols, rows);
    for i in 0..cols {
        for j in rows - 2..rows {
            assert!(
                grid[j * cols + i].density >= SURFACE,
                "({i},{j}) is not pool"
            );
        }
    }
}

/// On a coarse grid a blob only a couple of pixels across draws as one
/// round bump, with no lobes to tear its outline: its wax is a solid,
/// convex patch whatever its id and the time.
#[test]
fn coarse_grid_draws_small_blobs_round() {
    let (cols, rows) = (24, 24);
    let mut grid = vec![Sample::default(); cols * rows];
    let mut field = Field::default();
    for id in 0..50 {
        let mut world = World::bare(1.0);
        // 2.4 px in radius: below where lobes start to show.
        world.add(0.0, 0.5, 0.1, 0.5);
        world.blobs[0].id = id;
        world.time = f64::from(id as u32) * 3.7;
        field.prepare(&world, 1.0);
        field.fill(&mut grid, cols, rows);
        // Each row and column of wax is one unbroken run, symmetric about
        // the centre (the blob sits on a pixel corner).
        let wax = |i: usize, j: usize| grid[j * cols + i].density >= SURFACE;
        for j in 0..rows - 4 {
            let row: Vec<usize> = (0..cols).filter(|&i| wax(i, j)).collect();
            if let (Some(&lo), Some(&hi)) = (row.first(), row.last()) {
                assert_eq!(hi - lo + 1, row.len(), "id {id}: row {j} is broken");
                assert_eq!(lo + hi, cols - 1, "id {id}: row {j} is lopsided");
            }
        }
    }
}

#[test]
fn interpolation_moves_between_steps() {
    let mut world = World::bare(1.0);
    world.add(0.0, 0.5, 0.08, 1.0);
    world.run(240);
    let mut field = Field::default();
    let y_at = |field: &Field| field.blobs[0].y;
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
        let mut world = World::new(7, aspect);
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
    let mut world = World::new(1, 1.0);
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
        let mut world = World::new(21, 1.5);
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
    let mut base = World::new(8, 1.2);
    base.run(600);
    let mut hot = World::new(8, 1.2);
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
    let mut world = World::new(7, 1.2);
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
    let mut world = World::new(3, 1.0);
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
        let mut world = World::new(seed, 1.1);
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
