//! Synced lyrics from LRCLIB (lrclib.net): fetch, cache, parse, sync.
//!
//! Pure core, no terminal: the app (`app/model/lyrics.rs`) feeds it a
//! [`Track`] when the song changes and a [`Playback`] sample each frame.
//!
//! - [`lrc`]: the LRC parser ([`Synced`]: timed lines, metadata, offset).
//! - [`sync`]: [`Syncer`] maps an extrapolated position to the current
//!   line, its progress and the lines around it.
//! - [`client`]: the LRCLIB HTTP client (`/api/get`, `/api/search`
//!   fallback) over a small [`client::Http`] trait, so tests mock it.
//! - [`cache`]: the on-disk cache, negative results included, with TTLs.
//! - [`worker`]: [`LyricsService`], the fetch thread behind a channel API
//!   (newest request wins, transient failures retried).
//!
//! Privacy: a fetch sends title/artist/album/duration to lrclib.net, so the
//! app only starts the service when the user opts in (placing the widget).

pub mod cache;
pub mod client;
pub mod lrc;
pub mod sync;
pub mod worker;

use std::time::{Duration, Instant};

pub use lrc::Synced;
pub use sync::Syncer;
pub use worker::LyricsService;

/// What the media source knows about the playing track: the lookup key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Track {
    pub title: String,
    pub artist: String,
    /// Empty when unknown.
    pub album: String,
    /// LRCLIB matches on duration (±2 s); `None` skips straight to search.
    pub duration: Option<Duration>,
}

/// One position sample from the player, extrapolated between samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Playback {
    /// Position in the track when sampled.
    pub position: Duration,
    /// When the sample was taken.
    pub sampled_at: Instant,
    /// Paused positions don't advance.
    pub playing: bool,
}

impl Playback {
    /// The position at `now`: the sample plus the time since, if playing.
    pub fn position_at(&self, now: Instant) -> Duration {
        if self.playing {
            self.position + now.saturating_duration_since(self.sampled_at)
        } else {
            self.position
        }
    }
}

/// Lyrics for a track, best kind first.
#[derive(Clone, Debug, PartialEq)]
pub enum Lyrics {
    /// Time-synced lines.
    Synced(Synced),
    /// Untimed lines (blank lines kept as stanza breaks).
    Plain(Vec<String>),
    /// LRCLIB marks the track as having no vocals.
    Instrumental,
}

impl Lyrics {
    /// Builds lyrics from LRCLIB's fields: synced if it parses to at least
    /// one timed line, else plain, else nothing.
    pub fn from_parts(
        instrumental: bool,
        synced: Option<&str>,
        plain: Option<&str>,
    ) -> Option<Self> {
        if instrumental {
            return Some(Self::Instrumental);
        }
        if let Some(synced) = synced.map(Synced::parse).filter(|s| !s.lines.is_empty()) {
            return Some(Self::Synced(synced));
        }
        plain
            .map(lrc::plain_lines)
            .filter(|lines| lines.iter().any(|l| !l.is_empty()))
            .map(Self::Plain)
    }
}

/// LRCLIB's answer for a track, unparsed: what the cache stores, so a
/// better parser applies to cached lyrics too.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawLyrics {
    #[serde(default)]
    pub instrumental: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plain: Option<String>,
}

impl RawLyrics {
    /// `None` when there's nothing to show (no lyrics of either kind).
    pub fn lyrics(&self) -> Option<Lyrics> {
        Lyrics::from_parts(
            self.instrumental,
            self.synced.as_deref(),
            self.plain.as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_extrapolates_only_while_playing() {
        let t0 = Instant::now();
        let mut p = Playback {
            position: Duration::from_secs(10),
            sampled_at: t0,
            playing: true,
        };
        let later = t0 + Duration::from_millis(1500);
        assert_eq!(p.position_at(later), Duration::from_millis(11_500));
        // A sample from the future (clock skew) doesn't go backwards.
        assert_eq!(p.position_at(t0 - Duration::from_secs(1)), p.position);
        p.playing = false;
        assert_eq!(p.position_at(later), Duration::from_secs(10));
    }

    #[test]
    fn lyrics_prefer_synced_then_plain() {
        let synced = "[00:01.00]hello";
        assert!(matches!(
            Lyrics::from_parts(false, Some(synced), Some("hello")),
            Some(Lyrics::Synced(_))
        ));
        // Synced text without a single timed line falls back to plain.
        assert_eq!(
            Lyrics::from_parts(false, Some("[ar:x]\nno times"), Some("a\n\nb")),
            Some(Lyrics::Plain(vec!["a".into(), String::new(), "b".into()]))
        );
        assert_eq!(
            Lyrics::from_parts(true, Some(synced), None),
            Some(Lyrics::Instrumental)
        );
        assert_eq!(Lyrics::from_parts(false, None, Some("  \n ")), None);
        assert_eq!(Lyrics::from_parts(false, None, None), None);
    }
}
