//! Synced lyrics from LRCLIB (lrclib.net): fetch, cache, parse, sync.
//!
//! Pure core, no terminal: the app (`app/model/lyrics.rs`) feeds it a
//! [`Track`] when the song changes and a [`Playback`] sample each frame.
//!
//! - [`lrc`]: the LRC parser ([`Synced`]: timed lines, metadata, offset).
//! - [`sync`]: [`Syncer`] maps an extrapolated position to the current
//!   line, its progress and the lines around it.
//! - [`breaks`]: where a line breaks onto rows, in any script (nothing
//!   allocated).
//! - [`client`]: the LRCLIB HTTP client (`/api/get`, `/api/search`
//!   fallback) over a small [`client::Http`] trait, so tests mock it.
//! - [`cache`]: the on-disk cache, negative results included, with TTLs.
//! - [`worker`]: [`LyricsService`], the fetch thread behind a channel API
//!   (newest request wins, transient failures retried).
//!
//! Privacy: a fetch sends title/artist/album/duration to lrclib.net, so the
//! app only starts the service when the user opts in (placing the widget).

pub mod breaks;
pub mod cache;
pub mod client;
pub mod lrc;
pub mod sync;
pub mod words;
pub mod worker;

use std::time::{Duration, Instant};

pub use lrc::Synced;
pub use sync::Syncer;
pub use worker::LyricsService;

/// Whether the current line lights up word by word (`lyrics.karaoke`).
/// Word times are exact only when the lyrics have them (enhanced LRC,
/// rare); otherwise they're estimated ([`words`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Karaoke {
    /// Every synced line, word by word.
    #[default]
    On,
    /// Only lines whose lyrics time each word.
    Timed,
    /// The whole line at once (line timing as ever).
    Off,
}

impl Karaoke {
    pub const ALL: [Self; 3] = [Self::On, Self::Timed, Self::Off];

    /// Whether `line` is shown word by word.
    pub fn shows(self, line: &lrc::Line) -> bool {
        match self {
            Self::On => true,
            Self::Timed => line.exact,
            Self::Off => false,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Timed => "timed",
            Self::Off => "off",
        }
    }
}

impl serde::Serialize for Karaoke {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

/// `"on"`, `"timed"`, `"off"`, or `true` / `false` (on / off).
impl<'de> serde::Deserialize<'de> for Karaoke {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Name(String),
        }
        match Raw::deserialize(d)? {
            Raw::Bool(true) => Ok(Self::On),
            Raw::Bool(false) => Ok(Self::Off),
            Raw::Name(name) => Self::ALL
                .into_iter()
                .find(|k| k.name() == name)
                .ok_or_else(|| serde::de::Error::custom("on, timed or off")),
        }
    }
}

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
    /// No vocals: LRCLIB marks the track so, or its only "lyrics" are a
    /// note saying it is (`[Instrumental]`, `♪ Instrumental ♪`).
    Instrumental,
}

impl Lyrics {
    /// Builds lyrics from LRCLIB's fields: synced if it parses to at least
    /// one timed line, else plain, else nothing. Text that only says
    /// "instrumental" counts as [`Lyrics::Instrumental`].
    pub fn from_parts(
        instrumental: bool,
        synced: Option<&str>,
        plain: Option<&str>,
    ) -> Option<Self> {
        if instrumental {
            return Some(Self::Instrumental);
        }
        if let Some(synced) = synced.map(Synced::parse).filter(|s| !s.lines.is_empty()) {
            let marker = only_marker(synced.lines.iter().map(|l| l.text.as_str()));
            return Some(if marker {
                Self::Instrumental
            } else {
                Self::Synced(synced)
            });
        }
        let lines = plain
            .map(lrc::plain_lines)
            .filter(|lines| lines.iter().any(|l| !l.is_empty()))?;
        Some(if only_marker(lines.iter().map(String::as_str)) {
            Self::Instrumental
        } else {
            Self::Plain(lines)
        })
    }
}

/// Whether the non-empty `lines` are all just the word "instrumental", with
/// any brackets, notes or dashes around it. Some uploaders put that where
/// the words would be instead of setting LRCLIB's flag.
fn only_marker<'a>(lines: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = false;
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let word: String = line.chars().filter(|c| c.is_alphanumeric()).collect();
        if !word.eq_ignore_ascii_case("instrumental") {
            return false;
        }
        seen = true;
    }
    seen
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

    #[test]
    fn a_note_that_says_instrumental_is_instrumental() {
        for synced in [
            "[00:00.00]♪ Instrumental ♪",
            "[00:00.00][Instrumental]\n[01:30.00]\n[02:00.00](instrumental)",
        ] {
            assert_eq!(
                Lyrics::from_parts(false, Some(synced), None),
                Some(Lyrics::Instrumental),
                "{synced}"
            );
        }
        for plain in ["[Instrumental]", "\n♪ INSTRUMENTAL ♪\n\n- instrumental -\n"] {
            assert_eq!(
                Lyrics::from_parts(false, None, Some(plain)),
                Some(Lyrics::Instrumental),
                "{plain}"
            );
        }
        // Words that merely mention it are lyrics.
        assert!(matches!(
            Lyrics::from_parts(false, None, Some("[Instrumental]\nla la la")),
            Some(Lyrics::Plain(_))
        ));
        assert!(matches!(
            Lyrics::from_parts(false, Some("[00:01.00]instrumental break"), None),
            Some(Lyrics::Synced(_))
        ));
    }
}
