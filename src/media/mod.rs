//! Now playing: what a music player is doing, and remote control of it.
//!
//! The UI talks to a [`MediaSource`]: [`MediaSource::snapshot`] is a cheap
//! read of the latest known player state (safe to call every frame) and
//! [`MediaSource::send`] queues a [`Command`] without waiting for it. The
//! snapshot is updated optimistically at once ([`Snapshot::apply`]), so a
//! key press shows on the very next frame; the player's real answer
//! replaces it a moment later.
//!
//! - [`worker`]: [`Polled`], the source used for real players: a background
//!   thread that asks a blocking [`worker::Backend`] for the state on an
//!   adaptive cadence and runs commands as they arrive. Nothing here ever
//!   blocks the caller.
//! - [`spotify`]: the Spotify desktop app on macOS, through `osascript`
//!   ([`runner`] runs it with a timeout). It never launches Spotify.
//! - [`fake`]: [`FakeSource`], an in-memory player for tests and for
//!   building the UI without a real one.
//!
//! Pure apart from [`runner::Osascript`]: no terminal code, no I/O on the
//! caller's thread.

pub mod fake;
pub mod runner;
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

    fn play_pause(&self) {
        self.send(Command::PlayPause);
    }
    fn next(&self) {
        self.send(Command::Next);
    }
    fn previous(&self) {
        self.send(Command::Previous);
    }
    fn seek(&self, to: Duration) {
        self.send(Command::Seek(to));
    }
    fn set_shuffle(&self, on: bool) {
        self.send(Command::SetShuffle(on));
    }
    fn set_repeat(&self, on: bool) {
        self.send(Command::SetRepeat(on));
    }
    /// Volume 0..=100 (larger values are clamped).
    fn set_volume(&self, volume: u8) {
        self.send(Command::SetVolume(volume.min(100)));
    }
    /// Play a `spotify:` URI (track, album, playlist, …). URIs that aren't
    /// plain `spotify:` identifiers are ignored.
    fn play_uri(&self, uri: &str) {
        if let Some(command) = Command::play_uri(uri) {
            self.send(command);
        }
    }
}

/// The Spotify desktop app, or an unavailable source off macOS.
pub fn spotify() -> Box<dyn MediaSource> {
    #[cfg(target_os = "macos")]
    {
        Box::new(Polled::spawn(
            spotify::Spotify::new(runner::Osascript),
            worker::Cadence::default(),
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(Polled::unavailable(Unavailable::Unsupported))
    }
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
}

impl Command {
    /// `PlayUri` for a well-formed `spotify:` URI (`spotify:track:…`,
    /// `spotify:album:…`, `spotify:user:…:playlist:…`), else `None`. Only
    /// URI-safe ASCII is accepted, so the URI can go into a script as is.
    pub fn play_uri(uri: &str) -> Option<Self> {
        let uri = uri.trim();
        let rest = uri.strip_prefix("spotify:")?;
        let safe = |c: char| c.is_ascii_alphanumeric() || ":_-.%+".contains(c);
        (!rest.is_empty() && rest.chars().all(safe)).then(|| Self::PlayUri(uri.to_owned()))
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

/// Why the player can't be shown or controlled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// No backend for this platform (Spotify control needs macOS).
    Unsupported,
    NotInstalled,
    /// Installed but not open. We never launch it.
    NotRunning,
    /// macOS Automation permission was refused (AppleScript error -1743).
    PermissionDenied,
    /// The player didn't answer in time (busy, starting up, or macOS is
    /// waiting on a permission prompt).
    NotResponding,
    /// Anything else, with the underlying message.
    Error(String),
}

impl Unavailable {
    /// One calm sentence for the UI.
    pub fn message(&self) -> &str {
        match self {
            Self::Unsupported => "Spotify control needs macOS",
            Self::NotInstalled => "Spotify isn't installed",
            Self::NotRunning => "Spotify isn't running",
            Self::PermissionDenied => {
                "Allow control of Spotify: System Settings › Privacy & Security › \
                 Automation › your terminal › Spotify"
            }
            Self::NotResponding => {
                "Spotify isn't answering (if macOS asks to allow control, choose Allow)"
            }
            Self::Error(message) => message,
        }
    }
}

/// The track that is loaded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Track {
    /// Player-specific identifier (`spotify:track:…`).
    pub id: String,
    pub name: String,
    pub artist: String,
    pub album: String,
    /// Zero when the player doesn't say.
    pub duration: Duration,
    /// Cover art URL (may be empty: local files, some ads).
    pub artwork_url: String,
}

/// The player's state as last known.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
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
            status,
            track: None,
            position: Duration::ZERO,
            sampled_at: now,
            shuffle: false,
            repeat: false,
            volume: 0,
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
            Command::PlayUri(_) => {
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
        for bad in [
            "",
            "spotify:",
            "https://open.spotify.com/track/x",
            "spotify:track:x\" & do shell script \"rm",
            "spotify:track:x\\",
            "spotify:track:é",
            "spotify:track:a b",
        ] {
            assert_eq!(Command::play_uri(bad), None, "{bad:?}");
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
            assert!(!reason.message().is_empty());
        }
        assert!(
            Unavailable::PermissionDenied
                .message()
                .contains("Automation")
        );
    }
}
