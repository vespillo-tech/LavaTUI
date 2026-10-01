//! Pomodoro timer as a pure state machine. Every method that depends on time
//! takes `now: Instant` from the caller, so tests drive it with synthetic
//! instants and the UI passes the real one.
//!
//! ```text
//!            start / toggle            pause / toggle
//!   Idle ───────────────────▶ Running ◀──────────────▶ Paused
//!    ▲                           │      resume / toggle
//!    └──────── reset ────────────┴── (from any state)
//!
//! Phases: focus → break → focus → … → focus → long break (every `cycles`th)
//! A phase ends when `tick` sees its time is up, or on `skip`.
//! ```

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    /// Lowercase label for the panel and toasts.
    pub fn label(self) -> &'static str {
        match self {
            Phase::Focus => "focus",
            Phase::ShortBreak => "break",
            Phase::LongBreak => "long break",
        }
    }

    pub fn is_break(self) -> bool {
        self != Phase::Focus
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PomodoroConfig {
    pub focus: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
    /// A long break follows every `cycles`th focus (0 is treated as 1).
    pub cycles: u32,
    /// When a phase runs out, start the next one straight away. If false
    /// the next phase waits, paused at its full length.
    pub auto_advance: bool,
}

impl Default for PomodoroConfig {
    fn default() -> Self {
        Self {
            focus: Duration::from_secs(25 * 60),
            short_break: Duration::from_secs(5 * 60),
            long_break: Duration::from_secs(15 * 60),
            cycles: 4,
            auto_advance: true,
        }
    }
}

impl PomodoroConfig {
    pub fn duration(&self, phase: Phase) -> Duration {
        match phase {
            Phase::Focus => self.focus,
            Phase::ShortBreak => self.short_break,
            Phase::LongBreak => self.long_break,
        }
    }

    fn cycles(&self) -> u32 {
        self.cycles.max(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Nothing started; shows a full focus phase.
    Idle,
    Running,
    Paused,
}

/// Emitted when a phase ends (UI: bell, flash, toast).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhaseEnd {
    pub ended: Phase,
    pub next: Phase,
    /// Ended by `skip` rather than by running out.
    pub skipped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Run {
    Idle,
    /// `banked` is time run before the latest resume.
    Running {
        since: Instant,
        banked: Duration,
    },
    Paused {
        banked: Duration,
    },
}

#[derive(Clone, Debug)]
pub struct Pomodoro {
    config: PomodoroConfig,
    phase: Phase,
    run: Run,
    cycles: u32,
}

impl Pomodoro {
    pub fn new(config: PomodoroConfig) -> Self {
        Self {
            config,
            phase: Phase::Focus,
            run: Run::Idle,
            cycles: 0,
        }
    }

    pub fn config(&self) -> &PomodoroConfig {
        &self.config
    }

    /// Takes effect immediately; time already run in this phase is kept.
    pub fn set_config(&mut self, config: PomodoroConfig) {
        self.config = config;
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn status(&self) -> Status {
        match self.run {
            Run::Idle => Status::Idle,
            Run::Running { .. } => Status::Running,
            Run::Paused { .. } => Status::Paused,
        }
    }

    /// Focus phases finished since the last reset (skipped ones count: they
    /// still move you on through the set).
    pub fn cycles(&self) -> u32 {
        self.cycles
    }

    /// Position in the current set for the cycle dots: `(done, of)`. Shows
    /// a full set during the long break.
    pub fn set_progress(&self) -> (u32, u32) {
        let of = self.config.cycles();
        let done = if self.phase == Phase::LongBreak {
            of
        } else {
            self.cycles % of
        };
        (done, of)
    }

    pub fn phase_duration(&self) -> Duration {
        self.config.duration(self.phase)
    }

    /// Time run in the current phase (not counting pauses).
    pub fn elapsed(&self, now: Instant) -> Duration {
        match self.run {
            Run::Idle => Duration::ZERO,
            Run::Running { since, banked } => banked + now.saturating_duration_since(since),
            Run::Paused { banked } => banked,
        }
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        self.phase_duration().saturating_sub(self.elapsed(now))
    }

    /// Fraction of the phase done, 0.0..=1.0.
    pub fn progress(&self, now: Instant) -> f64 {
        let total = self.phase_duration().as_secs_f64();
        if total <= 0.0 {
            return 1.0;
        }
        (self.elapsed(now).as_secs_f64() / total).clamp(0.0, 1.0)
    }

    /// Idle → running focus; paused → resume. Returns whether anything changed.
    pub fn start(&mut self, now: Instant) -> bool {
        match self.run {
            Run::Idle => {
                self.phase = Phase::Focus;
                self.run = Run::Running {
                    since: now,
                    banked: Duration::ZERO,
                };
                true
            }
            Run::Paused { .. } => self.resume(now),
            Run::Running { .. } => false,
        }
    }

    pub fn pause(&mut self, now: Instant) -> bool {
        match self.run {
            Run::Running { .. } => {
                self.run = Run::Paused {
                    banked: self.elapsed(now),
                };
                true
            }
            _ => false,
        }
    }

    pub fn resume(&mut self, now: Instant) -> bool {
        match self.run {
            Run::Paused { banked } => {
                self.run = Run::Running { since: now, banked };
                true
            }
            _ => false,
        }
    }

    /// The space key: start if idle, else pause/resume.
    pub fn toggle(&mut self, now: Instant) {
        if !self.pause(now) {
            self.start(now);
        }
    }

    /// End the current phase now. Running stays running (into the next
    /// phase), paused stays paused. No-op when idle.
    pub fn skip(&mut self, now: Instant) -> Option<PhaseEnd> {
        let next_run = match self.run {
            Run::Idle => return None,
            Run::Running { .. } => Run::Running {
                since: now,
                banked: Duration::ZERO,
            },
            Run::Paused { .. } => Run::Paused {
                banked: Duration::ZERO,
            },
        };
        Some(self.advance(next_run, true))
    }

    /// Back to idle focus with zero cycles.
    pub fn reset(&mut self) {
        *self = Self::new(self.config);
    }

    /// The machine was suspended for `asleep` (which `Instant` doesn't
    /// count on macOS or Linux; the caller detects it from wall time). A
    /// running phase counts the sleep as run time, like a kitchen timer; a
    /// paused or idle one is unaffected. The next `tick` ends the phase if
    /// the sleep used it up.
    pub fn slept(&mut self, asleep: Duration) {
        if let Run::Running { banked, .. } = &mut self.run {
            *banked += asleep;
        }
    }

    /// Call every frame. Ends the phase if its time is up and returns the
    /// event. Ends at most one phase per call: after a long gap (laptop
    /// asleep) the next phase starts at `now` instead of also expiring.
    pub fn tick(&mut self, now: Instant) -> Option<PhaseEnd> {
        let Run::Running { since, banked } = self.run else {
            return None;
        };
        let duration = self.phase_duration();
        if self.elapsed(now) < duration {
            return None;
        }
        let next_run = if self.config.auto_advance {
            // Start the next phase at the deadline so frame jitter doesn't
            // accumulate, unless the deadline passed during a sleep (no
            // `Instant` for it) or would already have expired that phase too.
            let next = self.next_phase();
            let deadline = duration.checked_sub(banked).map(|left| since + left);
            let since = match deadline {
                Some(at) if now.saturating_duration_since(at) < self.config.duration(next) => at,
                _ => now,
            };
            Run::Running {
                since,
                banked: Duration::ZERO,
            }
        } else {
            Run::Paused {
                banked: Duration::ZERO,
            }
        };
        Some(self.advance(next_run, false))
    }

    fn next_phase(&self) -> Phase {
        match self.phase {
            Phase::Focus if (self.cycles + 1).is_multiple_of(self.config.cycles()) => {
                Phase::LongBreak
            }
            Phase::Focus => Phase::ShortBreak,
            _ => Phase::Focus,
        }
    }

    fn advance(&mut self, next_run: Run, skipped: bool) -> PhaseEnd {
        let ended = self.phase;
        let next = self.next_phase();
        if ended == Phase::Focus {
            self.cycles += 1;
        }
        self.phase = next;
        self.run = next_run;
        PhaseEnd {
            ended,
            next,
            skipped,
        }
    }
}

/// `18:24`, `5:00`, or `1:30:00` past an hour. Rounds up, so a fresh 25 min
/// phase reads `25:00` and `0:00` only shows once it's over.
pub fn format_remaining(d: Duration) -> String {
    let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
