//! Loop timing primitives, kept free of terminal code so they are testable:
//!
//! - [`FixedStep`]: accumulator that turns real elapsed time into a whole
//!   number of fixed simulation steps (the sim never sees a variable `dt`).
//! - [`FramePacer`]: when the next frame is due, for a target fps.
//! - [`FpsMeter`]: smoothed measured frame rate, for display.
//! - [`Quality`]: adaptive quality (§7): a coarser sample grid, then a
//!   lower frame rate, while frames run over budget.

use std::time::{Duration, Instant};

/// Fixed-timestep accumulator ("fix your timestep").
///
/// Sim time tracks real time × speed at any frame rate: a 10 fps frame at
/// ×4 runs 48 steps, a 1 fps frame 120 × speed. Only a genuine stall
/// (debugger, `SIGSTOP`, a frame far longer than any fps allows) is cut
/// short, so the app doesn't freeze while the sim catches up.
#[derive(Debug)]
pub struct FixedStep {
    dt: Duration,
    accumulator: Duration,
}

impl FixedStep {
    /// Real time longer than this between two frames is a stall: only this
    /// much of it is simulated and the rest is dropped. Above the slowest
    /// frame period (`--fps 1`) with room for a late frame.
    pub const STALL: Duration = Duration::from_millis(1500);

    pub fn new(hz: u32) -> Self {
        Self {
            dt: Duration::from_secs(1) / hz.max(1),
            accumulator: Duration::ZERO,
        }
    }

    /// The fixed step, in seconds.
    pub fn dt_secs(&self) -> f64 {
        self.dt.as_secs_f64()
    }

    /// Feed `elapsed` real time played at `speed`× (≥ 0); returns how many
    /// fixed steps to run now.
    pub fn advance(&mut self, elapsed: Duration, speed: f64) -> u32 {
        let elapsed = elapsed.min(Self::STALL);
        self.accumulator += elapsed.mul_f64(speed.max(0.0));
        let steps = (self.accumulator.as_nanos() / self.dt.as_nanos()) as u32;
        self.accumulator -= self.dt * steps;
        steps
    }

    /// How far (0..1) real time is into the next, not-yet-simulated step.
    /// Renderers can use it to interpolate between sim states.
    pub fn alpha(&self) -> f64 {
        self.accumulator.as_secs_f64() / self.dt.as_secs_f64()
    }
}

/// Schedules frame deadlines for a target frame rate.
#[derive(Debug)]
pub struct FramePacer {
    period: Duration,
    next: Instant,
}

impl FramePacer {
    pub fn new(fps: u32, now: Instant) -> Self {
        let period = Duration::from_secs(1) / fps.max(1);
        Self {
            period,
            next: now + period,
        }
    }

    /// When the next frame should be drawn.
    pub fn deadline(&self) -> Instant {
        self.next
    }

    /// Mark a frame as drawn. Deadlines stay on a fixed grid while we keep
    /// up. Early input frames keep their next deadline; late frames skip
    /// expired grid slots without adding a full period after the overrun.
    pub fn frame_done(&mut self, now: Instant) {
        if now >= self.next {
            let slots = (now - self.next).as_nanos() / self.period.as_nanos() + 1;
            self.next += Duration::from_nanos((slots * self.period.as_nanos()) as u64);
        }
    }
}

/// When a frame for input handled at `now` may draw: at once, unless the
/// last frame started under one period (at `fps`) ago, then one period
/// after it. So a stream of input faster than the frame rate can't draw
/// faster than it, and still shows within one frame. Never after
/// `deadline`, the frame that was due anyway.
pub fn input_frame_at(
    last_frame: Option<Instant>,
    fps: u32,
    now: Instant,
    deadline: Instant,
) -> Instant {
    let soonest = last_frame.map_or(now, |at| at + Duration::from_secs(1) / fps.max(1));
    soonest.max(now).min(deadline)
}

/// Exponentially smoothed frames-per-second.
#[derive(Debug, Default)]
pub struct FpsMeter {
    last: Option<Instant>,
    fps: f64,
}

impl FpsMeter {
    const SMOOTHING: f64 = 0.1;

    pub fn tick(&mut self, now: Instant) {
        if let Some(last) = self.last.replace(now) {
            let dt = now.duration_since(last).as_secs_f64();
            if dt > 0.0 {
                let sample = 1.0 / dt;
                self.fps = if self.fps == 0.0 {
                    sample
                } else {
                    self.fps + (sample - self.fps) * Self::SMOOTHING
                };
            }
        }
    }

    pub fn fps(&self) -> f64 {
        self.fps
    }
}

/// Adaptive quality (docs/design.md §7), silent and never touching the
/// user's settings:
///
/// 1. Frame time (a moving average) over 80 % of the budget for 2 s: the
///    lamp samples at half resolution per axis and upsamples.
/// 2. Still over: the frame rate halves (60 → 30, never below 30).
/// 3. Under 40 % for 5 s: one step back. The 40 % is of the budget the
///    step *returns to* (60 fps's, coming back from 30), so a recovery
///    can't put the frame straight back over the line.
///
/// It must never visibly oscillate: a step that comes back and is lost
/// again soon after doubles how long the next recovery waits (up to
/// [`Quality::MAX_HOLD`]), until the workload (window size × style grid)
/// changes.
#[derive(Debug, Clone)]
pub struct Quality {
    level: u8,
    ema_ms: Option<f64>,
    over_since: Option<Instant>,
    under_since: Option<Instant>,
    recovered_at: Option<Instant>,
    hold: Duration,
    workload: usize,
}

impl Default for Quality {
    fn default() -> Self {
        Quality {
            level: 0,
            ema_ms: None,
            over_since: None,
            under_since: None,
            recovered_at: None,
            hold: Self::UNDER_FOR,
            workload: 0,
        }
    }
}

impl Quality {
    pub const OVER: f64 = 0.8;
    pub const UNDER: f64 = 0.4;
    pub const OVER_FOR: Duration = Duration::from_secs(2);
    pub const UNDER_FOR: Duration = Duration::from_secs(5);
    pub const MAX_HOLD: Duration = Duration::from_secs(300);
    /// Lowest frame rate adaptation alone goes to.
    pub const MIN_FPS: u32 = 30;
    /// Moving-average time constant.
    const TAU: f64 = 0.5;

    /// Whether the lamp samples at a reduced grid.
    pub fn reduced_grid(&self) -> bool {
        self.level >= 1
    }

    /// Whether anything is held back.
    pub fn degraded(&self) -> bool {
        self.level > 0
    }

    /// The frame rate to aim for, given the user's.
    pub fn fps(&self, fps: u32) -> u32 {
        Self::fps_at(self.level, fps)
    }

    fn fps_at(level: u8, fps: u32) -> u32 {
        if level >= 2 {
            (fps / 2).max(Self::MIN_FPS).min(fps)
        } else {
            fps
        }
    }

    fn max_level(fps: u32) -> u8 {
        if Self::fps_at(2, fps) < fps { 2 } else { 1 }
    }

    /// What's being drawn changed (samples per frame): past flapping says
    /// nothing about it, so recoveries wait the normal time again.
    pub fn set_workload(&mut self, workload: usize) {
        if workload != self.workload {
            self.workload = workload;
            self.hold = Self::UNDER_FOR;
            self.recovered_at = None;
        }
    }

    /// Feed one frame: it took `frame_ms` of work, `dt` after the last one,
    /// against the user's target `fps`.
    pub fn frame(&mut self, frame_ms: f64, dt: Duration, fps: u32, now: Instant) {
        let ema = match self.ema_ms {
            None => frame_ms,
            Some(ema) => {
                let k = 1.0 - (-dt.as_secs_f64() / Self::TAU).exp();
                ema + (frame_ms - ema) * k
            }
        };
        self.ema_ms = Some(ema);
        let budget = |level: u8| 1000.0 / f64::from(Self::fps_at(level, fps).max(1));

        let over = ema > Self::OVER * budget(self.level) && self.level < Self::max_level(fps);
        let under = self.level > 0 && ema < Self::UNDER * budget(self.level - 1);
        let since = |t: &mut Option<Instant>, on: bool| {
            if on {
                Some(now - *t.get_or_insert(now))
            } else {
                *t = None;
                None
            }
        };
        if since(&mut self.over_since, over).is_some_and(|d| d >= Self::OVER_FOR) {
            // Lost again soon after coming back: wait longer next time.
            if self
                .recovered_at
                .is_some_and(|at| now - at < 2 * self.hold + Self::OVER_FOR)
            {
                self.hold = (self.hold * 2).min(Self::MAX_HOLD);
            }
            self.step(self.level + 1);
        } else if since(&mut self.under_since, under).is_some_and(|d| d >= self.hold) {
            self.recovered_at = Some(now);
            self.step(self.level - 1);
        }
    }

    fn step(&mut self, level: u8) {
        self.level = level;
        self.over_since = None;
        self.under_since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn fixed_step_accumulates_partial_steps() {
        let mut clock = FixedStep::new(100); // 10ms steps
        assert_eq!(clock.advance(MS * 4, 1.0), 0);
        assert_eq!(clock.advance(MS * 4, 1.0), 0);
        assert_eq!(clock.advance(MS * 4, 1.0), 1); // 12ms total
        assert!((clock.alpha() - 0.2).abs() < 1e-9);
        assert_eq!(clock.advance(MS * 25, 1.0), 2); // 27ms banked
    }

    /// Sim time ÷ real time is the speed factor at any frame rate (lava-ebq.8:
    /// a per-frame step cap used to play 10 fps at ×0.67).
    #[test]
    fn sim_time_tracks_real_time_at_any_fps() {
        for fps in [1, 2, 10, 30, 60, 144] {
            for speed in [0.25, 1.0, 4.0] {
                let mut clock = FixedStep::new(120);
                let frame = Duration::from_secs(1) / fps;
                let frames = 10 * fps; // 10 s of real time
                let steps: u32 = (0..frames).map(|_| clock.advance(frame, speed)).sum();
                let sim = f64::from(steps) * clock.dt_secs();
                let ratio = sim / (10.0 * speed);
                assert!(
                    (ratio - 1.0).abs() < 0.02,
                    "{fps} fps ×{speed}: sim/real = {ratio:.3}"
                );
            }
        }
    }

    #[test]
    fn fixed_step_drops_backlog_after_stall() {
        let mut clock = FixedStep::new(100);
        // A 5 s stall only simulates the stall limit...
        let steps = clock.advance(Duration::from_secs(5), 1.0);
        assert_eq!(steps, (FixedStep::STALL.as_millis() / 10) as u32);
        // ...and then it's back to normal, with no backlog left.
        assert_eq!(clock.advance(MS * 10, 1.0), 1);
        // A slow-but-honest frame (1 fps at ×4) is never cut.
        assert_eq!(clock.advance(Duration::from_secs(1), 4.0), 400);
    }

    #[test]
    fn pacer_keeps_grid_and_skips_expired_slots() {
        let t0 = Instant::now();
        let period = Duration::from_secs(1) / 50; // 20ms
        let mut pacer = FramePacer::new(50, t0);
        assert_eq!(pacer.deadline(), t0 + period);

        pacer.frame_done(t0 + MS * 21); // slightly late: stay on the grid
        assert_eq!(pacer.deadline(), t0 + period * 2);

        pacer.frame_done(t0 + MS * 100); // very late: skip expired slots
        assert_eq!(pacer.deadline(), t0 + MS * 100 + period);
    }

    #[test]
    fn early_input_frame_does_not_skip_the_next_deadline() {
        let t0 = Instant::now();
        let mut pacer = FramePacer::new(60, t0);
        let due = pacer.deadline();
        pacer.frame_done(t0 + MS);
        assert_eq!(pacer.deadline(), due);
    }

    #[test]
    fn late_frame_skips_missed_deadlines_without_shifting_the_grid() {
        let t0 = Instant::now();
        let mut pacer = FramePacer::new(50, t0);
        pacer.frame_done(t0 + MS * 45);
        assert_eq!(pacer.deadline(), t0 + MS * 60);
        pacer.frame_done(t0 + MS * 123);
        assert_eq!(pacer.deadline(), t0 + MS * 140);
    }

    #[test]
    fn input_frames_wait_out_the_period_but_never_the_deadline() {
        let t0 = Instant::now();
        let far = t0 + Duration::from_secs(60);
        // Nothing drawn yet, or the last frame a period ago: at once.
        assert_eq!(input_frame_at(None, 50, t0, far), t0);
        assert_eq!(
            input_frame_at(Some(t0), 50, t0 + MS * 25, far),
            t0 + MS * 25
        );
        // Within the period: one period after the last frame.
        assert_eq!(input_frame_at(Some(t0), 50, t0 + MS * 5, far), t0 + MS * 20);
        // A deadline sooner than that wins; one already past draws now.
        assert_eq!(
            input_frame_at(Some(t0), 50, t0 + MS * 5, t0 + MS * 10),
            t0 + MS * 10
        );
        assert!(input_frame_at(Some(t0), 50, t0 + MS * 5, t0) <= t0 + MS * 5);
    }

    #[test]
    fn fps_meter_converges() {
        let t0 = Instant::now();
        let mut meter = FpsMeter::default();
        for i in 0..200 {
            meter.tick(t0 + Duration::from_millis(16) * i);
        }
        assert!((meter.fps() - 62.5).abs() < 0.5);
    }

    /// Run `quality` for `secs` at 60 fps frames costing `cost(t)` ms.
    fn run(q: &mut Quality, t0: &mut Instant, secs: f64, cost: impl Fn(&Quality) -> f64) {
        let frames = (secs * 60.0) as u32;
        let dt = Duration::from_secs_f64(1.0 / 60.0);
        for _ in 0..frames {
            *t0 += dt;
            let ms = cost(q);
            q.frame(ms, dt, 60, *t0);
        }
    }

    #[test]
    fn quality_drops_grid_then_fps_and_recovers_in_reverse() {
        let mut q = Quality::default();
        let mut t = Instant::now();
        run(&mut q, &mut t, 1.0, |_| 15.0);
        assert!(!q.degraded(), "not before 2 s");
        run(&mut q, &mut t, 1.5, |_| 15.0);
        assert!(q.reduced_grid() && q.fps(60) == 60, "grid first");
        run(&mut q, &mut t, 2.5, |_| 15.0);
        assert_eq!(q.fps(60), 30, "then fps");
        run(&mut q, &mut t, 10.0, |_| 15.0);
        assert_eq!(q.fps(60), 30, "never below 30");
        // Cheap again: back in reverse, 5 s a step.
        run(&mut q, &mut t, 4.0, |_| 2.0);
        assert_eq!(q.fps(60), 30);
        run(&mut q, &mut t, 2.0, |_| 2.0);
        assert!(q.fps(60) == 60 && q.reduced_grid());
        run(&mut q, &mut t, 6.0, |_| 2.0);
        assert!(!q.degraded());
    }

    #[test]
    fn quality_holds_in_the_dead_band() {
        let mut q = Quality::default();
        let mut t = Instant::now();
        run(&mut q, &mut t, 3.0, |_| 14.0);
        assert!(q.reduced_grid());
        // 10 ms is under 80 % of 16.7 ms but over 40 %: stay put.
        run(&mut q, &mut t, 60.0, |_| 10.0);
        assert!(q.reduced_grid() && q.fps(60) == 60);
    }

    #[test]
    fn quality_never_flaps_between_steps() {
        // Full grid costs 15 ms (over), reduced 5 ms (under 40 %): the
        // naive controller would toggle every ~7 s forever.
        let mut q = Quality::default();
        let mut t = Instant::now();
        let mut switches = Vec::new();
        let mut was = q.reduced_grid();
        let dt = Duration::from_secs_f64(1.0 / 60.0);
        for frame in 0..(600 * 60) {
            t += dt;
            let ms = if q.reduced_grid() { 5.0 } else { 15.0 };
            q.frame(ms, dt, 60, t);
            if q.reduced_grid() != was {
                switches.push(frame / 60);
                was = q.reduced_grid();
            }
        }
        // Backoff: each retry waits twice as long (5, 10, 20 … 300 s).
        let late = switches.iter().filter(|&&s| s >= 180).count();
        assert!(late <= 2, "switches at {switches:?} s");
        assert_eq!(q.hold, Quality::MAX_HOLD);
        // A new window size starts over.
        q.set_workload(12345);
        assert_eq!(q.hold, Quality::UNDER_FOR);
    }

    #[test]
    fn quality_at_30_fps_only_drops_the_grid() {
        let mut q = Quality::default();
        let mut t = Instant::now();
        let dt = Duration::from_secs_f64(1.0 / 30.0);
        for _ in 0..300 {
            t += dt;
            q.frame(40.0, dt, 30, t);
        }
        assert!(q.reduced_grid());
        assert_eq!(q.fps(30), 30);
    }
}
