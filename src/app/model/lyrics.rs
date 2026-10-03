//! The lyrics widget's state: the LRCLIB service (alive only while the
//! widget is placed: placing it is the opt-in, since a lookup sends the
//! track's title, artist, album and length to lrclib.net), the lyrics of
//! the playing track, and where playback is in them.
//!
//! Driven by the media snapshot the music state reads each frame (the
//! source is held while either widget is placed): a new track sends a
//! request, answers are polled (never waited on), and the position is
//! extrapolated from the snapshot to the current line.

use std::time::{Duration, Instant};

use super::Model;
use crate::dock::{self, Place};
use crate::lyrics::sync::Cursor;
use crate::lyrics::worker::Answer;
use crate::lyrics::{self, Lyrics, LyricsService, Playback, Syncer};
use crate::media::{Snapshot, Status};

/// How long a new line takes to brighten (and the old one to dim).
pub const FADE: Duration = Duration::from_millis(320);
/// While LRCLIB is out of reach, the playing track is asked about again
/// this often (lava-75z.22: a network blip as a song began used to leave
/// it on "lyrics offline" to the end).
pub const ASK_AGAIN: Duration = Duration::from_secs(30);

type Start = Box<dyn Fn() -> Option<LyricsService>>;

/// What there is to show for the playing track.
#[derive(Debug, Clone, PartialEq)]
pub enum Fetch {
    /// Asked; no answer yet.
    Looking,
    Lyrics(Lyrics),
    NotFound,
    /// LRCLIB unreachable and nothing cached.
    Offline,
}

pub struct LyricsState {
    service: Option<LyricsService>,
    start: Start,
    /// The track the lyrics are for (the last one asked about).
    track: Option<lyrics::Track>,
    /// When it was last asked about.
    asked_at: Option<Instant>,
    pub found: Option<Fetch>,
    syncer: Syncer,
    /// Where playback is in synced lyrics, this frame.
    pub cursor: Option<Cursor>,
    /// When the current line last changed (not on a seek), and the line
    /// before it: the fade between the two.
    pub changed: Option<(Instant, Option<usize>)>,
    /// What the widget's forms are sized by, per song.
    pub sizing: dock::LyricsSizing,
}

impl Default for LyricsState {
    fn default() -> Self {
        Self {
            service: None,
            start: Box::new(|| LyricsService::start().ok()),
            track: None,
            asked_at: None,
            found: None,
            syncer: Syncer::default(),
            cursor: None,
            changed: None,
            sizing: dock::LyricsSizing::default(),
        }
    }
}

impl LyricsState {
    /// Use `start` for the lookup service (tests and `--demo`: a mock LRCLIB).
    pub fn start_with(&mut self, start: impl Fn() -> Option<LyricsService> + 'static) {
        self.start = Box::new(start);
        self.forget();
        self.service = None;
    }

    fn forget(&mut self) {
        self.track = None;
        self.asked_at = None;
        self.found = None;
        self.cursor = None;
        self.changed = None;
        self.sizing = dock::LyricsSizing::default();
        self.syncer.reset();
    }

    /// The synced lyrics, if that's what there is.
    pub fn synced(&self) -> Option<&lyrics::Synced> {
        match &self.found {
            Some(Fetch::Lyrics(Lyrics::Synced(s))) => Some(s),
            _ => None,
        }
    }

    /// Once a frame: start or stop the service, ask about a new track, take
    /// in an answer, and move the cursor to `now`, the lyrics `delay_ms`
    /// later than the player's position (sooner if negative).
    pub fn sync(&mut self, on: bool, snapshot: Option<&Snapshot>, delay_ms: i32, now: Instant) {
        if !on {
            self.service = None;
            self.forget();
            return;
        }
        let playing = snapshot.and_then(|snap| {
            let track = snap.track.as_ref()?;
            matches!(snap.status, Status::Playing | Status::Paused).then_some((snap, track))
        });
        let Some((snap, track)) = playing else {
            // Nothing playing: keep the lyrics (a pause at the end of a
            // track, a moment of `Connecting`) but nothing to sync.
            self.cursor = None;
            return;
        };
        let key = lyrics::Track {
            title: track.name.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration: Some(track.duration).filter(|d| !d.is_zero()),
        };
        if key.title.trim().is_empty() {
            self.forget();
            return;
        }
        if self.track.as_ref() != Some(&key) {
            self.forget();
            if self.service.is_none() {
                self.service = (self.start)();
            }
            self.found = Some(match &mut self.service {
                Some(service) => {
                    service.request(key.clone());
                    Fetch::Looking
                }
                None => Fetch::Offline,
            });
            self.track = Some(key);
            self.asked_at = Some(now);
        } else if self.found == Some(Fetch::Offline)
            && self
                .asked_at
                .is_none_or(|at| now.saturating_duration_since(at) >= ASK_AGAIN)
        {
            // "lyrics offline" stays up until the answer replaces it.
            if self.service.is_none() {
                self.service = (self.start)();
            }
            if let Some(service) = &mut self.service {
                service.request(key);
            }
            self.asked_at = Some(now);
        }
        if let Some(response) = self.service.as_mut().and_then(LyricsService::poll) {
            self.found = Some(match response.answer {
                Answer::Lyrics(lyrics) => {
                    self.sizing = sizing(&lyrics);
                    Fetch::Lyrics(lyrics)
                }
                Answer::NotFound => Fetch::NotFound,
                Answer::Offline(_) => Fetch::Offline,
            });
        }

        self.syncer.delay_ms = delay_ms;
        let playback = Playback {
            position: snap.position,
            sampled_at: snap.sampled_at,
            playing: snap.status == Status::Playing,
        };
        let duration = self.track.as_ref().and_then(|t| t.duration);
        let Some(Fetch::Lyrics(Lyrics::Synced(synced))) = &self.found else {
            self.cursor = None;
            return;
        };
        let cursor = self.syncer.cursor(synced, duration, &playback, now);
        let before = self.cursor.and_then(|c| c.index);
        if self.cursor.is_some() && cursor.index != before {
            self.changed = (!cursor.seeked).then_some((now, before));
        }
        self.cursor = Some(cursor);
    }

    /// 0..=1 through the fade into the current line (1: settled).
    pub fn fade(&self, now: Instant) -> f32 {
        self.changed.map_or(1.0, |(at, _)| {
            (now.saturating_duration_since(at).as_secs_f32() / FADE.as_secs_f32()).min(1.0)
        })
    }

    /// When what's shown next changes on its own: the end of a fade, the
    /// next word, line or gap dot (only while playing). For a frozen
    /// lamp's sleep.
    pub fn wake(&self, now: Instant) -> Option<Instant> {
        if self.fade(now) < 1.0 {
            return Some(now);
        }
        let cursor = self.cursor.filter(|c| c.playing)?;
        let base = cursor.position.saturating_sub(self.syncer.lead);
        Some(now + cursor.next_change?.saturating_sub(base))
    }
}

/// How the widget's forms are sized for `lyrics`.
fn sizing(lyrics: &Lyrics) -> dock::LyricsSizing {
    match lyrics {
        Lyrics::Synced(s) => dock::LyricsSizing::of(s.lines.iter().map(|l| l.text.as_str())),
        Lyrics::Plain(lines) => dock::LyricsSizing::of(lines.iter().map(String::as_str)),
        Lyrics::Instrumental => dock::LyricsSizing::default(),
    }
}

/// `m:ss.s`.
fn clock(d: Duration) -> String {
    let t = d.as_secs_f64();
    format!("{}:{:04.1}", (t / 60.0) as u64, t % 60.0)
}

impl Model {
    /// The lyrics' timing in numbers, for the performance info (`d`) while
    /// synced lyrics play, so what's seen can be told exactly: where the
    /// player is (after lyrics timing), how old its last reading is, the
    /// line's start, sung-by and next times, the word, and whether word
    /// times are the file's or estimated.
    /// `♪ 1:23.4 (read 0.4 s ago) · line 12/48 1:21.0 sung 1:24.2 next 1:25.5 · word 3/8 estimated · timing +0 ms`
    pub fn lyrics_readout(&self) -> Option<String> {
        let synced = self.lyrics.synced()?;
        let cursor = self.lyrics.cursor?;
        let snap = self.music.snapshot.as_ref()?;
        let syncer = &self.lyrics.syncer;
        let at = cursor.position.saturating_sub(syncer.lead);
        let age = self.now.saturating_duration_since(snap.sampled_at);
        let mut out = format!("♪ {} (read {:.1} s ago)", clock(at), age.as_secs_f64());
        match cursor.index.and_then(|i| Some((i, synced.lines.get(i)?))) {
            Some((i, line)) => {
                let next = synced
                    .lines
                    .get(i + 1)
                    .map_or(String::new(), |n| format!(" next {}", clock(n.at)));
                out += &format!(
                    " · line {}/{} {} sung {}{next}",
                    i + 1,
                    synced.lines.len(),
                    clock(line.at),
                    clock(line.end)
                );
                if !line.words.is_empty() {
                    let word = cursor.word.map_or_else(
                        || {
                            if cursor.sung == 0 {
                                "-".to_owned()
                            } else {
                                "done".to_owned()
                            }
                        },
                        |w| (w + 1).to_string(),
                    );
                    let how = if line.exact {
                        "from the file"
                    } else {
                        "estimated"
                    };
                    out += &format!(" · word {word}/{} {how}", line.words.len());
                }
            }
            None => out += " · before the first line",
        }
        out += &format!(" · timing {:+} ms", syncer.delay_ms);
        Some(out)
    }

    /// Whether the lyrics widget is placed (lookups are on).
    pub fn lyrics_on(&self) -> bool {
        self.settings.dock.place(&dock::Lyrics) != Place::Off
    }

    /// Each frame, after the music state has read the player.
    pub(super) fn sync_lyrics(&mut self) {
        let on = self.lyrics_on();
        let delay = self.settings.lyrics.delay_ms;
        self.lyrics
            .sync(on, self.music.snapshot.as_ref(), delay, self.now);
    }
}
