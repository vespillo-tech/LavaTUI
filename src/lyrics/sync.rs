//! Which line and word are playing: position → line index, progress,
//! the word being sung, neighbours.
//!
//! The position is extrapolated from the player's position (pinned down
//! over polls by the media worker to a few ms), so it may still step back
//! a little when a reading corrects it; [`Syncer`] holds the cursor still
//! until playback catches up rather than flicking the highlight back a
//! word or a line, while a real seek (in either direction) is followed at
//! once and flagged so the widget can cut instead of animating.

use std::time::{Duration, Instant};

use super::Playback;
use super::lrc::{Line, Synced};

/// Lines light up this much early: the eye reads a line ahead of the voice.
pub const DEFAULT_LEAD: Duration = Duration::from_millis(150);
/// Words light up this much early: about with the voice (a highlight a
/// hair early reads as on time; one late reads as late).
pub const WORD_LEAD: Duration = Duration::from_millis(50);
/// A step back smaller than this holds the cursor where it was until
/// playback catches up.
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
    /// The position used for lines, lead included.
    pub position: Duration,
    /// The position jumped (seek, track restart): don't animate.
    pub seeked: bool,
    /// The word of the current line being sung: `None` just before its
    /// first (the line lights up a little ahead), and once it's all sung.
    pub word: Option<usize>,
    /// How many of the current line's words have been sung.
    pub sung: usize,
    /// When (in playback position, no lead) what's shown next changes on
    /// its own: a word, a line, a gap's dot. `None` at the end.
    pub next_change: Option<Duration>,
    /// The position is moving.
    pub playing: bool,
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
    /// How early lines light up ([`DEFAULT_LEAD`]).
    pub lead: Duration,
    /// How early words light up ([`WORD_LEAD`]).
    pub word_lead: Duration,
    /// The lyrics timing setting: this many ms later than the player's
    /// position (sooner if negative), applied to the extrapolated
    /// position, so it holds from the first moment of a song.
    pub delay_ms: i32,
    last: Option<Last>,
}

#[derive(Clone, Copy, Debug)]
struct Last {
    /// The playback position used (no lead).
    position: Duration,
    at: Instant,
    playing: bool,
}

impl Default for Syncer {
    fn default() -> Self {
        Self::new(DEFAULT_LEAD, WORD_LEAD)
    }
}

impl Syncer {
    pub fn new(lead: Duration, word_lead: Duration) -> Self {
        Self {
            lead,
            word_lead,
            delay_ms: 0,
            last: None,
        }
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
        let shift = Duration::from_millis(u64::from(self.delay_ms.unsigned_abs()));
        let mut base = playback.position_at(now);
        base = if self.delay_ms >= 0 {
            base.saturating_sub(shift)
        } else {
            base + shift
        };
        let seeked = self.last.is_some_and(|last| {
            let expected = if last.playing {
                last.position + now.saturating_duration_since(last.at)
            } else {
                last.position
            };
            base.abs_diff(expected) > SEEK
        });
        // A small step back while playing: hold still until playback
        // catches up (never back a word or a line by a correction).
        if let Some(last) = self.last.filter(|_| !seeked && playback.playing)
            && base < last.position
            && last.position - base <= JITTER
        {
            base = last.position;
        }
        self.last = Some(Last {
            position: base,
            at: now,
            playing: playback.playing,
        });

        let position = base + self.lead;
        let lines = &lyrics.lines;
        let index = lines.partition_point(|l| l.at <= position).checked_sub(1);
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

        // The word, and the next moment anything shown changes.
        let word_at = base + self.word_lead;
        let mut word = None;
        let mut sung = 0;
        let mut next: Option<Duration> = lines
            .get(index.map_or(0, |i| i + 1))
            .map(|l| l.at.saturating_sub(self.lead));
        let mut soonest = |t: Duration, lead: Duration| {
            let t = t.saturating_sub(lead);
            if t > base && next.is_none_or(|n| t < n) {
                next = Some(t);
            }
        };
        match index.map(|i| &lines[i]) {
            Some(line) if !line.words.is_empty() => {
                sung = line.words.partition_point(|w| w.at <= word_at);
                if word_at >= line.end {
                    sung = line.words.len();
                } else if sung > 0 {
                    word = Some(sung - 1);
                    sung -= 1;
                }
                match line.words.get(sung + usize::from(word.is_some())) {
                    Some(w) => soonest(w.at, self.word_lead),
                    None => soonest(line.end, self.word_lead),
                }
            }
            // A gap or the intro: its dots light at each third.
            _ => {
                for third in 1..3 {
                    soonest(start + (end - start) * third / 3, self.lead);
                }
            }
        }
        Cursor {
            index,
            progress,
            position,
            seeked,
            word,
            sung,
            next_change: next,
            playing: playback.playing,
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
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
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
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
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
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
        // Extrapolated just past "two"...
        let ahead = s.cursor(&l, None, &c.playing(9.0, 0.0), c.t(1.1));
        assert_eq!(ahead.index, Some(1));
        // ...then a fresh sample says we're 0.2 s behind that: held still
        // where it was, until playback catches up.
        let behind = s.cursor(&l, None, &c.playing(9.9, 1.15), c.t(1.15));
        assert_eq!(behind.index, Some(1));
        assert_eq!(behind.position, ahead.position);
        assert!(!behind.seeked);
        let held = s.cursor(&l, None, &c.playing(9.9, 1.15), c.t(1.25));
        assert_eq!(held.position, ahead.position);
        // And it carries on normally.
        let on = s.cursor(&l, None, &c.playing(9.9, 1.15), c.t(1.4));
        assert_eq!(on.index, Some(1));
        assert!(on.position > ahead.position);
    }

    #[test]
    fn the_timing_setting_moves_the_extrapolated_position() {
        let l = lyrics();
        let c = clock();
        // A reading at 0.1 s, the very start of a song; lyrics 0.5 s later.
        let p = c.playing(0.1, 0.0);
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
        s.delay_ms = 500;
        // 5.4 s on, the player is at 5.5 s: the lyrics show 5.0 s.
        let cur = s.cursor(&l, None, &p, c.t(5.4));
        assert_eq!(cur.position, secs(5.0));
        assert_eq!(cur.index, Some(0), "line one starts at 5 s");
        // Sooner: 0.5 s ahead of the player.
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
        s.delay_ms = -500;
        assert_eq!(s.cursor(&l, None, &p, c.t(4.4)).position, secs(5.0));
        // Never before the song's start.
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
        s.delay_ms = 1000;
        assert_eq!(s.cursor(&l, None, &p, c.t(0.2)).position, Duration::ZERO);
    }

    #[test]
    fn a_pause_shows_where_it_stopped() {
        let l = lyrics();
        let c = clock();
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
        let ahead = s.cursor(&l, None, &c.playing(10.2, 0.0), c.t(0.0));
        assert_eq!(ahead.index, Some(1));
        // The player paused 300 ms before we heard: back to where it is.
        let paused = Playback {
            playing: false,
            ..c.playing(9.9, 0.0)
        };
        let cur = s.cursor(&l, None, &paused, c.t(0.0));
        assert_eq!(cur.index, Some(0));
        assert!(!cur.playing);
    }

    /// "a b c" from 5 s, a word a second, sung by 8 s; next line at 10 s.
    fn worded() -> Synced {
        Synced::parse("[00:05.00]<00:05.00>a <00:06.00>b <00:07.00>c <00:08.00>\n[00:10.00]next")
    }

    #[test]
    fn the_word_being_sung() {
        let l = worded();
        assert!(l.lines[0].exact);
        let c = clock();
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
        let p = c.playing(0.0, 0.0);
        let mut at = |t| s.cursor(&l, None, &p, c.t(t));
        let words = |cur: Cursor| (cur.word, cur.sung);
        assert_eq!(words(at(4.9)), (None, 0), "the intro");
        assert_eq!(words(at(5.0)), (Some(0), 0));
        assert_eq!(words(at(6.5)), (Some(1), 1));
        assert_eq!(words(at(7.99)), (Some(2), 2));
        // Sung, waiting for the next line.
        assert_eq!(words(at(9.0)), (None, 3));
        // The next line's words (estimated) start with it.
        assert_eq!(words(at(10.0)), (Some(0), 0));
    }

    #[test]
    fn words_have_their_own_lead() {
        let l = worded();
        let c = clock();
        let p = c.playing(0.0, 0.0);
        let mut s = Syncer::new(Duration::from_millis(150), Duration::from_millis(50));
        // The line lights up first, its first word a little later.
        let early = s.cursor(&l, None, &p, c.t(4.9));
        assert_eq!((early.index, early.word, early.sung), (Some(0), None, 0));
        let on = s.cursor(&l, None, &p, c.t(4.96));
        assert_eq!(on.word, Some(0));
        let next = s.cursor(&l, None, &p, c.t(5.96));
        assert_eq!(next.word, Some(1));
    }

    #[test]
    fn next_change_is_the_next_word_line_or_dot() {
        let l = worded();
        let c = clock();
        let p = c.playing(0.0, 0.0);
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
        let next = |s: &mut Syncer, t| s.cursor(&l, None, &p, c.t(t)).next_change;
        // The intro's dots light at 5/3 and 10/3 s, then the line.
        assert_eq!(next(&mut s, 1.0), Some(secs(5.0) / 3));
        assert_eq!(next(&mut s, 4.0), Some(secs(5.0)));
        assert_eq!(next(&mut s, 5.5), Some(secs(6.0)));
        assert_eq!(next(&mut s, 7.5), Some(secs(8.0)), "the line's end");
        assert_eq!(next(&mut s, 8.5), Some(secs(10.0)), "the next line");
        // With leads, each comes that much sooner.
        let mut led = Syncer::new(Duration::from_millis(150), Duration::from_millis(50));
        assert_eq!(next(&mut led, 5.5), Some(Duration::from_millis(5950)));
        assert_eq!(next(&mut led, 8.5), Some(Duration::from_millis(9850)));
    }

    #[test]
    fn seeks_are_followed_and_flagged() {
        let l = lyrics();
        let c = clock();
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
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
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
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
        let mut s = Syncer::new(Duration::ZERO, Duration::ZERO);
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
