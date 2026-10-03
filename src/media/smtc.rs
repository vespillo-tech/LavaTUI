//! Windows: the System Media Transport Controls (the media flyout's
//! sessions), Spotify's first while it plays ([`choice`](super::choice)).
//!
//! [`Smtc`] (Windows only) is a [`Backend`](super::worker::Backend) over
//! `GlobalSystemMediaTransportControlsSessionManager` (the `windows`
//! crate), its async calls waited on with `join` on the worker thread.
//! Everything else here is pure and platform-neutral (so its tests run
//! anywhere): picking a session ([`choose`], [`app_name`]), status codes
//! ([`status`]), the timeline ([`position`]) and the track ([`track`]).
//!
//! What SMTC can't do: volume (no such control: `Capabilities::volume` is
//! off), playing a URI (`Capabilities::uris` is off: the library plays
//! through the Web API or says why it can't), or naming the Spotify track
//! (no URI; the model matches the Web API's player to the track). Cover art
//! comes as a thumbnail stream, not a URL: the worker reads it once per
//! track ([`Cover`]) and hands the bytes to the art loader
//! ([`art::stash`](super::art::stash), lava-75z.16).
//!
//! Untested on real Windows: it is built and its mapping is unit tested,
//! nothing more.

use std::time::Duration;

use super::{Status, Track};

/// 100 ns ticks (`TimeSpan`, `DateTime`) per millisecond.
const TICKS_PER_MS: i64 = 10_000;
/// From 1601-01-01 (Windows `DateTime`) to 1970-01-01, in ticks.
pub const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// One SMTC session, as far as choosing goes.
#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a> {
    pub app_id: &'a str,
    pub playing: bool,
    /// The one Windows calls current.
    pub current: bool,
}

/// The session to show, by the shared rule ([`choice`](super::choice):
/// Spotify while it plays, else whatever plays, else the one in use, else
/// the one Windows calls current). `in_use` is the app id followed last.
pub fn choose<'a>(
    sessions: impl IntoIterator<Item = Candidate<'a>>,
    in_use: Option<&str>,
) -> Option<usize> {
    super::choice::pick(sessions.into_iter().map(|s| super::choice::Player {
        spotify: is_spotify(s.app_id),
        playing: s.playing,
        current: s.current,
        in_use: in_use == Some(s.app_id),
    }))
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
/// has no id, so what's playing is what identifies it, and no Spotify URI
/// (the model matches the Web API's player to it instead,
/// lava-1xk.24).
pub fn track(title: &str, artist: &str, album: &str, duration: Duration) -> Option<Track> {
    (!title.is_empty()).then(|| Track {
        id: format!("{title}\u{1f}{artist}\u{1f}{album}"),
        uri: None,
        name: title.to_owned(),
        artist: artist.to_owned(),
        album: album.to_owned(),
        duration,
        artwork_url: String::new(),
    })
}

/// Reads of a track's thumbnail while it has none (players often set it a
/// moment after the title).
const COVER_TRIES: u8 = 3;

/// The playing track's cover: its thumbnail read once per track (a few
/// times while there's none yet) and stashed for the art loader.
#[derive(Debug, Default)]
pub struct Cover {
    track: String,
    url: String,
    tries: u8,
}

impl Cover {
    /// The cover URL (`lavatui-thumb:…`, or empty) for `track_id`, asking
    /// `read` for the thumbnail's bytes when it's due.
    pub fn url(&mut self, track_id: &str, read: impl FnOnce() -> Option<Vec<u8>>) -> String {
        if self.track != track_id {
            *self = Self {
                track: track_id.to_owned(),
                ..Self::default()
            };
        }
        if self.url.is_empty() && self.tries < COVER_TRIES {
            self.tries += 1;
            self.url = read().and_then(super::art::stash).unwrap_or_default();
        }
        self.url.clone()
    }
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
        GlobalSystemMediaTransportControlsSessionMediaProperties as Properties,
    };
    use windows::Media::MediaPlaybackAutoRepeatMode;
    use windows::Storage::Streams::DataReader;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::core::{IUnknown, Interface};

    use super::{
        Candidate, Cover, UNIX_EPOCH_TICKS, app_name, choose, duration, position, status, to_ticks,
        track,
    };
    use windows::Foundation::TypedEventHandler;

    use crate::media::worker::{Backend, Nudge};
    use crate::media::{Capabilities, Command, Snapshot, Status, Unavailable};

    /// The SMTC backend. Initialises WinRT on the worker thread on first
    /// use and keeps the session manager.
    #[derive(Default)]
    pub struct Smtc {
        initialised: bool,
        manager: Option<Manager>,
        cover: Cover,
        /// The app id of the session followed last.
        in_use: Option<String>,
        /// Where change events go ([`Backend::listen`]).
        nudge: Option<Nudge>,
        /// The manager's and the session's event handlers, while set.
        events: Option<Events>,
    }

    /// Handlers on the session in use (play / pause, position, track) and
    /// on the manager (players coming and going), each nudging the worker.
    /// Removed when dropped (another session chosen, the manager lost).
    struct Events {
        manager: (Manager, i64),
        session: Option<(String, Session, [i64; 3])>,
    }

    impl Drop for Events {
        fn drop(&mut self) {
            let _ = self.manager.0.RemoveSessionsChanged(self.manager.1);
            if let Some((_, session, [a, b, c])) = self.session.take() {
                let _ = session.RemovePlaybackInfoChanged(a);
                let _ = session.RemoveTimelinePropertiesChanged(b);
                let _ = session.RemoveMediaPropertiesChanged(c);
            }
        }
    }

    /// A handler that nudges.
    fn nudging<S, A>(nudge: &Nudge) -> TypedEventHandler<S, A>
    where
        S: windows::core::RuntimeType + 'static,
        A: windows::core::RuntimeType + 'static,
    {
        let nudge = nudge.clone();
        TypedEventHandler::new(move |_, _| {
            nudge.changed();
            Ok(())
        })
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
            let Some(session) = session(&manager, self.in_use.as_deref())? else {
                self.in_use = None;
                let snap =
                    Snapshot::new(Status::Unavailable(Unavailable::NotRunning), Instant::now());
                return Ok((snap, None));
            };
            let id = session.SourceAppUserModelId()?.to_string();
            self.follow(&manager, &id, &session);
            let name: Arc<str> = Arc::from(app_name(&id));
            self.in_use = Some(id);
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
            let mut track = track(
                &props.Title()?.to_string(),
                &props.Artist()?.to_string(),
                &props.AlbumTitle()?.to_string(),
                duration(start, end),
            );
            if let Some(t) = &mut track {
                t.artwork_url = self.cover.url(&t.id, || thumbnail(&props).ok());
            }
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

        /// Keep event handlers on `manager` and on `session` (app `id`),
        /// moving them when another session is in use. Best effort: a
        /// handler that can't be set leaves polling to catch the change.
        fn follow(&mut self, manager: &Manager, id: &str, session: &Session) {
            let Some(nudge) = &self.nudge else { return };
            if self
                .events
                .as_ref()
                .is_some_and(|e| &e.manager.0 != manager)
            {
                self.events = None;
            }
            if self.events.is_none() {
                let Ok(token) = manager.SessionsChanged(&nudging(nudge)) else {
                    return;
                };
                self.events = Some(Events {
                    manager: (manager.clone(), token),
                    session: None,
                });
            }
            let Some(events) = &mut self.events else {
                return;
            };
            if events.session.as_ref().is_some_and(|(at, ..)| at == id) {
                return;
            }
            if let Some((_, old, [a, b, c])) = events.session.take() {
                let _ = old.RemovePlaybackInfoChanged(a);
                let _ = old.RemoveTimelinePropertiesChanged(b);
                let _ = old.RemoveMediaPropertiesChanged(c);
            }
            let tokens = (
                session.PlaybackInfoChanged(&nudging(nudge)),
                session.TimelinePropertiesChanged(&nudging(nudge)),
                session.MediaPropertiesChanged(&nudging(nudge)),
            );
            if let (Ok(a), Ok(b), Ok(c)) = tokens {
                events.session = Some((id.to_owned(), session.clone(), [a, b, c]));
            }
        }
    }

    impl Backend for Smtc {
        /// The session's own change events (set on the next exchange, on
        /// the worker thread, and moved with the session in use).
        fn listen(&mut self, nudge: Nudge) -> Option<Box<dyn Send>> {
            self.nudge = Some(nudge);
            None
        }

        fn exchange(&mut self, commands: &[Command]) -> Snapshot {
            match self.try_exchange(commands) {
                Ok((mut snap, name)) => {
                    snap.player = name;
                    snap
                }
                Err(err) => {
                    self.events = None;
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

        /// No volume, and nothing to tell the app what to play.
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                volume: false,
                uris: false,
                contexts: false,
                ..Capabilities::ALL
            }
        }
    }

    /// The thumbnail's encoded bytes (JPEG / PNG), up to the art loader's
    /// cap; an error when there's none.
    fn thumbnail(props: &Properties) -> windows::core::Result<Vec<u8>> {
        const MAX: u64 = 8 * 1024 * 1024;
        let stream = props.Thumbnail()?.OpenReadAsync()?.join()?;
        let size = stream.Size()?.min(MAX) as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
        let loaded = reader.LoadAsync(size)?.join()?;
        let mut bytes = vec![0; loaded as usize];
        reader.ReadBytes(&mut bytes)?;
        Ok(bytes)
    }

    /// The session to follow ([`choose`]), or none.
    fn session(manager: &Manager, in_use: Option<&str>) -> windows::core::Result<Option<Session>> {
        // No current session is an error here, not a session.
        let current = manager.GetCurrentSession().ok();
        let current = current.as_ref().and_then(identity);
        let sessions = manager.GetSessions()?;
        let mut found = Vec::new();
        for i in 0..sessions.Size()? {
            let session = sessions.GetAt(i)?;
            let id = session.SourceAppUserModelId()?.to_string();
            let playing = session
                .GetPlaybackInfo()
                .and_then(|info| info.PlaybackStatus())
                .is_ok_and(|code| status(code.0) == Status::Playing);
            let is_current = current.is_some() && identity(&session) == current;
            found.push((session, id, playing, is_current));
        }
        let candidates = found.iter().map(|(_, id, playing, current)| Candidate {
            app_id: id,
            playing: *playing,
            current: *current,
        });
        Ok(choose(candidates, in_use).map(|i| found.swap_remove(i).0))
    }

    /// The session's COM identity (its `IUnknown`'s address).
    fn identity(session: &Session) -> Option<usize> {
        let unknown = session.cast::<IUnknown>().ok()?;
        Some(unknown.as_raw() as usize)
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
            // No such controls (and `capabilities` says so: the model
            // never sends them).
            Command::SetVolume(_) | Command::PlayUri(_) | Command::PlayInContext { .. } => {
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = TICKS_PER_MS;

    fn at<'a>(app_id: &'a str, playing: bool, current: bool) -> Candidate<'a> {
        Candidate {
            app_id,
            playing,
            current,
        }
    }

    #[test]
    fn spotify_first_while_it_plays_then_whatever_plays() {
        let spotify = "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify";
        // Playing: Spotify, over the current session too.
        let all = [at("chrome", true, true), at(spotify, true, false)];
        assert_eq!(choose(all, None), Some(1));
        // Idle, it doesn't win over a player that plays (codex review #3),
        // nor over the one Windows calls current.
        let idle = [at("Spotify.exe", false, false), at("vlc.exe", true, false)];
        assert_eq!(choose(idle, None), Some(1));
        assert_eq!(choose(idle, Some("Spotify.exe")), Some(1));
        let quiet = [at("Spotify.exe", false, false), at("MSEdge", false, true)];
        assert_eq!(choose(quiet, None), Some(1));
        // Nothing playing: the one in use keeps the keys.
        assert_eq!(choose(quiet, Some("Spotify.exe")), Some(0));
        let none = [at("chrome", false, false), at("MSEdge", false, false)];
        assert_eq!(choose(none, None), Some(0));
        assert_eq!(choose([], None), None);
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
    fn the_cover_is_read_once_per_track_and_retried_while_missing() {
        let png = {
            let img = image::RgbImage::from_pixel(4, 4, image::Rgb([9, 90, 200]));
            let mut out = std::io::Cursor::new(Vec::new());
            img.write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        let mut cover = Cover::default();
        let reads = std::cell::Cell::new(0);
        let read = |bytes: Option<Vec<u8>>| {
            reads.set(reads.get() + 1);
            bytes
        };
        // No thumbnail yet: asked again on the next polls, then given up.
        for _ in 0..5 {
            assert_eq!(cover.url("a", || read(None)), "");
        }
        assert_eq!(reads.get(), COVER_TRIES);
        // A new track: read once, then the same URL without reading.
        let url = cover.url("b", || read(Some(png.clone())));
        assert!(url.starts_with(crate::media::art::THUMB_SCHEME), "{url}");
        assert_eq!(cover.url("b", || read(Some(Vec::new()))), url);
        assert_eq!(reads.get(), COVER_TRIES + 1);

        // The URL loads as the cover: the bytes went through the stash.
        let mut loader = crate::media::art::ArtLoader::spawn(NoFetch, None);
        loader.want(&url);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let art = loop {
            match loader.get() {
                crate::media::art::ArtState::Ready(art) => break art,
                _ if std::time::Instant::now() > deadline => panic!("no cover"),
                _ => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        assert_eq!(art.mean(), crate::theme::Rgb(9, 90, 200));
    }

    struct NoFetch;

    impl crate::media::art::Fetch for NoFetch {
        fn get(&mut self, url: &str) -> Result<Vec<u8>, String> {
            panic!("fetched {url}")
        }
    }

    #[test]
    fn seek_targets_are_ticks() {
        assert_eq!(to_ticks(Duration::from_millis(61_250)), 612_500_000);
        assert_eq!(to_ticks(Duration::MAX), i64::MAX / MS * MS);
    }
}
