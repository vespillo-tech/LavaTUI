//! Loop timing primitives, kept free of terminal code so they are testable:
//!
//! - [`FixedStep`]: accumulator that turns real elapsed time into a whole
//!   number of fixed simulation steps (the sim never sees a variable `dt`).
//! - [`FramePacer`]: when the next frame is due, for a target fps.
//! - [`FpsMeter`]: smoothed measured frame rate, for display.

use std::time::{Duration, Instant};

/// Fixed-timestep accumulator ("fix your timestep").
#[derive(Debug)]
pub struct FixedStep {
    dt: Duration,
    accumulator: Duration,
    max_steps: u32,
}

impl FixedStep {
    /// Most steps run per `advance`. After a long stall (suspend, debugger)
    /// the backlog is dropped instead of freezing the app to catch up.
    const MAX_STEPS_PER_ADVANCE: u32 = 8;

    pub fn new(hz: u32) -> Self {
        Self {
            dt: Duration::from_secs(1) / hz.max(1),
            accumulator: Duration::ZERO,
            max_steps: Self::MAX_STEPS_PER_ADVANCE,
        }
    }

    /// The fixed step, in seconds.
    pub fn dt_secs(&self) -> f64 {
        self.dt.as_secs_f64()
    }

    /// Feed `elapsed` real time; returns how many fixed steps to run now.
    pub fn advance(&mut self, elapsed: Duration) -> u32 {
        self.accumulator += elapsed;
        let mut steps = 0;
        while self.accumulator >= self.dt {
            if steps == self.max_steps {
                self.accumulator = Duration::ZERO;
                break;
            }
            self.accumulator -= self.dt;
            steps += 1;
        }
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
        assert_eq!(clock.advance(MS * 4), 0);
        assert_eq!(clock.advance(MS * 4), 0);
        assert_eq!(clock.advance(MS * 4), 1); // 12ms total
        assert!((clock.alpha() - 0.2).abs() < 1e-9);
        assert_eq!(clock.advance(MS * 25), 2); // 27ms banked
    }

    #[test]
    fn fixed_step_drops_backlog_after_stall() {
        let mut clock = FixedStep::new(100);
        assert_eq!(clock.advance(Duration::from_secs(5)), 8);
        assert_eq!(clock.alpha(), 0.0);
        assert_eq!(clock.advance(MS * 10), 1);
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
