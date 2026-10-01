//! Loop timing primitives, kept free of terminal code so they are testable:
//!
//! - [`FixedStep`]: accumulator that turns real elapsed time into a whole
//!   number of fixed simulation steps (the sim never sees a variable `dt`).
//! - [`FramePacer`]: when the next frame is due, for a target fps.
//! - [`FpsMeter`]: smoothed measured frame rate, for display.

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
    /// up; if we fall behind we resync to `now` rather than bursting frames.
    pub fn frame_done(&mut self, now: Instant) {
        self.next += self.period;
        if self.next <= now {
            self.next = now + self.period;
        }
    }
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
    fn pacer_keeps_grid_and_resyncs_when_late() {
        let t0 = Instant::now();
        let period = Duration::from_secs(1) / 50; // 20ms
        let mut pacer = FramePacer::new(50, t0);
        assert_eq!(pacer.deadline(), t0 + period);

        pacer.frame_done(t0 + MS * 21); // slightly late: stay on the grid
        assert_eq!(pacer.deadline(), t0 + period * 2);

        pacer.frame_done(t0 + MS * 100); // very late: resync
        assert_eq!(pacer.deadline(), t0 + MS * 100 + period);
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
}
