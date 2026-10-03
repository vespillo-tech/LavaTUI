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
            (b.x - world.wall_centre).abs() <= half + 0.02 && (0.0..=1.0).contains(&b.y),
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
        1.0 - pool_surface(world.pool_level(), 0.0, world.floor(), world.time) + 0.01,
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
                    let mounds = Mounds {
                        centre: 0.0,
                        width: floor,
                        humps: (floor / POOL_HUMP).round().max(1.0),
                    };
                    let floor = Floor {
                        width: floor,
                        from: mounds,
                        to: mounds,
                        blend: 1.0,
                        ..Floor::default()
                    };
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

/// The end of one step and the start of the next are the same moment, so
/// they must draw the same wax, whatever the step did: merges, splits,
/// buds, melts, a pool running dry (ghosts, fades and every interpolated
/// part of the pose).
#[test]
fn steps_join_up_through_every_event() {
    let (cols, rows) = (48, 36);
    let mut events = Stats::default();
    for (seed, aspect, top) in [(5, 1.4, false), (9, 0.6, false), (3, 1.0, true)] {
        let mut world = World::new(seed, aspect);
        world.set_heat(5);
        world.set_top_wax(top);
        world.prewarm(600, DT);
        if top {
            // Just under the layer, two blobs start melting into it: one
            // all of it, the other (too big for it) a share before it
            // pulls away; the bulges they leave sag into drops.
            world.cap_area = CAP_KEEP * world.wall_width;
            for (x, radius, share) in [(0.2, 0.06, f64::INFINITY), (-0.2, 0.16, 0.4)] {
                world.add(x, 0.97 - radius, radius, 0.56);
                let blob = world.blobs.last_mut().unwrap();
                let left = share * blob.area();
                (blob.phase, blob.end) = (Phase::Melting { left }, End::Top);
            }
            world.wax_target = world.wax_area();
        }
        let mut field = Field::default();
        let mut end = vec![Sample::default(); cols * rows];
        let mut start = end.clone();
        let before = world.stats();
        // The lamp at 20 rows, a side panel coming and going, lamp only
        // (a row more, no panel) and a panel under it, each in turn.
        let lamp = |cols: f64, rows: f64| Frame {
            x: 0.0,
            y: 0.0,
            width: cols * aspect * 20.0,
            height: rows,
        };
        let frames = [
            lamp(1.0, 20.0),
            lamp(0.7, 20.0),
            lamp(1.0, 21.0),
            lamp(1.0, 14.0),
        ];
        world.set_frame(frames[0]);
        for i in 0..7200 {
            // The top layer goes and comes back.
            if top && (i == 4800 || i == 5400) {
                world.set_top_wax(i == 5400);
            }
            if i % 450 == 0 {
                world.set_frame(frames[i / 450 % frames.len()]);
            }
            field.prepare(&world, 1.0);
            field.fill(&mut end, cols, rows);
            world.step(DT);
            field.prepare(&world, 0.0);
            field.fill(&mut start, cols, rows);
            let worst = end
                .iter()
                .zip(&start)
                .map(|(a, b)| (a.density - b.density).abs())
                .fold(0.0, f32::max);
            assert!(worst < 1e-3, "density jumps {worst} at t={:.3}", world.time);
        }
        let after = world.stats();
        events.budded += after.budded - before.budded;
        events.merged += after.merged - before.merged;
        events.split += after.split - before.split;
        events.melted += after.melted - before.melted;
        events.dripped += after.dripped - before.dripped;
        events.capped += after.capped - before.capped;
        events.pinched += after.pinched - before.pinched;
    }
    assert!(
        events.budded > 0 && events.merged > 0 && events.split > 0 && events.melted > 0,
        "{events:?}"
    );
    assert!(
        events.dripped > 0 && events.capped > 0 && events.pinched > 0,
        "{events:?}"
    );
}

/// A lamp at `cells` (x, y, width, height; cells half as wide as tall),
/// its [`Frame`].
fn cells_frame((x, y, w, h): (u16, u16, u16, u16)) -> Frame {
    Frame {
        x: f64::from(x) / 2.0,
        y: f64::from(y),
        width: f64::from(w) / 2.0,
        height: f64::from(h),
    }
}

/// The field drawn as half blocks on a lamp at `cells`: density by
/// screen pixel (column, row × 2).
fn on_screen(field: &Field, cells: (u16, u16, u16, u16)) -> Vec<((u16, u16), f32)> {
    let (x, y, w, h) = cells;
    let (cols, rows) = (usize::from(w), usize::from(h) * 2);
    let mut samples = vec![Sample::default(); cols * rows];
    field.fill(&mut samples, cols, rows);
    let at = |i: usize| (x + (i % cols) as u16, y * 2 + (i / cols) as u16);
    samples
        .iter()
        .enumerate()
        .map(|(i, s)| (at(i), s.density))
        .collect()
}

/// When the lamp changes size or place on screen, every cell it kept shows
/// the wax it showed; then the view settles on the whole lamp, keeping in
/// place the side edge that stayed put (the middle when both or neither
/// did) and the top, and the walls follow.
#[test]
fn resizes_keep_the_wax_on_screen() {
    // (from, to) in cells, and the point across the new lamp (0 its left
    // edge, 1 its right) that keeps its place.
    let cases = [
        // A side panel comes, goes (100 columns, 30 rows).
        ((0, 0, 100, 30), (0, 0, 70, 30), 0.0),
        ((0, 0, 70, 30), (0, 0, 100, 30), 0.0),
        // Lamp only: the panel and the status row go, come back.
        ((0, 0, 70, 29), (0, 0, 100, 30), 0.0),
        ((0, 0, 100, 30), (0, 0, 70, 29), 0.0),
        // Portrait: a panel under the lamp.
        ((0, 0, 40, 60), (0, 0, 40, 42), 0.5),
        // A panel on the left; a tiny window.
        ((0, 0, 100, 30), (30, 0, 70, 30), 1.0),
        ((0, 0, 24, 6), (0, 0, 16, 5), 0.0),
    ];
    for (from, to, keep) in cases {
        let mut world = World::new(5, f64::from(from.2) / f64::from(from.3) / 2.0);
        world.set_frame(cells_frame(from));
        world.prewarm(600, DT);
        let mut field = Field::default();
        field.prepare(&world, 0.5);
        let before: std::collections::HashMap<_, _> = on_screen(&field, from).into_iter().collect();
        world.set_frame(cells_frame(to));
        field.prepare(&world, 0.5);
        let mut shared = 0;
        for (at, d) in on_screen(&field, to) {
            if let Some(was) = before.get(&at) {
                assert!(
                    (d - was).abs() < 1e-3,
                    "{from:?} -> {to:?} at {at:?}: {was} -> {d}"
                );
                shared += 1;
            }
        }
        assert!(shared > 0, "{from:?} -> {to:?}");

        let pinned = world.view;
        world.run(120 * 6);
        let aim = world.aim;
        assert_eq!(world.view, aim, "{from:?} -> {to:?}: settled");
        assert_close(world.wall_width, aim.width, 1e-6);
        assert_close(world.wall_centre, aim.centre(), 1e-6);
        assert_eq!((aim.y, aim.height), (0.0, 1.0));
        let at = |v: View| v.x + keep * v.width;
        assert_close(at(aim), at(pinned), 1e-9);
        assert_close(aim.top(), pinned.top(), 1e-9);
        assert_sane(&world);
    }
}

/// The view glides from rest to rest: no frame jumps, and it neither
/// overshoots nor stops short.
#[test]
fn view_glides_without_a_jerk() {
    let mut world = World::new(5, 100.0 / 60.0);
    world.set_frame(cells_frame((0, 0, 100, 30)));
    world.set_frame(cells_frame((0, 0, 100, 20)));
    let (start, aim) = (world.view, world.aim);
    let mut steps = Vec::new();
    for _ in 0..240 {
        let was = world.view.height;
        world.step(DT);
        steps.push(world.view.height - was);
        let h = world.view.height;
        assert!((start.height..=aim.height).contains(&h), "{h}");
    }
    assert_eq!(world.view, aim);
    // It starts gently: the first step is a small part of the fastest.
    let fastest = steps.iter().fold(0.0_f64, |a, s| a.max(s.abs()));
    assert!(steps[0].abs() < 0.01 * fastest, "{} of {fastest}", steps[0]);
}

/// `cargo test --release -- --ignored --nocapture bench_fill`
/// (`TOP_WAX=1`: with the top layer)
#[test]
#[ignore = "benchmark"]
fn bench_fill() {
    use std::time::Instant;
    for (aspect, cols, rows) in [(0.83, 200, 120), (1.5, 300, 200), (1.33, 80, 48)] {
        let mut world = World::new(7, aspect);
        world.set_top_wax(std::env::var("TOP_WAX").is_ok_and(|s| s == "1"));
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

/// Long, hot runs including narrow/wide resize bursts: correlate slow
/// steps with topology events rather than assuming pairwise work hitches.
#[test]
#[ignore = "simulation outlier benchmark"]
fn bench_step_outliers() {
    use std::time::Instant;
    for aspect in [0.4, 80.0 / 48.0, 160.0 / 80.0, 250.0 / 140.0, 10.0] {
        let mut world = World::new(7, aspect);
        world.set_heat(5);
        world.prewarm(7200, DT);
        let initial = world.stats();
        let mut times = Vec::with_capacity(72_000);
        let (mut max_blobs, mut event_max, mut quiet_max) = (0, 0, 0);
        for i in 0..72_000 {
            if i % 7200 == 0 {
                world.set_aspect(if i % 14_400 == 0 { 0.4 } else { aspect });
            }
            if i % 1200 == 0 {
                world.heat_pulse(0.5, 0.98);
            }
            let before = world.stats();
            let start = Instant::now();
            world.step(DT);
            let ns = start.elapsed().as_nanos() as u64;
            times.push(ns);
            max_blobs = max_blobs.max(world.blobs.len());
            if before != world.stats() {
                event_max = event_max.max(ns);
            } else {
                quiet_max = quiet_max.max(ns);
            }
        }
        times.sort_unstable();
        println!(
            "aspect {aspect:.3} step mean/p99/max us {:.2}/{:.2}/{:.2}, event/quiet max {:.2}/{:.2}, max_blobs {max_blobs}, events {:?}",
            times.iter().sum::<u64>() as f64 / times.len() as f64 / 1000.0,
            times[times.len() * 99 / 100] as f64 / 1000.0,
            times[times.len() - 1] as f64 / 1000.0,
            event_max as f64 / 1000.0,
            quiet_max as f64 / 1000.0,
            (
                world.stats().budded - initial.budded,
                world.stats().merged - initial.merged,
                world.stats().split - initial.split,
                world.stats().melted - initial.melted
            ),
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

// --- top layer -------------------------------------------------------------

/// Turned on, the top layer eases in from the pool; turned off, it thins
/// back into it. Wax is conserved every step and its shown depth never
/// jumps.
#[test]
fn top_wax_eases_in_and_out_conserving_wax() {
    let mut world = World::new(4, 1.3);
    world.prewarm(600, DT);
    // (A pool at its least has none to spare: the layer then fills as
    // blobs melt back into it.)
    world.pool_area += 5.0 * CAP_KEEP * world.wall_width;
    world.wax_target = world.wax_area();
    let wax = world.wax_area();
    let mut last = world.cap_depth();
    assert_eq!(last, 0.0);
    let mut biggest_step: f64 = 0.0;
    let mut run = |world: &mut World, secs: f64| {
        for _ in 0..(secs * 120.0) as u32 {
            world.step(DT);
            assert_close(world.wax_area(), wax, 1e-9);
            biggest_step = biggest_step.max((world.cap_depth() - last).abs());
            last = world.cap_depth();
        }
    };
    world.set_top_wax(true);
    run(&mut world, 0.5);
    let early = world.cap_depth();
    assert!(early > 0.0 && early < 0.5 * CAP_KEEP, "eases in: {early}");
    run(&mut world, CAP_FADE);
    assert_eq!(world.cap_on, 1.0);
    assert!(world.cap_depth() > 0.95 * CAP_KEEP, "{}", world.cap_depth());

    world.set_top_wax(false);
    run(&mut world, 0.5);
    assert!(
        world.cap_depth() > 0.5 * CAP_KEEP,
        "eases out: {}",
        world.cap_depth()
    );
    run(&mut world, CAP_FADE);
    assert_eq!(world.cap_depth(), 0.0);
    run(&mut world, 6.0);
    assert_eq!(world.cap_area, 0.0, "all back in the pool");
    assert!(
        world
            .blobs
            .iter()
            .all(|b| !b.at_top() || b.phase == Phase::Free)
    );
    assert!(
        biggest_step < 0.02 * CAP_KEEP,
        "depth stepped by {biggest_step}"
    );
}

/// With the top layer on, rising blobs give it wax and it lets drops fall,
/// and it stays a thin layer at every aspect, even after the lamp narrows.
#[test]
fn top_wax_takes_wax_and_drips_staying_thin() {
    let mut events = Stats::default();
    for (seed, aspect) in [(5, 1.4), (3, 2.5), (7, 1.0), (2, 0.4)] {
        let mut world = World::new(seed, aspect);
        world.set_top_wax(true);
        let mut deepest: f64 = 0.0;
        // The lamp halves at 4 minutes; its spare wax melts away within
        // about two more (as in `resize_eases_walls_without_teleporting`).
        for step in 0..120 * 360 {
            if step == 120 * 240 {
                world.set_aspect(0.5 * aspect);
            }
            world.step(DT);
            if step > 120 * 3 {
                deepest = deepest.max(world.cap_depth());
                assert!(world.cap_depth() >= 0.9 * CAP_KEEP, "{seed}: ran thin");
            }
        }
        let stats = world.stats();
        println!("seed {seed}: deepest {deepest:.4}, {stats:?}");
        // Narrowing the lamp thickens it for a moment.
        assert!(deepest < 1.6 * CAP_FULL, "seed {seed}: {deepest}");
        assert!(world.cap_depth() <= 1.01 * CAP_FULL, "seed {seed}");
        (events.dripped, events.pinched) = (
            events.dripped + stats.dripped,
            events.pinched + stats.pinched,
        );
        assert_close(world.wax_area(), FILL * 0.5 * aspect, 0.02);
        assert_sane(&world);
    }
    // About one of each a minute (fewer in a narrow lamp, whose blobs
    // seldom reach the top).
    assert!(events.dripped >= 12 && events.pinched >= 12, "{events:?}");
}

/// Pressed against the top layer, a big blob that has cooled sticks; a
/// small, still-warm one doesn't (it touches and turns back).
#[test]
fn cool_heavy_blobs_stick_and_small_warm_ones_turn_back() {
    let world = World::new(3, 1.0);
    let typical = world.typical_radius();
    let mut blob = world.blobs[0].clone();
    let mut rate = |radius: f64, temp: f64| {
        (blob.radius, blob.temp) = (radius, temp);
        stick_rate(&blob, typical)
    };
    assert_eq!(rate(1.5 * typical, 0.5), STICK_RATE);
    assert_eq!(rate(0.3 * typical, 0.5), 0.0, "small");
    assert_eq!(rate(1.5 * typical, 0.7), 0.0, "warm");
    assert!(rate(typical, 0.57) < rate(1.5 * typical, 0.55));
}

/// A blob melting into the top layer seeps in slowly and leaves a warm
/// bulge where it joined, which cools, spreads and evens out (or sags
/// into a drop); the layer keeps the wax.
#[test]
fn melting_into_the_top_layer_leaves_a_warm_bulge_that_evens_out() {
    let mut world = World::new(3, 1.0);
    world.blobs.clear();
    world.pool_area = world.wax_target;
    world.spawn_timer = 1e9;
    world.set_top_wax(true);
    world.run(120 * 3);
    world.drip_timer = 1e9;
    let id = world.add(-0.2, 0.9, 0.06, 0.6);
    let blob = world.blobs.last_mut().unwrap();
    (blob.phase, blob.end) = (Phase::MELTING, End::Top);
    world.wax_target = world.wax_area();
    let wax = world.wax_area();
    let cap = world.cap_total();
    world.run(120 * 2);
    let blob = world
        .blobs
        .iter()
        .find(|b| b.id == id)
        .expect("still seeping in after 2 s");
    assert!(blob.radius < 0.06 && blob.radius > 0.03, "{}", blob.radius);
    let warm = world.lumps[0].shape;
    assert!(
        warm.temp > CAP_TEMP + 0.1,
        "the bulge is warm: {}",
        warm.temp
    );
    world.run(120 * 30);
    assert!(world.blobs.iter().all(|b| b.id != id), "melted in");
    assert_eq!(world.stats().capped, 1);
    assert!(
        world
            .lumps
            .iter()
            .all(|l| l.shape.temp < warm.temp && l.shape.width > warm.width)
    );
    assert!(world.cap_total() > cap, "{} vs {cap}", world.cap_total());
    assert_close(world.wax_area(), wax, 1e-9);
}

#[test]
fn same_seed_same_lamp_with_top_wax() {
    let play = |seed| {
        let mut world = World::new(seed, 1.1);
        world.set_top_wax(true);
        world.run(120 * 90);
        world.reseed(seed ^ 0xABCD);
        world.run(1500);
        world.set_top_wax(false);
        world.run(500);
        world
    };
    let (a, b) = (play(42), play(42));
    assert_eq!(a.blobs, b.blobs);
    assert_eq!(a.pool_area.to_bits(), b.pool_area.to_bits());
    assert_eq!(a.cap_area.to_bits(), b.cap_area.to_bits());
}

/// How often the top layer takes wax and drips, and how deep it gets.
#[test]
#[ignore = "tuning report"]
fn top_wax_report() {
    for (seed, aspect) in [(5, 1.4), (9, 0.6), (3, 2.5), (7, 1.0)] {
        let mut world = World::new(seed, aspect);
        world.set_top_wax(true);
        let mut depths = Vec::new();
        let (mut touched, mut joined) = (
            std::collections::BTreeSet::new(),
            std::collections::BTreeSet::new(),
        );
        let mut sag_drops = 0;
        for _ in 0..600 {
            for _ in 0..120 {
                let drops_before: Vec<u64> = (world.blobs.iter())
                    .filter(|b| b.at_top() && matches!(b.phase, Phase::Budding { .. }))
                    .map(|b| b.id)
                    .collect();
                world.step(DT);
                for b in &world.blobs {
                    let top = b.y + b.radius * b.stretch;
                    if b.phase == Phase::Free
                        && top > world.cap_under(b.x) - CAP_TOUCH
                        && b.cooldown <= 0.0
                        && b.vy > -0.005
                    {
                        touched.insert(b.id);
                    }
                    if b.at_top() && matches!(b.phase, Phase::Melting { .. }) {
                        joined.insert(b.id);
                    }
                    if b.at_top()
                        && matches!(b.phase, Phase::Budding { .. })
                        && !drops_before.contains(&b.id)
                        && world.lumps.iter().any(|l| l.follow == b.id)
                    {
                        sag_drops += 1;
                    }
                }
            }
            depths.push(world.cap_mean_depth());
        }
        println!(
            "touched {} joined {} sag drops {sag_drops}",
            touched.len(),
            joined.len()
        );
        let lo = depths.iter().copied().fold(f64::MAX, f64::min);
        let hi = depths.iter().copied().fold(0.0, f64::max);
        let mean = depths.iter().sum::<f64>() / depths.len() as f64;
        println!(
            "seed {seed} aspect {aspect}: {:?}, depth {lo:.4}..{hi:.4} mean {mean:.4}",
            world.stats()
        );
    }
}
