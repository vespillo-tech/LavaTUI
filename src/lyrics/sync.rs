//! Which line is playing: position → line index, progress, neighbours.
//!
//! The position is extrapolated from the last player sample, so it runs a
//! little ahead or behind; each new sample can nudge it back. [`Syncer`]
//! keeps that jitter from flicking the highlight back a line, while a real
//! seek (in either direction) is followed at once and flagged so the widget
//! can cut instead of animating.

use std::time::{Duration, Instant};

use super::Playback;
use super::lrc::{Line, Synced};

/// Default lead: lines light up slightly early, which reads as on time.
pub const DEFAULT_LEAD: Duration = Duration::from_millis(150);
/// A step back smaller than this past a line's start keeps the line.
pub const JITTER: Duration = Duration::from_millis(400);
/// Further than this from where the position should be is a seek.
pub const SEEK: Duration = Duration::from_millis(1500);
/// The last line's length when the track's isn't known.
pub const LAST_LINE: Duration = Duration::from_secs(6);

/// Where playback is in the lyrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cursor {
    /// The current line; `None` before the first one.
    pub index: Option<usize>,
    /// 0..=1 through the current line (or the intro before the first).
    pub progress: f32,
    /// The position used, lead included.
    pub position: Duration,
    /// The position jumped (seek, track restart): don't animate.
    pub seeked: bool,
}

impl Cursor {
    pub fn current<'a>(&self, lyrics: &'a Synced) -> Option<&'a Line> {
        lyrics.lines.get(self.index?)
    }

    #[cfg(test)]
    pub fn previous<'a>(&self, lyrics: &'a Synced) -> Option<&'a Line> {
        lyrics.lines.get(self.index?.checked_sub(1)?)
    }

    #[cfg(test)]
    pub fn next<'a>(&self, lyrics: &'a Synced) -> Option<&'a Line> {
        lyrics.lines.get(self.index.map_or(0, |i| i + 1))
    }
}

/// Tracks the cursor across frames. One per track: [`reset`](Self::reset)
/// on track change.
#[derive(Clone, Debug)]
pub struct Syncer {
    pub lead: Duration,
    last: Option<Last>,
}

#[derive(Clone, Copy, Debug)]
struct Last {
    index: Option<usize>,
    position: Duration,
    at: Instant,
    playing: bool,
}

impl Default for Syncer {
    fn default() -> Self {
        Self::new(DEFAULT_LEAD)
    }
}

impl Syncer {
    pub fn new(lead: Duration) -> Self {
        Self { lead, last: None }
    }

    pub fn reset(&mut self) {
        self.last = None;
    }

    /// The cursor at `now`. `track_len` bounds the last line.
    pub fn cursor(
        &mut self,
        lyrics: &Synced,
        track_len: Option<Duration>,
        playback: &Playback,
        now: Instant,
    ) -> Cursor {
        let position = playback.position_at(now) + self.lead;
        let lines = &lyrics.lines;
        let mut index = lines.partition_point(|l| l.at <= position).checked_sub(1);

        let seeked = self.last.is_some_and(|last| {
            let expected = if last.playing {
                last.position + now.saturating_duration_since(last.at)
            } else {
                last.position
            };
            position.abs_diff(expected) > SEEK
        });
        if let Some(last) = self.last.filter(|_| !seeked) {
            // A small step back across a line start: hold the later line.
            if let Some(held) = last.index
                && index.is_none_or(|i| i < held)
                && lines
                    .get(held)
                    .is_some_and(|l| l.at.saturating_sub(position) <= JITTER)
            {
                index = Some(held);
            }
        }
        self.last = Some(Last {
            index,
            position,
            at: now,
            playing: playback.playing,
        });

        let (start, end) = match index {
            None => (
                Duration::ZERO,
                lines.first().map_or(Duration::ZERO, |l| l.at),
            ),
            Some(i) => {
                let start = lines[i].at;
                let end = match lines.get(i + 1) {
                    Some(next) => next.at,
                    None => track_len
                        .filter(|&len| len > start)
                        .unwrap_or(start + LAST_LINE),
                };
                (start, end)
            }
        };
        let span = end.saturating_sub(start).as_secs_f32();
        let progress = if span > 0.0 {
            (position.saturating_sub(start).as_secs_f32() / span).clamp(0.0, 1.0)
        } else {
            1.0
        };
        Cursor {
            index,
            progress,
            position,
            seeked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lyrics() -> Synced {
        Synced::parse("[00:05.00]one\n[00:10.00]two\n[00:15.00]\n[00:20.00]three")
    }

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    /// A fake clock: `t(s)` is `s` seconds after a fixed origin.
    struct Clock(Instant);

    impl Clock {
        fn t(&self, s: f64) -> Instant {
            self.0 + secs(s)
        }

        fn playing(&self, pos: f64, sampled: f64) -> Playback {
            Playback {
                position: secs(pos),
                sampled_at: self.t(sampled),
                playing: true,
            }
        }
    }

    fn clock() -> Clock {
        Clock(Instant::now())
    }

    #[test]
    fn intro_lines_gaps_and_end() {
        let l = lyrics();
        let c = clock();
        let mut s = Syncer::new(Duration::ZERO);
        let p = c.playing(0.0, 0.0);
        let at = |s: &mut Syncer, t| s.cursor(&l, Some(secs(30.0)), &p, c.t(t));

        let intro = at(&mut s, 2.5);
        assert_eq!(intro.index, None);
        assert!((intro.progress - 0.5).abs() < 1e-3);
        assert_eq!(intro.current(&l), None);
        assert_eq!(intro.next(&l).unwrap().text, "one");

        let one = at(&mut s, 7.5);
        assert_eq!(one.index, Some(0));
        assert!((one.progress - 0.5).abs() < 1e-3);
        assert_eq!(one.previous(&l), None);
        assert_eq!(one.next(&l).unwrap().text, "two");

        let gap = at(&mut s, 16.0);
        assert!(gap.current(&l).unwrap().is_gap());
        assert_eq!(gap.previous(&l).unwrap().text, "two");
        assert_eq!(gap.next(&l).unwrap().text, "three");

        // The last line runs to the track's end.
        let last = at(&mut s, 25.0);
        assert_eq!(last.index, Some(3));
        assert!((last.progress - 0.5).abs() < 1e-3);
        assert_eq!(last.next(&l), None);
        assert_eq!(at(&mut s, 99.0).progress, 1.0);
        assert!(!last.seeked);
    }

    #[test]
    fn lead_lights_lines_early() {
        let l = lyrics();
        let c = clock();
        let p = c.playing(0.0, 0.0);
        let mut s = Syncer::default();
        assert_eq!(s.cursor(&l, None, &p, c.t(4.80)).index, None);
        assert_eq!(s.cursor(&l, None, &p, c.t(4.86)).index, Some(0));
    }

    #[test]
    fn unknown_track_length_and_paused() {
        let l = lyrics();
        let c = clock();
        let mut s = Syncer::new(Duration::ZERO);
        let paused = Playback {
            playing: false,
            ..c.playing(23.0, 0.0)
        };
        // Paused: time passing changes nothing.
        let a = s.cursor(&l, None, &paused, c.t(1.0));
        let b = s.cursor(&l, None, &paused, c.t(50.0));
        assert_eq!(a, b);
        assert!((a.progress - 0.5).abs() < 1e-3, "{LAST_LINE:?} default");
        assert!(!b.seeked);
    }

    #[test]
    fn small_backward_correction_holds_the_line() {
        let l = lyrics();
        let c = clock();
        let mut s = Syncer::new(Duration::ZERO);
        // Extrapolated just past "two"...
        let ahead = s.cursor(&l, None, &c.playing(9.0, 0.0), c.t(1.1));
        assert_eq!(ahead.index, Some(1));
        // ...then a fresh sample says we're 0.2 s behind that.
        let behind = s.cursor(&l, None, &c.playing(9.9, 1.15), c.t(1.15));
        assert_eq!(behind.index, Some(1));
        assert_eq!(behind.progress, 0.0);
        assert!(!behind.seeked);
        // And it carries on normally.
        let on = s.cursor(&l, None, &c.playing(9.9, 1.15), c.t(1.4));
        assert_eq!(on.index, Some(1));
    }

    #[test]
    fn seeks_are_followed_and_flagged() {
        let l = lyrics();
        let c = clock();
        let mut s = Syncer::new(Duration::ZERO);
        assert_eq!(
            s.cursor(&l, None, &c.playing(21.0, 0.0), c.t(0.0)).index,
            Some(3)
        );

        // Back to the start of "one".
        let back = s.cursor(&l, None, &c.playing(5.5, 0.1), c.t(0.1));
        assert_eq!(back.index, Some(0));
        assert!(back.seeked);
        let steady = s.cursor(&l, None, &c.playing(5.5, 0.1), c.t(0.2));
        assert!(!steady.seeked);

        // Forward over a line.
        let fwd = s.cursor(&l, None, &c.playing(16.0, 0.3), c.t(0.3));
        assert_eq!(fwd.index, Some(2));
        assert!(fwd.seeked);

        // Back to before the first line (track restart).
        let restart = s.cursor(&l, None, &c.playing(0.0, 0.4), c.t(0.4));
        assert_eq!(restart.index, None);
        assert!(restart.seeked);
    }

    #[test]
    fn backward_step_past_jitter_is_not_held() {
        let l = lyrics();
        let c = clock();
        let mut s = Syncer::new(Duration::ZERO);
        assert_eq!(
            s.cursor(&l, None, &c.playing(10.2, 0.0), c.t(0.0)).index,
            Some(1)
        );
        // 1 s back: not a seek (< SEEK) but beyond JITTER: back to "one".
        let back = s.cursor(&l, None, &c.playing(9.2, 0.0), c.t(0.0));
        assert_eq!(back.index, Some(0));
        assert!(!back.seeked);
    }

    #[test]
    fn reset_forgets_the_last_track() {
        let l = lyrics();
        let c = clock();
        let mut s = Syncer::new(Duration::ZERO);
        s.cursor(&l, None, &c.playing(21.0, 0.0), c.t(0.0));
        s.reset();
        let fresh = s.cursor(&l, None, &c.playing(1.0, 0.0), c.t(0.0));
        assert!(!fresh.seeked);
        assert_eq!(fresh.index, None);
    }

    #[test]
    fn empty_lyrics() {
        let c = clock();
        let cur =
            Syncer::default().cursor(&Synced::default(), None, &c.playing(3.0, 0.0), c.t(0.0));
        assert_eq!(cur.index, None);
        assert_eq!(cur.progress, 1.0);
        assert_eq!(cur.next(&Synced::default()), None);
    }
}
