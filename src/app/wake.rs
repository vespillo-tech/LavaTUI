//! Keep input live while avoiding a coalesced poll at the frame deadline.
use std::io;
use std::time::{Duration, Instant};

const GUARD: Duration = Duration::from_micros(1500);
const SPIN: Duration = Duration::from_micros(200);

trait Wait {
    fn now(&self) -> Instant;
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn sleep(&mut self, duration: Duration);
    fn relax(&mut self);
}

struct System;
impl Wait for System {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        ratatui::crossterm::event::poll(timeout)
    }
    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
    fn relax(&mut self) {
        std::hint::spin_loop();
    }
}

pub(super) fn poll(timeout: Duration) -> io::Result<bool> {
    precise_poll(&mut System, timeout)
}

fn precise_poll(wait: &mut impl Wait, timeout: Duration) -> io::Result<bool> {
    if timeout.is_zero() {
        return wait.poll(Duration::ZERO);
    }
    let deadline = wait.now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(wait.now());
        if remaining > GUARD {
            if wait.poll(remaining - GUARD)? {
                return Ok(true);
            }
        } else {
            if wait.poll(Duration::ZERO)? {
                return Ok(true);
            }
            let remaining = deadline.saturating_duration_since(wait.now());
            if remaining.is_zero() {
                return Ok(false);
            }
            if remaining > SPIN {
                // Cap sleeps so input arriving in the guard window waits
                // at most 100 us plus scheduler latency, not the full guard.
                wait.sleep((remaining - SPIN).min(Duration::from_micros(100)));
            } else {
                // At most 200 us of CPU relaxation per scheduled frame. An OS
                // yield can deschedule even an interactive thread for ~10 ms;
                // keep the final window on-core instead. No spin
                // window when frozen or unfocused (selected by the caller).
                wait.relax();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        now: Instant,
        input: Option<Instant>,
        oversleep: Duration,
        spun: Duration,
        polls: Vec<Duration>,
    }
    impl Wait for Fake {
        fn now(&self) -> Instant {
            self.now
        }
        fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            self.polls.push(timeout);
            if let Some(input) = self.input.filter(|at| *at <= self.now + timeout) {
                self.now = self.now.max(input);
                return Ok(true);
            }
            self.now += timeout;
            if !timeout.is_zero() {
                self.now += self.oversleep;
            }
            Ok(false)
        }
        fn sleep(&mut self, duration: Duration) {
            self.now += duration + self.oversleep;
        }
        fn relax(&mut self) {
            let duration = Duration::from_micros(10);
            self.now += duration;
            self.spun += duration;
        }
    }
    fn fake(now: Instant) -> Fake {
        Fake {
            now,
            input: None,
            oversleep: Duration::ZERO,
            spun: Duration::ZERO,
            polls: Vec::new(),
        }
    }

    #[test]
    fn wakes_on_deadline_with_bounded_spin() {
        let start = Instant::now();
        let mut clock = fake(start);
        let timeout = Duration::from_millis(17);
        assert!(!precise_poll(&mut clock, timeout).unwrap());
        assert_eq!(clock.now, start + timeout);
        assert_eq!(clock.polls[0], timeout - GUARD);
        assert_eq!(clock.spun, SPIN);
    }

    #[test]
    fn guard_absorbs_late_poll_and_sleep() {
        let start = Instant::now();
        let mut clock = fake(start);
        clock.oversleep = Duration::from_micros(300);
        assert!(!precise_poll(&mut clock, Duration::from_millis(17)).unwrap());
        assert!(clock.now >= start + Duration::from_millis(17));
        assert!(clock.now <= start + Duration::from_micros(17300));
        assert!(clock.spun <= SPIN);
    }

    #[test]
    fn input_wakes_in_both_blocked_and_precise_phases() {
        let start = Instant::now();
        for at in [
            Duration::from_millis(5),
            Duration::from_micros(16250),
            Duration::from_micros(16850),
        ] {
            let mut clock = fake(start);
            clock.input = Some(start + at);
            assert!(precise_poll(&mut clock, Duration::from_millis(17)).unwrap());
            assert!(clock.now - (start + at) <= Duration::from_micros(100));
        }
    }

    #[test]
    fn zero_poll_and_overslept_deadline_do_not_spin() {
        let mut clock = fake(Instant::now());
        assert!(!precise_poll(&mut clock, Duration::ZERO).unwrap());
        assert_eq!(clock.polls, [Duration::ZERO]);
        clock.oversleep = Duration::from_millis(20);
        assert!(!precise_poll(&mut clock, Duration::from_millis(17)).unwrap());
        assert_eq!(clock.spun, Duration::ZERO);
    }
}
