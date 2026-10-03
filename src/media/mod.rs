//! Now playing: what a music player is doing, and remote control of it.
//!
//! The UI talks to a [`MediaSource`]: [`MediaSource::snapshot`] is a cheap
//! read of the latest known player state (safe to call every frame) and
//! [`MediaSource::send`] queues a [`Command`] without waiting for it. The
//! snapshot is updated optimistically at once ([`Snapshot::apply`]), so a
//! key press shows on the very next frame; the player's real answer
//! replaces it a moment later.
//!
//! [`detect`] picks the backend for this platform. Everything here is
//! platform-neutral except the backends, each behind its own `cfg`:
//!
//! - [`worker`]: [`Polled`], the source used for real players: a background
//!   thread that asks a blocking [`worker::Backend`] for the state on an
//!   adaptive cadence and runs commands as they arrive. Nothing here ever
//!   blocks the caller. A new backend implements `Backend` and gets the
//!   threading, optimistic state and smoothing for free.
//! - macOS: [`players`] asks the Spotify desktop app (`spotify`) and Apple
//!   Music (`apple_music`) through one long-lived `osascript` process
//!   (`applescript`: the shared script loop and record format; `runner`:
//!   requests over stdin, replies with a timeout). It never launches a
//!   player.
//! - [`mpris`] (Linux): any MPRIS player on the session bus, over zbus.
//! - [`smtc`] (Windows): the System Media Transport Controls sessions.
//! - [`choice`]: which player each of them follows when several are open:
//!   Spotify first only while it plays, else whatever plays, else the one
//!   in use.
//! - [`fake`]: [`FakeSource`], an in-memory player for tests and for
//!   building the UI without a real one.
//! - [`art`]: [`ArtLoader`](art::ArtLoader), album covers fetched, cached
//!   and decoded on their own thread.
//!
//! No terminal code, and no I/O on the caller's thread.

#[cfg(target_os = "macos")]
pub mod apple_music;
#[cfg(target_os = "macos")]
pub mod applescript;
pub mod art;
#[cfg(any(target_os = "macos", target_os = "linux", windows, test))]
pub mod choice;
pub mod fake;
// Whether a player honours shuffle / repeat (MPRIS players, Apple Music).
#[cfg(any(target_os = "linux", target_os = "macos", test))]
pub mod modes;
// The pure parts of the Linux and Windows backends are tested everywhere
// (elsewhere the rest of each is unused).
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub mod mpris;
#[cfg(target_os = "macos")]
pub mod notify;
// The choosing is pure and tested everywhere.
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub mod players;
#[cfg(target_os = "macos")]
pub mod runner;
#[cfg(any(windows, test))]
#[cfg_attr(not(windows), allow(dead_code))]
pub mod smtc;
#[cfg(target_os = "macos")]
pub mod spotify;
pub mod worker;

use std::sync::Arc;
use std::time::{Duration, Instant};

pub use fake::FakeSource;
pub use worker::Polled;

/// A music player the UI can show and control.
pub trait MediaSource: Send {
    /// The latest known state. Cheap (a lock and a few `Arc` clones), never
    /// waits on the player.
    fn snapshot(&self) -> Snapshot;

    /// Queue a command. Returns at once; the snapshot reflects the expected
    /// result immediately and the player's real state shortly after.
    fn send(&self, command: Command);

    /// Which optional controls actually work with this player.
    fn capabilities(&self) -> Capabilities {
        Capabilities::ALL
    }

    #[cfg(test)]
    fn play_pause(&self) {
        self.send(Command::PlayPause);
    }
    #[cfg(test)]
    fn next(&self) {
        self.send(Command::Next);
    }
    #[cfg(test)]
    fn set_shuffle(&self, on: bool) {
        self.send(Command::SetShuffle(on));
    }
    /// Volume 0..=100 (larger values are clamped). The MPRIS live tests'.
    #[cfg(all(test, target_os = "linux"))]
    fn set_volume(&self, volume: u8) {
        self.send(Command::SetVolume(volume.min(100)));
    }
    /// Play a URI the player understands (`spotify:track:…`, an album or
    /// playlist URI, …). Malformed URIs are ignored; ones the player
    /// doesn't know do nothing.
    #[cfg(test)]
    fn play_uri(&self, uri: &str) {
        if let Some(command) = Command::play_uri(uri) {
            self.send(command);
        }
    }
}

/// The media source for this platform: Spotify and Apple Music
/// (AppleScript) on macOS, MPRIS on Linux, SMTC on Windows; elsewhere a source that is always
/// `Unavailable(Unsupported)`. Starts the backend's worker thread; cheap to
/// call, never blocks.
pub fn detect() -> Box<dyn MediaSource> {
    #[cfg(target_os = "macos")]
    {
        Box::new(Polled::spawn(
            players::Players::detect(),
            worker::Cadence::default(),
        ))
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(Polled::spawn(
            mpris::Mpris::new(),
            worker::Cadence::default(),
        ))
    }
    #[cfg(windows)]
    {
        Box::new(Polled::spawn(smtc::Smtc::new(), worker::Cadence::default()))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        Box::new(Polled::unavailable(Unavailable::Unsupported))
    }
}

/// Optional controls a player may not honour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// `SetShuffle` takes effect (and `Snapshot::shuffle` means something).
    pub shuffle: bool,
    /// `SetRepeat` takes effect.
    pub repeat: bool,
    /// `SetVolume` takes effect (and `Snapshot::volume` means something;
    /// Windows' media controls have no volume).
    pub volume: bool,
    /// `PlayUri` / `PlayInContext` start something (Windows' media
    /// controls can't be told what to play).
    pub uris: bool,
    /// `PlayInContext` carries on through the rest of the playlist; where
    /// it's off, the track plays alone (MPRIS has no contexts).
    pub contexts: bool,
}

impl Capabilities {
    pub const ALL: Self = Self {
        shuffle: true,
        repeat: true,
        volume: true,
        uris: true,
        contexts: true,
    };
    pub const NONE: Self = Self {
        shuffle: false,
        repeat: false,
        volume: false,
        uris: false,
        contexts: false,
    };
}

/// Something to ask the player to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    /// Jump to this position in the current track.
    Seek(Duration),
    SetShuffle(bool),
    SetRepeat(bool),
    /// 0..=100.
    SetVolume(u8),
    /// A validated `spotify:` URI; build it with [`Command::play_uri`].
    PlayUri(String),
    /// A track inside its playlist or album (both validated URIs); build
    /// it with [`Command::play_in_context`].
    PlayInContext {
        track: String,
        context: String,
    },
}

impl Command {
    /// `PlayUri` for a well-formed URI (`scheme:rest`, e.g.
    /// `spotify:album:…`, `https://…`), else `None`. Only URI-safe ASCII is
    /// accepted (no quotes, backslashes, spaces or control characters), so
    /// backends can pass it to a script or a bus as is.
    pub fn play_uri(uri: &str) -> Option<Self> {
        let uri = uri.trim();
        let (scheme, rest) = uri.split_once(':')?;
        let scheme_ok = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
        let safe = |c: char| c.is_ascii_alphanumeric() || ":/_-.%+?=&~#@!$*,;".contains(c);
        (scheme_ok && !rest.is_empty() && rest.chars().all(safe))
            .then(|| Self::PlayUri(uri.to_owned()))
    }

    /// `PlayInContext` when both are well-formed URIs (as
    /// [`Command::play_uri`]), else `None`.
    pub fn play_in_context(track: &str, context: &str) -> Option<Self> {
        let (Self::PlayUri(track), Self::PlayUri(context)) =
            (Self::play_uri(track)?, Self::play_uri(context)?)
        else {
            return None;
        };
        Some(Self::PlayInContext { track, context })
    }

    /// The URI this plays (the track, for one in a context).
    #[cfg(test)]
    pub fn uri(&self) -> Option<&str> {
        match self {
            Self::PlayUri(uri) | Self::PlayInContext { track: uri, .. } => Some(uri),
            _ => None,
        }
    }
}

/// What the player is doing, or why there is no player to ask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not asked yet (the first answer is on its way).
    Connecting,
    Playing,
    Paused,
    /// Running, nothing loaded.
    Stopped,
    Unavailable(Unavailable),
}

impl Status {
    /// The player answered: play state, track and settings are real.
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Playing | Self::Paused | Self::Stopped)
    }
}

/// [`Snapshot::player_name`] when the backend doesn't say.
const UNNAMED_PLAYER: &str = "The player";

/// Why the player can't be shown or controlled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// No backend for this platform yet.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS: no osascript
    Unsupported,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only macOS can tell
    NotInstalled,
    /// Installed but not open (or no player at all). We never launch one.
    NotRunning,
    /// The OS refused us control of the player (on macOS: the Automation
    /// permission, AppleScript error -1743).
    #[cfg_attr(windows, allow(dead_code))]
    PermissionDenied,
    /// The player didn't answer in time (busy, starting up, or macOS is
    /// waiting on a permission prompt).
    #[cfg_attr(windows, allow(dead_code))]
    NotResponding,
    /// Anything else, with the underlying message.
    Error(String),
}

impl Unavailable {
    /// One calm sentence for the UI. `player` names the player
    /// ([`Snapshot::player_name`]).
    pub fn message(&self, player: &str) -> String {
        self.message_for(player, "music")
    }

    /// [`Self::message`] for a widget showing `what` (`music`, `album
    /// art`, `lyrics`): the next step names what it brings back.
    pub fn message_for(&self, player: &str, what: &str) -> String {
        match self {
            Self::Unsupported => "No media player support on this platform yet".into(),
            Self::NotInstalled => format!("{player} is not installed"),
            Self::NotRunning if player == UNNAMED_PLAYER => {
                format!("Open your music player to show {what}")
            }
            Self::NotRunning => format!("Open {player} to show {what}"),
            #[cfg(target_os = "macos")]
            Self::PermissionDenied => format!(
                "Allow control of {player}: System Settings › Privacy & Security › \
                 Automation › your terminal › {player}"
            ),
            #[cfg(not(target_os = "macos"))]
            Self::PermissionDenied => format!("Not allowed to control {player}"),
            Self::NotResponding if cfg!(target_os = "macos") => {
                format!("{player} isn't answering (if macOS asks to allow control, choose Allow)")
            }
            Self::NotResponding => format!("{player} isn't answering"),
            Self::Error(message) => message.clone(),
        }
    }
}

/// The track that is loaded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Track {
    /// The player's own identifier: Spotify's URI or Music's persistent
    /// ID on macOS, an MPRIS
    /// object path (what seeking needs), or the title, artist and album
    /// where the player gives none (Windows). Tells tracks apart; not
    /// something to hand to the Web API (that's [`Track::uri`]).
    pub id: String,
    /// The Spotify track this is (`spotify:track:…`), when the player says
    /// so ([`spotify_track_uri`]); `None` for local files, ads, episodes
    /// and other players. Windows never says (see
    /// `Model::playing_uri`).
    pub uri: Option<String>,
    pub name: String,
    pub artist: String,
    pub album: String,
    /// Zero when the player doesn't say.
    pub duration: Duration,
    /// Cover art URL (may be empty: local files, some ads).
    pub artwork_url: String,
}

/// The canonical Spotify track URI (`spotify:track:<id>`) for the ways
/// players name one: the URI itself, an `open.spotify.com/track/<id>`
/// link (any `?si=` ignored), or Spotify's MPRIS object path
/// `/com/spotify/track/<id>`. Anything else (a local file, an ad, an
/// episode, another player's id) is `None`: never a guess.
pub fn spotify_track_uri(text: &str) -> Option<String> {
    let text = text.trim();
    let link = || {
        let rest = text
            .strip_prefix("https://open.spotify.com/")
            .or_else(|| text.strip_prefix("http://open.spotify.com/"))?;
        // Links may carry a locale first (`intl-de/track/…`).
        let rest = match rest.split_once('/') {
            Some((first, tail)) if first.starts_with("intl-") => tail,
            _ => rest,
        };
        let id = rest.strip_prefix("track/")?;
        Some(id.split(['?', '#']).next().unwrap_or(id))
    };
    let id = text
        .strip_prefix("spotify:track:")
        .or_else(|| text.strip_prefix("/com/spotify/track/"))
        .or_else(link)?;
    let base62 = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric());
    base62.then(|| format!("spotify:track:{id}"))
}

/// The player's state as last known.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// Which player this is ("Spotify"), once the backend knows.
    pub player: Option<Arc<str>>,
    pub status: Status,
    pub track: Option<Arc<Track>>,
    /// Playback position at `sampled_at`; use [`Snapshot::position_at`].
    pub position: Duration,
    pub sampled_at: Instant,
    pub shuffle: bool,
    pub repeat: bool,
    /// 0..=100.
    pub volume: u8,
}

impl Snapshot {
    pub fn new(status: Status, now: Instant) -> Self {
        Self {
            player: None,
            status,
            track: None,
            position: Duration::ZERO,
            sampled_at: now,
            shuffle: false,
            repeat: false,
            volume: 0,
        }
    }

    /// The player's name, or a generic one.
    pub fn player_name(&self) -> &str {
        self.player.as_deref().unwrap_or(UNNAMED_PLAYER)
    }

    /// The player is a Spotify app (by the name its backend gives it).
    pub fn is_spotify(&self) -> bool {
        self.player
            .as_deref()
            .is_some_and(|name| name.to_ascii_lowercase().starts_with("spotify"))
    }

    /// Why there's nothing to show, as one sentence (`None` when available
    /// or still connecting).
    pub fn unavailable_message(&self) -> Option<String> {
        match &self.status {
            Status::Unavailable(reason) => Some(reason.message(self.player_name())),
            _ => None,
        }
    }

    /// Position extrapolated to `now`: advances while playing, never past
    /// the end of the track. Lets the UI tick smoothly between polls.
    pub fn position_at(&self, now: Instant) -> Duration {
        let mut position = self.position;
        if self.status == Status::Playing {
            position += now.saturating_duration_since(self.sampled_at);
        }
        match self.duration() {
            Some(duration) => position.min(duration),
            None => position,
        }
    }

    /// 0..=1 through the track at `now`, if its length is known.
    pub fn progress_at(&self, now: Instant) -> Option<f64> {
        let duration = self.duration()?;
        Some(self.position_at(now).as_secs_f64() / duration.as_secs_f64())
    }

    fn duration(&self) -> Option<Duration> {
        self.track
            .as_ref()
            .map(|track| track.duration)
            .filter(|duration| !duration.is_zero())
    }

    /// The expected effect of `command`, applied at `now` (optimistic UI).
    /// Does nothing while the player is unavailable.
    pub fn apply(&mut self, command: &Command, now: Instant) {
        if !self.status.is_available() {
            return;
        }
        match command {
            Command::PlayPause => {
                let position = self.position_at(now);
                self.status = match self.status {
                    Status::Playing => Status::Paused,
                    _ => Status::Playing,
                };
                self.rebase(position, now);
            }
            Command::Next | Command::Previous => self.rebase(Duration::ZERO, now),
            Command::Seek(to) => {
                let to = self.duration().map_or(*to, |duration| (*to).min(duration));
                self.rebase(to, now);
            }
            Command::SetShuffle(on) => self.shuffle = *on,
            Command::SetRepeat(on) => self.repeat = *on,
            Command::SetVolume(volume) => self.volume = (*volume).min(100),
            Command::PlayUri(_) | Command::PlayInContext { .. } => {
                self.status = Status::Playing;
                self.rebase(Duration::ZERO, now);
            }
        }
    }

    fn rebase(&mut self, position: Duration, now: Instant) {
        self.position = position;
        self.sampled_at = now;
    }

    /// The same playback carrying on: same track, playing in both. Used to
    /// keep the extrapolated clock when a fresh sample barely differs.
    pub(crate) fn continues(&self, previous: &Self) -> bool {
        self.status == Status::Playing
            && previous.status == Status::Playing
            && self.track.as_ref().map(|t| &t.id) == previous.track.as_ref().map(|t| &t.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    fn playing(now: Instant) -> Snapshot {
        Snapshot {
            track: Some(Arc::new(Track {
                id: "spotify:track:a".into(),
                duration: S * 100,
                ..Track::default()
            })),
            position: S * 10,
            volume: 50,
            ..Snapshot::new(Status::Playing, now)
        }
    }

    #[test]
    fn position_extrapolates_only_while_playing_and_stops_at_the_end() {
        let t0 = Instant::now();
        let mut snap = playing(t0);
        assert_eq!(snap.position_at(t0 + S * 5), S * 15);
        assert_eq!(snap.position_at(t0 + S * 500), S * 100);
        assert_eq!(snap.progress_at(t0 + S * 40), Some(0.5));
        // A clock that reads earlier than the sample doesn't go backwards.
        assert_eq!(snap.position_at(t0.checked_sub(S).unwrap_or(t0)), S * 10);
        snap.status = Status::Paused;
        assert_eq!(snap.position_at(t0 + S * 5), S * 10);
    }

    #[test]
    fn unknown_duration_is_not_a_cap() {
        let t0 = Instant::now();
        let mut snap = playing(t0);
        snap.track = Some(Arc::new(Track::default()));
        assert_eq!(snap.position_at(t0 + S * 500), S * 510);
        assert_eq!(snap.progress_at(t0), None);
    }

    #[test]
    fn apply_is_the_expected_effect() {
        let t0 = Instant::now();
        let mut snap = playing(t0);
        snap.apply(&Command::PlayPause, t0 + S * 2);
        assert_eq!(snap.status, Status::Paused);
        assert_eq!(snap.position_at(t0 + S * 9), S * 12); // frozen at pause
        snap.apply(&Command::PlayPause, t0 + S * 9);
        assert_eq!(snap.status, Status::Playing);
        assert_eq!(snap.position_at(t0 + S * 10), S * 13);

        snap.apply(&Command::Seek(S * 1000), t0);
        assert_eq!(snap.position, S * 100);
        snap.apply(&Command::Next, t0);
        assert_eq!(snap.position, Duration::ZERO);
        snap.apply(&Command::SetVolume(250), t0);
        assert_eq!(snap.volume, 100);
        snap.apply(&Command::SetShuffle(true), t0);
        snap.apply(&Command::SetRepeat(true), t0);
        assert!(snap.shuffle && snap.repeat);
    }

    #[test]
    fn apply_does_nothing_without_a_player() {
        let t0 = Instant::now();
        let mut snap = Snapshot::new(Status::Unavailable(Unavailable::NotRunning), t0);
        let before = snap.clone();
        snap.apply(&Command::PlayPause, t0);
        snap.apply(&Command::SetVolume(10), t0);
        assert_eq!(snap, before);
    }

    #[test]
    fn play_uri_accepts_only_plain_spotify_uris() {
        assert_eq!(
            Command::play_uri(" spotify:track:0DZXVpUtPUom1VO6h5a0SU "),
            Some(Command::PlayUri(
                "spotify:track:0DZXVpUtPUom1VO6h5a0SU".into()
            ))
        );
        assert!(Command::play_uri("spotify:user:me:playlist:37i9dQ").is_some());
        assert!(Command::play_uri("https://open.spotify.com/track/x?si=1").is_some());
        for bad in [
            "",
            "spotify:",
            "no-scheme",
            ":empty-scheme",
            "1abc:x",
            "spotify:track:x\" & do shell script \"rm",
            "spotify:track:x\\",
            "spotify:track:é",
            "spotify:track:a b",
        ] {
            assert_eq!(Command::play_uri(bad), None, "{bad:?}");
            assert_eq!(Command::play_in_context(bad, "spotify:playlist:p"), None);
            assert_eq!(Command::play_in_context("spotify:track:t", bad), None);
        }
        let both = Command::play_in_context(" spotify:track:t", "spotify:playlist:p ");
        assert_eq!(
            both,
            Some(Command::PlayInContext {
                track: "spotify:track:t".into(),
                context: "spotify:playlist:p".into(),
            })
        );
        assert_eq!(both.unwrap().uri(), Some("spotify:track:t"));
    }

    #[test]
    fn spotify_track_uris_are_normalised_never_guessed() {
        let uri = Some("spotify:track:4uLU6hMCjMI75M1A2tKUQC".to_owned());
        for named in [
            "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
            " /com/spotify/track/4uLU6hMCjMI75M1A2tKUQC ",
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=abc123",
            "https://open.spotify.com/intl-de/track/4uLU6hMCjMI75M1A2tKUQC",
        ] {
            assert_eq!(spotify_track_uri(named), uri, "{named:?}");
        }
        for not in [
            "",
            "spotify:track:",
            "spotify:local:Artist:Album:Song:215",
            "spotify:ad:000000012c4a1bd4",
            "spotify:episode:4uLU6hMCjMI75M1A2tKUQC",
            "/com/spotify/ad/000000012c4a1bd4",
            "/org/mpris/MediaPlayer2/Track/7",
            "https://open.spotify.com/episode/4uLU6hMCjMI75M1A2tKUQC",
            "https://example.com/track/4uLU6hMCjMI75M1A2tKUQC",
            "file:///home/someone/Music/song.flac",
            "spotify:track:a b",
            "Song\u{1f}Artist\u{1f}Album",
        ] {
            assert_eq!(spotify_track_uri(not), None, "{not:?}");
        }
    }

    #[test]
    fn spotify_players_by_name() {
        let mut snap = Snapshot::new(Status::Playing, Instant::now());
        assert!(!snap.is_spotify());
        for (name, spotify) in [("Spotify", true), ("spotify", true), ("VLC", false)] {
            snap.player = Some(name.into());
            assert_eq!(snap.is_spotify(), spotify, "{name}");
        }
    }

    #[test]
    fn every_unavailable_reason_has_a_message() {
        for reason in [
            Unavailable::Unsupported,
            Unavailable::NotInstalled,
            Unavailable::NotRunning,
            Unavailable::PermissionDenied,
            Unavailable::NotResponding,
            Unavailable::Error("boom".into()),
        ] {
            assert!(!reason.message("Spotify").is_empty());
        }
        assert_eq!(
            Unavailable::NotRunning.message("Spotify"),
            "Open Spotify to show music"
        );
        #[cfg(target_os = "macos")]
        assert!(
            Unavailable::PermissionDenied
                .message("Spotify")
                .contains("Automation › your terminal › Spotify")
        );
        let mut snap = Snapshot::new(Status::Playing, Instant::now());
        assert_eq!(snap.unavailable_message(), None);
        snap.status = Status::Unavailable(Unavailable::NotRunning);
        assert_eq!(
            snap.unavailable_message().as_deref(),
            Some("Open your music player to show music")
        );
    }
}
