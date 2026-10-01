//! Windows: the System Media Transport Controls (the media flyout's
//! sessions), Spotify's session preferred.
//!
//! [`Smtc`] (Windows only) is a [`Backend`](super::worker::Backend) over
//! `GlobalSystemMediaTransportControlsSessionManager` (the `windows`
//! crate), its async calls waited on with `join` on the worker thread.
//! Everything else here is pure and platform-neutral (so its tests run
//! anywhere): picking a session ([`choose`], [`app_name`]), status codes
//! ([`status`]), the timeline ([`position`]) and the track ([`track`]).
//!
//! What SMTC can't do: volume (no such control: `Capabilities::volume` is
//! off), cover art as a URL (it hands out a thumbnail stream, so there's
//! no art yet), playing a URI (that would mean launching the player).
//!
//! Untested on real Windows: it is built and its mapping is unit tested,
//! nothing more.

use std::time::Duration;

use super::{Status, Track};

/// 100 ns ticks (`TimeSpan`, `DateTime`) per millisecond.
const TICKS_PER_MS: i64 = 10_000;
/// From 1601-01-01 (Windows `DateTime`) to 1970-01-01, in ticks.
pub const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// The session to show, by app id: Spotify's if there is one, else the
/// one Windows calls current (`None`).
pub fn choose<'a>(app_ids: impl IntoIterator<Item = &'a str>) -> Option<usize> {
    app_ids.into_iter().position(is_spotify)
}

fn is_spotify(app_id: &str) -> bool {
    app_id.to_ascii_lowercase().contains("spotify")
}

/// A display name from an app user model id: `Spotify.exe` → "Spotify",
/// `Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic` → "ZuneMusic",
/// `chrome` → "Chrome".
pub fn app_name(app_id: &str) -> String {
    if is_spotify(app_id) {
        return "Spotify".into();
    }
    let app = app_id.rsplit('!').next().unwrap_or(app_id);
    let app = app.strip_suffix(".exe").unwrap_or(app);
    let app = app.split('_').next().unwrap_or(app);
    let base = app.rsplit('.').next().unwrap_or(app);
    let mut chars = base.chars();
    chars.next().map_or_else(
        || "The player".into(),
        |first| first.to_uppercase().chain(chars).collect(),
    )
}

/// `GlobalSystemMediaTransportControlsSessionPlaybackStatus` as a status.
pub fn status(code: i32) -> Status {
    match code {
        4 => Status::Playing,
        5 => Status::Paused,
        // Closed, Opened, Changing, Stopped.
        _ => Status::Stopped,
    }
}

/// Where playback is now: the timeline's position (`TimeSpan` ticks) was
/// true at `last_updated` (`DateTime` ticks); a playing session has moved
/// on since `now` (`DateTime` ticks). Kept within `start..=end` when the
/// end is known.
pub fn position(
    position: i64,
    start: i64,
    end: i64,
    last_updated: i64,
    now: i64,
    playing: bool,
) -> Duration {
    let mut at = position.saturating_sub(start);
    if playing && last_updated > 0 {
        at = at.saturating_add(now.saturating_sub(last_updated).max(0));
    }
    if end > start {
        at = at.min(end - start);
    }
    ticks(at)
}

/// The length of the timeline, zero when unknown.
pub fn duration(start: i64, end: i64) -> Duration {
    ticks(end.saturating_sub(start))
}

fn ticks(ticks: i64) -> Duration {
    Duration::from_millis((ticks / TICKS_PER_MS).max(0) as u64)
}

/// The track from the session's media properties, if it has a title. SMTC
/// has no id, so what's playing is what identifies it.
pub fn track(title: &str, artist: &str, album: &str, duration: Duration) -> Option<Track> {
    (!title.is_empty()).then(|| Track {
        id: format!("{title}\u{1f}{artist}\u{1f}{album}"),
        name: title.to_owned(),
        artist: artist.to_owned(),
        album: album.to_owned(),
        duration,
        artwork_url: String::new(),
    })
}

/// A `TimeSpan` for a seek target.
pub fn to_ticks(at: Duration) -> i64 {
    i64::try_from(at.as_millis())
        .unwrap_or(i64::MAX / TICKS_PER_MS)
        .saturating_mul(TICKS_PER_MS)
}

#[cfg(windows)]
pub use backend::Smtc;

#[cfg(windows)]
mod backend {
    use std::sync::Arc;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
    };
    use windows::Media::MediaPlaybackAutoRepeatMode;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

    use super::{UNIX_EPOCH_TICKS, app_name, choose, duration, position, status, to_ticks, track};
    use crate::media::worker::Backend;
    use crate::media::{Capabilities, Command, Snapshot, Status, Unavailable};

    /// The SMTC backend. Initialises WinRT on the worker thread on first
    /// use and keeps the session manager.
    #[derive(Default)]
    pub struct Smtc {
        initialised: bool,
        manager: Option<Manager>,
    }

    impl Smtc {
        pub fn new() -> Self {
            Self::default()
        }

        fn manager(&mut self) -> windows::core::Result<Manager> {
            if !self.initialised {
                // Already initialised (another apartment type) is fine too.
                let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
                self.initialised = true;
            }
            if self.manager.is_none() {
                self.manager = Some(Manager::RequestAsync()?.join()?);
            }
            Ok(self.manager.clone().expect("just requested"))
        }

        fn try_exchange(
            &mut self,
            commands: &[Command],
        ) -> windows::core::Result<(Snapshot, Option<Arc<str>>)> {
            let manager = self.manager()?;
            let Some(session) = session(&manager)? else {
                let snap =
                    Snapshot::new(Status::Unavailable(Unavailable::NotRunning), Instant::now());
                return Ok((snap, None));
            };
            let name: Arc<str> = Arc::from(app_name(&session.SourceAppUserModelId()?.to_string()));
            for command in commands {
                // A command the app refuses doesn't spoil the read.
                let _ = send(&session, command);
            }
            let info = session.GetPlaybackInfo()?;
            let status = status(info.PlaybackStatus()?.0);
            let timeline = session.GetTimelineProperties()?;
            let (start, end) = (timeline.StartTime()?.Duration, timeline.EndTime()?.Duration);
            let now_ticks = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, to_ticks)
                .saturating_add(UNIX_EPOCH_TICKS);
            let at = position(
                timeline.Position()?.Duration,
                start,
                end,
                timeline.LastUpdatedTime()?.UniversalTime,
                now_ticks,
                status == Status::Playing,
            );
            let props = session.TryGetMediaPropertiesAsync()?.join()?;
            let track = track(
                &props.Title()?.to_string(),
                &props.Artist()?.to_string(),
                &props.AlbumTitle()?.to_string(),
                duration(start, end),
            );
            let shuffle = info
                .IsShuffleActive()
                .and_then(|v| v.Value())
                .unwrap_or(false);
            let repeat = info
                .AutoRepeatMode()
                .and_then(|v| v.Value())
                .is_ok_and(|mode| mode != MediaPlaybackAutoRepeatMode::None);
            let snap = Snapshot {
                player: None,
                track: track.map(Arc::new),
                position: at,
                shuffle,
                repeat,
                ..Snapshot::new(status, Instant::now())
            };
            Ok((snap, Some(name)))
        }
    }

    impl Backend for Smtc {
        fn exchange(&mut self, commands: &[Command]) -> Snapshot {
            match self.try_exchange(commands) {
                Ok((mut snap, name)) => {
                    snap.player = name;
                    snap
                }
                Err(err) => {
                    self.manager = None;
                    Snapshot::new(
                        Status::Unavailable(Unavailable::Error(format!(
                            "media controls: {}",
                            err.message()
                        ))),
                        Instant::now(),
                    )
                }
            }
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                volume: false,
                ..Capabilities::ALL
            }
        }
    }

    /// Spotify's session, else the current one, else none.
    fn session(manager: &Manager) -> windows::core::Result<Option<Session>> {
        let sessions = manager.GetSessions()?;
        let mut ids = Vec::new();
        for i in 0..sessions.Size()? {
            ids.push(sessions.GetAt(i)?.SourceAppUserModelId()?.to_string());
        }
        if let Some(i) = choose(ids.iter().map(String::as_str)) {
            return Ok(Some(sessions.GetAt(i as u32)?));
        }
        // No current session is an error here, not a session.
        Ok(manager.GetCurrentSession().ok())
    }

    fn send(session: &Session, command: &Command) -> windows::core::Result<bool> {
        match command {
            Command::PlayPause => session.TryTogglePlayPauseAsync()?.join(),
            Command::Next => session.TrySkipNextAsync()?.join(),
            Command::Previous => session.TrySkipPreviousAsync()?.join(),
            Command::Seek(to) => session
                .TryChangePlaybackPositionAsync(to_ticks(*to))?
                .join(),
            Command::SetShuffle(on) => session.TryChangeShuffleActiveAsync(*on)?.join(),
            Command::SetRepeat(on) => session
                .TryChangeAutoRepeatModeAsync(if *on {
                    MediaPlaybackAutoRepeatMode::List
                } else {
                    MediaPlaybackAutoRepeatMode::None
                })?
                .join(),
            // No such controls.
            Command::SetVolume(_) | Command::PlayUri(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = TICKS_PER_MS;

    #[test]
    fn spotify_is_preferred() {
        let ids = ["chrome", "Spotify.exe", "MSEdge"];
        assert_eq!(choose(ids), Some(1));
        assert_eq!(
            choose(["SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"]),
            Some(0)
        );
        assert_eq!(choose(["chrome", "MSEdge"]), None);
        assert_eq!(choose([]), None);
    }

    #[test]
    fn app_names() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(
            app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
            "Spotify"
        );
        assert_eq!(
            app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "ZuneMusic"
        );
        assert_eq!(app_name("chrome"), "Chrome");
        assert_eq!(app_name("vlc.exe"), "Vlc");
        assert_eq!(app_name(""), "The player");
    }

    #[test]
    fn status_codes() {
        assert_eq!(status(4), Status::Playing);
        assert_eq!(status(5), Status::Paused);
        for code in [0, 1, 2, 3, 99, -1] {
            assert_eq!(status(code), Status::Stopped);
        }
    }

    #[test]
    fn position_moves_on_while_playing_within_the_track() {
        let ms = Duration::from_millis;
        // 10 s in at t=1000 s; asked 2.5 s later.
        let (pos, upd, now) = (10_000 * MS, 1_000_000 * MS, 1_002_500 * MS);
        assert_eq!(position(pos, 0, 300_000 * MS, upd, now, true), ms(12_500));
        assert_eq!(position(pos, 0, 300_000 * MS, upd, now, false), ms(10_000));
        // Never past the end; the start is subtracted.
        assert_eq!(position(pos, 0, 11_000 * MS, upd, now, true), ms(11_000));
        assert_eq!(position(pos, 4_000 * MS, 0, upd, now, false), ms(6_000));
        // A clock behind the update or no update time: no extrapolation.
        assert_eq!(position(pos, 0, 0, upd, upd - MS, true), ms(10_000));
        assert_eq!(position(pos, 0, 0, 0, now, true), ms(10_000));
        assert_eq!(position(-5, 0, 0, 0, 0, false), Duration::ZERO);
        assert_eq!(duration(0, 303_440 * MS), ms(303_440));
        assert_eq!(duration(5, 0), Duration::ZERO);
    }

    #[test]
    fn tracks_need_a_title_and_are_identified_by_their_text() {
        let t = track(
            "Life",
            "Dreamcatcher",
            "Dreamcatcher",
            Duration::from_secs(303),
        )
        .unwrap();
        assert_eq!(t.id, "Life\u{1f}Dreamcatcher\u{1f}Dreamcatcher");
        assert_eq!(t.artwork_url, "");
        assert_eq!(track("", "x", "y", Duration::ZERO), None);
    }

    #[test]
    fn seek_targets_are_ticks() {
        assert_eq!(to_ticks(Duration::from_millis(61_250)), 612_500_000);
        assert_eq!(to_ticks(Duration::MAX), i64::MAX / MS * MS);
    }
}
