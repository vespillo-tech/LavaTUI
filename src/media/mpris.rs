//! Linux: any MPRIS player on the D-Bus session bus, Spotify preferred.
//!
//! [`Mpris`] (Linux only) is a [`Backend`](super::worker::Backend) over
//! zbus's blocking API: each exchange lists the bus names, picks a player
//! ([`choose`]), sends the commands and reads every `Player` property in
//! one `GetAll`. Everything else here is pure and platform-neutral (so its
//! tests run anywhere): D-Bus values arrive as [`Meta`], [`snapshot`]
//! turns them into a [`Snapshot`] and [`plan`] turns a [`Command`] into a
//! D-Bus [`Call`].
//!
//! Players differ in what they implement (Spotify has long reported
//! `Position` as 0 and ignored `Shuffle`/`LoopStatus`); a property that's
//! missing or the wrong type just falls back to a neutral value, and a
//! command the player refuses is ignored.
//!
//! Tested on Linux against `tools/linux/fake_mpris.py` (a player with
//! Spotify's quirks too) on a private session bus: the `live` tests,
//! `tools/linux/run.sh`. Not yet against real players on a desktop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Command, Snapshot, Status, Track};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const SPOTIFY: &str = "org.mpris.MediaPlayer2.spotify";
/// `mpris:trackid` when nothing is loaded.
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

/// A D-Bus value, as much of it as MPRIS needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Meta {
    /// A string or an object path.
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<Meta>),
    Map(HashMap<String, Meta>),
    Other,
}

impl Meta {
    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    /// Integers, and the floats some players send instead.
    fn int(&self) -> Option<i64> {
        match *self {
            Self::Int(n) => Some(n),
            Self::Float(f) if f.is_finite() => Some(f as i64),
            _ => None,
        }
    }

    fn float(&self) -> Option<f64> {
        match *self {
            Self::Float(f) if f.is_finite() => Some(f),
            Self::Int(n) => Some(n as f64),
            _ => None,
        }
    }

    /// A list of strings joined with ", " (`xesam:artist` is `as`), or a
    /// plain string (some players send one).
    fn joined(&self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text.clone()),
            Self::List(items) => {
                let texts: Vec<&str> = items.iter().filter_map(Self::text).collect();
                Some(texts.join(", "))
            }
            _ => None,
        }
    }
}

/// The player to show: Spotify if it's on the bus, else the first MPRIS
/// player by name (so the choice is stable).
pub fn choose<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut players: Vec<&str> = names
        .into_iter()
        .filter(|name| name.starts_with(PREFIX) && name.len() > PREFIX.len())
        .collect();
    players.sort_unstable();
    players
        .iter()
        .find(|name| **name == SPOTIFY || name.starts_with(&format!("{SPOTIFY}.")))
        .or(players.first())
        .copied()
}

/// A name for the player when it has no `Identity`: the bus name's last
/// part, minus a `.instance123` suffix, capitalised (`vlc` → "Vlc").
pub fn fallback_name(bus_name: &str) -> String {
    let rest = bus_name.strip_prefix(PREFIX).unwrap_or(bus_name);
    let base = rest.split('.').next().unwrap_or(rest);
    let mut chars = base.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The `Player` interface's properties (from `GetAll`) as a snapshot
/// sampled at `now`. Missing or odd properties fall back to neutral values.
pub fn snapshot(props: &HashMap<String, Meta>, now: Instant) -> Snapshot {
    let status = match props.get("PlaybackStatus").and_then(Meta::text) {
        Some("Playing") => Status::Playing,
        Some("Paused") => Status::Paused,
        _ => Status::Stopped,
    };
    let track = match props.get("Metadata") {
        Some(Meta::Map(metadata)) => track(metadata).map(Arc::new),
        _ => None,
    };
    let status = match (&status, &track) {
        // Paused with nothing loaded is how some players say stopped.
        (Status::Paused, None) => Status::Stopped,
        _ => status,
    };
    let position = props
        .get("Position")
        .and_then(Meta::int)
        .map_or(Duration::ZERO, micros);
    let volume = props
        .get("Volume")
        .and_then(Meta::float)
        .map_or(0, |v| (v * 100.0).round().clamp(0.0, 100.0) as u8);
    Snapshot {
        player: None,
        status,
        track,
        position,
        sampled_at: now,
        shuffle: props.get("Shuffle") == Some(&Meta::Bool(true)),
        repeat: matches!(
            props.get("LoopStatus").and_then(Meta::text),
            Some("Track" | "Playlist")
        ),
        volume,
    }
}

/// The track in an MPRIS `Metadata` map, if one is loaded.
pub fn track(metadata: &HashMap<String, Meta>) -> Option<Track> {
    let text = |key: &str| metadata.get(key).and_then(Meta::joined).unwrap_or_default();
    let name = text("xesam:title");
    let artist = text("xesam:artist");
    let album = text("xesam:album");
    let id = match metadata.get("mpris:trackid").and_then(Meta::text) {
        Some(id) if id != NO_TRACK && !id.is_empty() => id.to_owned(),
        // No id (some browsers): what's playing is what identifies it.
        _ if !name.is_empty() => format!("{name}\u{1f}{artist}\u{1f}{album}"),
        _ => return None,
    };
    // Spotify names its track in `xesam:url` (older versions only in the
    // trackid); a local file, an ad or another player names none.
    let uri = ["xesam:url", "mpris:trackid"]
        .iter()
        .filter_map(|key| metadata.get(*key).and_then(Meta::text))
        .find_map(super::spotify_track_uri);
    Some(Track {
        id,
        uri,
        name,
        artist,
        album,
        duration: metadata
            .get("mpris:length")
            .and_then(Meta::int)
            .map_or(Duration::ZERO, micros),
        artwork_url: text("mpris:artUrl"),
    })
}

/// Players that always report `Position` 0 (Spotify, for years) would
/// send the progress bar back to the start every poll: while the same
/// track stays loaded, playing or paused, a 0 means "unknown" and the last
/// read's position, moved on to now, stands. `last` has our own commands
/// applied ([`Snapshot::apply`]), so a seek, pause or restart (`Previous`)
/// sent from here moves it too; one made in the player itself shows at
/// the next track change.
pub fn keep_position(fresh: &mut Snapshot, last: Option<&Snapshot>) {
    let loaded = |snap: &Snapshot| matches!(snap.status, Status::Playing | Status::Paused);
    if let Some(last) = last
        && fresh.position.is_zero()
        && loaded(fresh)
        && loaded(last)
        && fresh.track.is_some()
        && fresh.track.as_ref().map(|t| &t.id) == last.track.as_ref().map(|t| &t.id)
    {
        fresh.position = last.position_at(fresh.sampled_at);
    }
}

fn micros(us: i64) -> Duration {
    Duration::from_micros(us.max(0) as u64)
}

/// One D-Bus call on the player.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    /// A `Player` method without arguments (`PlayPause`, `Next`, …).
    Method(&'static str),
    /// `SetPosition(track, µs)`: needs the track id as an object path.
    SetPosition(String, i64),
    /// `Seek(offset µs)`, relative: for a track without a usable id.
    Seek(i64),
    SetBool(&'static str, bool),
    SetText(&'static str, &'static str),
    SetFloat(&'static str, f64),
    OpenUri(String),
}

/// How to ask for `command`, given the last snapshot (`seek` needs to know
/// the track, or where playback is, at `now`).
pub fn plan(command: &Command, last: Option<&Snapshot>, now: Instant) -> Call {
    match command {
        Command::PlayPause => Call::Method("PlayPause"),
        Command::Next => Call::Method("Next"),
        Command::Previous => Call::Method("Previous"),
        Command::Seek(to) => {
            let to_us = to.as_micros().min(i64::MAX as u128) as i64;
            match last.and_then(|snap| snap.track.as_ref().map(|t| (snap, t))) {
                Some((_, track)) if is_object_path(&track.id) => {
                    Call::SetPosition(track.id.clone(), to_us)
                }
                Some((snap, _)) => {
                    let at = snap.position_at(now).as_micros().min(i64::MAX as u128) as i64;
                    Call::Seek(to_us - at)
                }
                None => Call::SetPosition(NO_TRACK.into(), to_us),
            }
        }
        Command::SetShuffle(on) => Call::SetBool("Shuffle", *on),
        Command::SetRepeat(on) => {
            Call::SetText("LoopStatus", if *on { "Playlist" } else { "None" })
        }
        Command::SetVolume(volume) => {
            Call::SetFloat("Volume", f64::from((*volume).min(100)) / 100.0)
        }
        Command::PlayUri(uri) => Call::OpenUri(uri.clone()),
        // MPRIS has no contexts: the track alone.
        Command::PlayInContext { track, .. } => Call::OpenUri(track.clone()),
    }
}

/// A valid D-Bus object path (`/`, or `/`-separated `[A-Za-z0-9_]+`).
fn is_object_path(text: &str) -> bool {
    text == "/"
        || text.strip_prefix('/').is_some_and(|rest| {
            rest.split('/').all(|part| {
                !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
}

#[cfg(target_os = "linux")]
pub use bus::Mpris;

#[cfg(target_os = "linux")]
mod bus {
    use std::collections::HashMap;
    use std::io;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use zbus::blocking::Connection;
    use zbus::zvariant::{OwnedValue, Value};

    use super::{Call, Meta, choose, fallback_name, keep_position, plan, snapshot};
    use crate::media::worker::Backend;
    use crate::media::{Capabilities, Command, Snapshot, Status, Unavailable};

    const PATH: &str = "/org/mpris/MediaPlayer2";
    const ROOT: &str = "org.mpris.MediaPlayer2";
    const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
    const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
    /// Any one call; a player that takes longer is "not responding".
    const TIMEOUT: Duration = Duration::from_secs(3);

    /// The MPRIS backend. Connects on first use and again after the bus
    /// connection breaks.
    #[derive(Default)]
    pub struct Mpris {
        conn: Option<Connection>,
        /// The player last used: bus name and display name.
        player: Option<(String, Arc<str>)>,
        last: Option<Snapshot>,
    }

    impl Mpris {
        pub fn new() -> Self {
            Self::default()
        }

        fn connection(&mut self) -> zbus::Result<&Connection> {
            if self.conn.is_none() {
                let conn = zbus::blocking::connection::Builder::session()?
                    .method_timeout(TIMEOUT)
                    .build()?;
                self.conn = Some(conn);
            }
            Ok(self.conn.as_ref().expect("just connected"))
        }

        fn try_exchange(&mut self, commands: &[Command]) -> zbus::Result<Snapshot> {
            let conn = self.connection()?.clone();
            let names: Vec<String> = conn
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "ListNames",
                    &(),
                )?
                .body()
                .deserialize()?;
            let Some(bus) = choose(names.iter().map(String::as_str)).map(str::to_owned) else {
                self.player = None;
                return Ok(Snapshot::new(
                    Status::Unavailable(Unavailable::NotRunning),
                    Instant::now(),
                ));
            };
            if self.player.as_ref().is_none_or(|(name, _)| *name != bus) {
                let identity = get(&conn, &bus, ROOT, "Identity")
                    .ok()
                    .and_then(|v| String::try_from(v).ok())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| fallback_name(&bus));
                self.player = Some((bus.clone(), Arc::from(identity)));
                self.last = None;
            }
            for command in commands {
                let now = Instant::now();
                // A command the player refuses doesn't spoil the read.
                if call(&conn, &bus, &plan(command, self.last.as_ref(), now)).is_ok()
                    && let Some(last) = &mut self.last
                {
                    // What it did, for `keep_position` to carry on from.
                    last.apply(command, now);
                }
            }
            let props: HashMap<String, OwnedValue> = conn
                .call_method(
                    Some(bus.as_str()),
                    PATH,
                    Some(PROPERTIES),
                    "GetAll",
                    &PLAYER,
                )?
                .body()
                .deserialize()?;
            let props = props.iter().map(|(k, v)| (k.clone(), meta(v))).collect();
            let mut snap = snapshot(&props, Instant::now());
            keep_position(&mut snap, self.last.as_ref());
            Ok(snap)
        }
    }

    impl Backend for Mpris {
        fn exchange(&mut self, commands: &[Command]) -> Snapshot {
            let mut snap = match self.try_exchange(commands) {
                Ok(snap) => snap,
                Err(err) => {
                    let reason = classify(&err);
                    if matches!(err, zbus::Error::InputOutput(_) | zbus::Error::Address(_)) {
                        self.conn = None;
                    }
                    Snapshot::new(Status::Unavailable(reason), Instant::now())
                }
            };
            snap.player = self.player.as_ref().map(|(_, name)| Arc::clone(name));
            self.last = snap.status.is_available().then(|| snap.clone());
            snap
        }

        /// MPRIS has all of them (whether a player honours them varies)
        /// but no contexts: a track in its playlist plays alone.
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                contexts: false,
                ..Capabilities::ALL
            }
        }
    }

    fn get(conn: &Connection, bus: &str, iface: &str, name: &str) -> zbus::Result<OwnedValue> {
        conn.call_method(Some(bus), PATH, Some(PROPERTIES), "Get", &(iface, name))?
            .body()
            .deserialize()
    }

    fn set(conn: &Connection, bus: &str, name: &str, value: Value<'_>) -> zbus::Result<()> {
        conn.call_method(
            Some(bus),
            PATH,
            Some(PROPERTIES),
            "Set",
            &(PLAYER, name, value),
        )?;
        Ok(())
    }

    fn call(conn: &Connection, bus: &str, call: &Call) -> zbus::Result<()> {
        match call {
            Call::Method(method) => conn
                .call_method(Some(bus), PATH, Some(PLAYER), *method, &())
                .map(drop),
            Call::SetPosition(track, us) => {
                let path = zbus::zvariant::ObjectPath::try_from(track.as_str())?;
                conn.call_method(Some(bus), PATH, Some(PLAYER), "SetPosition", &(path, *us))
                    .map(drop)
            }
            Call::Seek(us) => conn
                .call_method(Some(bus), PATH, Some(PLAYER), "Seek", &(*us,))
                .map(drop),
            Call::SetBool(name, on) => set(conn, bus, name, Value::Bool(*on)),
            Call::SetText(name, text) => set(conn, bus, name, Value::from(*text)),
            Call::SetFloat(name, v) => set(conn, bus, name, Value::F64(*v)),
            Call::OpenUri(uri) => conn
                .call_method(Some(bus), PATH, Some(PLAYER), "OpenUri", &(uri.as_str(),))
                .map(drop),
        }
    }

    fn meta(value: &Value<'_>) -> Meta {
        match value {
            Value::Str(s) => Meta::Text(s.to_string()),
            Value::ObjectPath(p) => Meta::Text(p.to_string()),
            Value::Bool(b) => Meta::Bool(*b),
            Value::U8(n) => Meta::Int((*n).into()),
            Value::I16(n) => Meta::Int((*n).into()),
            Value::U16(n) => Meta::Int((*n).into()),
            Value::I32(n) => Meta::Int((*n).into()),
            Value::U32(n) => Meta::Int((*n).into()),
            Value::I64(n) => Meta::Int(*n),
            Value::U64(n) => Meta::Int(i64::try_from(*n).unwrap_or(i64::MAX)),
            Value::F64(f) => Meta::Float(*f),
            Value::Value(inner) => meta(inner),
            Value::Array(items) => Meta::List(items.iter().map(meta).collect()),
            Value::Dict(dict) => Meta::Map(
                dict.iter()
                    .filter_map(|(k, v)| match k {
                        Value::Str(k) => Some((k.to_string(), meta(v))),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => Meta::Other,
        }
    }

    fn classify(err: &zbus::Error) -> Unavailable {
        let name = match err {
            zbus::Error::MethodError(name, _, _) => name.as_str().to_owned(),
            zbus::Error::FDO(fdo) => match **fdo {
                zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_) => {
                    return Unavailable::NotRunning;
                }
                zbus::fdo::Error::NoReply(_) | zbus::fdo::Error::Timeout(_) => {
                    return Unavailable::NotResponding;
                }
                zbus::fdo::Error::AccessDenied(_) => return Unavailable::PermissionDenied,
                _ => String::new(),
            },
            zbus::Error::InputOutput(io) if io.kind() == io::ErrorKind::TimedOut => {
                return Unavailable::NotResponding;
            }
            _ => String::new(),
        };
        match name.rsplit('.').next() {
            Some("ServiceUnknown" | "NameHasNoOwner") => Unavailable::NotRunning,
            Some("NoReply" | "Timeout" | "TimedOut") => Unavailable::NotResponding,
            Some("AccessDenied") => Unavailable::PermissionDenied,
            _ => Unavailable::Error(format!("MPRIS: {err}")),
        }
    }
}

/// The backend against `tools/linux/fake_mpris.py` on a real session bus:
/// `tools/linux/run.sh mpris` (Docker), or on a Linux desktop
/// `dbus-run-session -- cargo test mpris::live -- --ignored --test-threads=1`
/// (needs python3-dbus-next; a private bus keeps real players out of it).
#[cfg(all(test, target_os = "linux"))]
mod live {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command as Process, Stdio};
    use std::thread;

    use super::*;
    use crate::media::worker::testing::{fast, wait_for};
    use crate::media::worker::{Backend, Polled};
    use crate::media::{MediaSource, Unavailable};

    const S: Duration = Duration::from_secs(1);
    const MS: Duration = Duration::from_millis(1);
    const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tools/linux/fake_mpris.py");

    /// A running fake player; killed (and so off the bus) on drop.
    struct Fake(Child);

    impl Fake {
        fn start(args: &[&str]) -> Self {
            let mut child = Process::new("python3")
                .arg(FAKE)
                .args(args)
                .stdout(Stdio::piped())
                .spawn()
                .expect("python3 tools/linux/fake_mpris.py");
            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let mut line = String::new();
            stdout.read_line(&mut line).unwrap();
            assert!(line.starts_with("ready "), "fake player said {line:?}");
            // Keep draining its call log so it never blocks on a full pipe.
            thread::spawn(move || for _ in stdout.lines() {});
            Self(child)
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn not_running(snap: &Snapshot) -> bool {
        snap.status == Status::Unavailable(Unavailable::NotRunning)
    }

    fn title(snap: &Snapshot) -> &str {
        snap.track.as_ref().map_or("", |t| t.name.as_str())
    }

    /// Off the bus: the name is released when the process's connection
    /// closes, a moment after the kill.
    fn gone(mpris: &mut Mpris) -> Snapshot {
        for _ in 0..200 {
            let snap = mpris.exchange(&[]);
            if not_running(&snap) {
                return snap;
            }
            thread::sleep(MS * 10);
        }
        panic!("player still on the bus");
    }

    #[test]
    #[ignore = "needs a D-Bus session bus and python3-dbus-next (tools/linux/run.sh mpris)"]
    fn every_field_and_command_round_trips() {
        let mut mpris = Mpris::new();
        assert!(not_running(&mpris.exchange(&[])), "no player yet");

        let fake = Fake::start(&[]);
        let snap = mpris.exchange(&[]);
        assert_eq!(snap.player.as_deref(), Some("Fake Player"));
        assert_eq!(snap.status, Status::Playing);
        let track = snap.track.as_deref().unwrap();
        assert_eq!(track.id, "/org/lavatui/fake/track/t0");
        assert_eq!(track.name, "Slow Bloom");
        assert_eq!(track.artist, "The Wax Hearts");
        assert_eq!(track.album, "Lamp Light");
        assert_eq!(track.duration, S * 241);
        assert_eq!(track.artwork_url, "file:///nonexistent/lavatui-fake-t0.png");
        assert!(
            (S * 12..S * 15).contains(&snap.position),
            "{:?}",
            snap.position
        );
        assert_eq!(snap.volume, 50);
        assert!(!snap.shuffle && !snap.repeat);

        let snap = mpris.exchange(&[Command::PlayPause]);
        assert_eq!(snap.status, Status::Paused);
        let paused_at = snap.position;
        thread::sleep(MS * 200);
        assert_eq!(
            mpris.exchange(&[]).position,
            paused_at,
            "paused stands still"
        );
        assert_eq!(
            mpris.exchange(&[Command::PlayPause]).status,
            Status::Playing
        );

        let snap = mpris.exchange(&[Command::Next]);
        assert_eq!(title(&snap), "Convection");
        assert_eq!(snap.track.as_ref().unwrap().artist, "Mara Vell, Ode Kiri");
        assert!(snap.position < S, "a new track starts at 0");
        assert_eq!(title(&mpris.exchange(&[Command::Previous])), "Slow Bloom");

        // SetPosition with the track's id.
        let snap = mpris.exchange(&[Command::Seek(S * 60)]);
        assert!(
            (S * 60..S * 61).contains(&snap.position),
            "{:?}",
            snap.position
        );

        let snap = mpris.exchange(&[Command::SetShuffle(true), Command::SetRepeat(true)]);
        assert!(snap.shuffle && snap.repeat);
        let snap = mpris.exchange(&[Command::SetShuffle(false), Command::SetRepeat(false)]);
        assert!(!snap.shuffle && !snap.repeat);
        assert_eq!(mpris.exchange(&[Command::SetVolume(42)]).volume, 42);

        let snap = mpris.exchange(&[Command::play_uri("fake:song").unwrap()]);
        assert_eq!(title(&snap), "Opened fake:song");
        let in_context = Command::play_in_context("fake:other", "fake:album").unwrap();
        assert_eq!(title(&mpris.exchange(&[in_context])), "Opened fake:other");

        drop(fake);
        let snap = gone(&mut mpris);
        assert_eq!(snap.player, None);
    }

    #[test]
    #[ignore = "needs a D-Bus session bus and python3-dbus-next (tools/linux/run.sh mpris)"]
    fn without_a_track_id_seek_is_relative() {
        let _fake = Fake::start(&["--no-trackid", "--paused"]);
        let mut mpris = Mpris::new();
        let snap = mpris.exchange(&[]);
        assert_eq!(snap.status, Status::Paused);
        assert_eq!(
            snap.track.as_ref().unwrap().id,
            "Slow Bloom\u{1f}The Wax Hearts\u{1f}Lamp Light"
        );
        assert_eq!(snap.position, S * 12);
        // Seek(offset) from where it is: 12 s → 100 s.
        assert_eq!(mpris.exchange(&[Command::Seek(S * 100)]).position, S * 100);
        assert_eq!(mpris.exchange(&[Command::Seek(S * 30)]).position, S * 30);
    }

    #[test]
    #[ignore = "needs a D-Bus session bus and python3-dbus-next (tools/linux/run.sh mpris)"]
    fn spotify_is_preferred_and_its_quirks_handled() {
        let _other = Fake::start(&["--name", "aplayer"]);
        let _spotify = Fake::start(&["--spotify"]);
        let mut mpris = Mpris::new();
        let snap = mpris.exchange(&[]);
        // Chosen over "aplayer", which sorts first.
        assert_eq!(snap.player.as_deref(), Some("Spotify"));
        assert_eq!(snap.track.as_ref().unwrap().id, "/com/spotify/track/faket0");
        // Position always reads 0: unknown on the first read ...
        assert_eq!(snap.position, Duration::ZERO);
        thread::sleep(MS * 300);
        // ... then carried on from the last read while the track plays.
        let snap = mpris.exchange(&[]);
        assert!(
            (MS * 250..S).contains(&snap.position),
            "{:?}",
            snap.position
        );

        // A seek isn't undone by the next read's 0.
        let snap = mpris.exchange(&[Command::Seek(S * 100)]);
        assert!(
            (S * 100..S * 101).contains(&snap.position),
            "{:?}",
            snap.position
        );
        thread::sleep(MS * 200);
        let snap = mpris.exchange(&[]);
        assert!(
            (S * 100..S * 101).contains(&snap.position),
            "{:?}",
            snap.position
        );
        // Nor is a pause, or the resume after it.
        let snap = mpris.exchange(&[Command::PlayPause]);
        assert_eq!(snap.status, Status::Paused);
        assert!(
            (S * 100..S * 101).contains(&snap.position),
            "{:?}",
            snap.position
        );
        let snap = mpris.exchange(&[Command::PlayPause]);
        assert_eq!(snap.status, Status::Playing);
        assert!(
            (S * 100..S * 101).contains(&snap.position),
            "{:?}",
            snap.position
        );
        // A new track does start at 0.
        let snap = mpris.exchange(&[Command::Next]);
        assert_eq!(snap.track.as_ref().unwrap().id, "/com/spotify/track/faket1");
        assert_eq!(snap.position, Duration::ZERO);

        // Shuffle / repeat are accepted and ignored: the read says so.
        let snap = mpris.exchange(&[Command::SetShuffle(true), Command::SetRepeat(true)]);
        assert!(!snap.shuffle && !snap.repeat);
        // Volume works.
        assert_eq!(mpris.exchange(&[Command::SetVolume(70)]).volume, 70);
    }

    /// The whole source, as the app uses it: the worker thread, optimistic
    /// commands, the player going away and coming back.
    #[test]
    #[ignore = "needs a D-Bus session bus and python3-dbus-next (tools/linux/run.sh mpris)"]
    fn polled_source_follows_the_player() {
        let source = Polled::spawn(Mpris::new(), fast());
        wait_for(&source, "no player", not_running);
        let fake = Fake::start(&[]);
        wait_for(&source, "the track", |s| title(s) == "Slow Bloom");
        source.next();
        wait_for(&source, "next", |s| title(s) == "Convection");
        source.play_pause();
        wait_for(&source, "paused", |s| s.status == Status::Paused);
        source.set_volume(10);
        wait_for(&source, "volume", |s| s.volume == 10);
        drop(fake);
        wait_for(&source, "player gone", not_running);
        let _fake = Fake::start(&["--spotify"]);
        wait_for(&source, "spotify", |s| {
            s.player.as_deref() == Some("Spotify") && title(s) == "Slow Bloom"
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    fn text(s: &str) -> Meta {
        Meta::Text(s.into())
    }

    /// As Spotify 1.2 on Linux sends it.
    fn spotify_metadata() -> HashMap<String, Meta> {
        [
            (
                "mpris:trackid",
                text("/com/spotify/track/0DZXVpUtPUom1VO6h5a0SU"),
            ),
            ("mpris:length", Meta::Int(303_440_000)),
            (
                "mpris:artUrl",
                text("https://i.scdn.co/image/ab67616d0000b273cb5ed04a1191ccec6717e76d"),
            ),
            ("xesam:album", text("Dreamcatcher")),
            ("xesam:albumArtist", Meta::List(vec![text("Dreamcatcher")])),
            (
                "xesam:artist",
                Meta::List(vec![text("Sigur Rós"), text("Jónsi")]),
            ),
            ("xesam:title", text("Life")),
            ("xesam:trackNumber", Meta::Int(3)),
            (
                "xesam:url",
                text("https://open.spotify.com/track/0DZXVpUtPUom1VO6h5a0SU"),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect()
    }

    fn props(entries: Vec<(&str, Meta)>) -> HashMap<String, Meta> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect()
    }

    #[test]
    fn parses_spotify_metadata() {
        let track = track(&spotify_metadata()).unwrap();
        // The object path stays the id (seeking needs it); the URL says
        // which Spotify track it is.
        assert_eq!(track.id, "/com/spotify/track/0DZXVpUtPUom1VO6h5a0SU");
        assert_eq!(
            track.uri.as_deref(),
            Some("spotify:track:0DZXVpUtPUom1VO6h5a0SU")
        );
        assert_eq!(track.name, "Life");
        assert_eq!(track.artist, "Sigur Rós, Jónsi");
        assert_eq!(track.album, "Dreamcatcher");
        assert_eq!(track.duration, Duration::from_millis(303_440));
        assert!(track.artwork_url.starts_with("https://i.scdn.co/"));
    }

    #[test]
    fn only_spotify_tracks_get_a_spotify_uri() {
        let with = |entries: Vec<(&str, Meta)>| {
            let mut m = spotify_metadata();
            m.remove("xesam:url");
            m.extend(entries.into_iter().map(|(k, v)| (k.to_owned(), v)));
            track(&m).unwrap()
        };
        // Older Spotify: the URI as the trackid, no URL.
        let old = with(vec![(
            "mpris:trackid",
            text("spotify:track:7xGfFoTpQ2E7fRF5lN10tr"),
        )]);
        assert_eq!(
            old.uri.as_deref(),
            Some("spotify:track:7xGfFoTpQ2E7fRF5lN10tr")
        );
        // An ad, a local file, another player: no URI, nothing guessed.
        for (id, url) in [
            (
                "/com/spotify/ad/000000012c4a1bd4",
                "https://open.spotify.com/ad/x",
            ),
            (
                "/com/spotify/local/Someone/Album/Song/215",
                "spotify:local:Someone:Album:Song:215",
            ),
            (
                "/org/videolan/vlc/playlist/7",
                "file:///home/someone/Music/a.flac",
            ),
        ] {
            let t = with(vec![("mpris:trackid", text(id)), ("xesam:url", text(url))]);
            assert_eq!((t.id.as_str(), t.uri), (id, None), "{url}");
        }
    }

    #[test]
    fn odd_metadata_degrades() {
        // A string artist, a float length, no art, no id: still a track.
        let m = props(vec![
            ("xesam:title", text("Song")),
            ("xesam:artist", text("Someone")),
            ("mpris:length", Meta::Float(61e6)),
        ]);
        let t = track(&m).unwrap();
        assert_eq!((t.artist.as_str(), t.duration), ("Someone", S * 61));
        assert_eq!(t.id, "Song\u{1f}Someone\u{1f}");
        assert_eq!(t.artwork_url, "");
        // Nothing loaded.
        assert_eq!(track(&HashMap::new()), None);
        assert_eq!(track(&props(vec![("mpris:trackid", text(NO_TRACK))])), None);
        // Wrong types are ignored, not misread.
        let m = props(vec![
            ("mpris:trackid", text("/x")),
            ("xesam:title", Meta::Int(5)),
            ("mpris:length", text("long")),
        ]);
        let t = track(&m).unwrap();
        assert_eq!((t.name.as_str(), t.duration), ("", Duration::ZERO));
    }

    #[test]
    fn player_properties_make_a_snapshot() {
        let now = Instant::now();
        let p = props(vec![
            ("PlaybackStatus", text("Playing")),
            ("Metadata", Meta::Map(spotify_metadata())),
            ("Position", Meta::Int(12_500_000)),
            ("Volume", Meta::Float(0.684)),
            ("Shuffle", Meta::Bool(true)),
            ("LoopStatus", text("Track")),
        ]);
        let snap = snapshot(&p, now);
        assert_eq!(snap.status, Status::Playing);
        assert_eq!(snap.position, Duration::from_millis(12_500));
        assert_eq!(snap.sampled_at, now);
        assert_eq!(snap.volume, 68);
        assert!(snap.shuffle && snap.repeat);
        assert_eq!(snap.track.unwrap().name, "Life");
    }

    #[test]
    fn missing_player_properties_fall_back() {
        let now = Instant::now();
        let snap = snapshot(&HashMap::new(), now);
        assert_eq!(snap.status, Status::Stopped);
        assert_eq!((snap.position, snap.volume), (Duration::ZERO, 0));
        assert!(!snap.shuffle && !snap.repeat);
        let snap = snapshot(
            &props(vec![
                ("PlaybackStatus", text("Paused")),
                ("LoopStatus", text("None")),
                ("Volume", Meta::Float(7.5)),
                ("Position", Meta::Int(-3)),
            ]),
            now,
        );
        // Paused with nothing loaded: stopped.
        assert_eq!(snap.status, Status::Stopped);
        assert_eq!((snap.volume, snap.position), (100, Duration::ZERO));
        assert!(!snap.repeat);
    }

    #[test]
    fn spotify_is_preferred_then_the_first_player() {
        let names = [
            "org.freedesktop.DBus",
            ":1.42",
            "org.mpris.MediaPlayer2.vlc",
            "org.mpris.MediaPlayer2.spotify",
            "org.mpris.MediaPlayer2.",
        ];
        assert_eq!(choose(names), Some("org.mpris.MediaPlayer2.spotify"));
        assert_eq!(
            choose(names[..3].iter().copied()),
            Some("org.mpris.MediaPlayer2.vlc")
        );
        assert_eq!(
            choose([
                "org.mpris.MediaPlayer2.spotify.instance7",
                "org.mpris.MediaPlayer2.a"
            ]),
            Some("org.mpris.MediaPlayer2.spotify.instance7")
        );
        assert_eq!(
            choose([
                "org.mpris.MediaPlayer2.spotifyd2",
                "org.mpris.MediaPlayer2.z"
            ]),
            Some("org.mpris.MediaPlayer2.spotifyd2")
        );
        assert_eq!(choose(["org.freedesktop.Notifications"]), None);
        assert_eq!(
            fallback_name("org.mpris.MediaPlayer2.vlc.instance123"),
            "Vlc"
        );
        assert_eq!(fallback_name("org.mpris.MediaPlayer2.firefox"), "Firefox");
    }

    #[test]
    fn commands_plan_into_calls() {
        let now = Instant::now();
        let mut snap = snapshot(
            &props(vec![
                ("PlaybackStatus", text("Paused")),
                ("Metadata", Meta::Map(spotify_metadata())),
                ("Position", Meta::Int(10_000_000)),
            ]),
            now,
        );
        assert_eq!(
            plan(&Command::PlayPause, None, now),
            Call::Method("PlayPause")
        );
        assert_eq!(plan(&Command::Next, None, now), Call::Method("Next"));
        assert_eq!(
            plan(&Command::Previous, None, now),
            Call::Method("Previous")
        );
        assert_eq!(
            plan(&Command::Seek(S * 61), Some(&snap), now),
            Call::SetPosition(
                "/com/spotify/track/0DZXVpUtPUom1VO6h5a0SU".into(),
                61_000_000
            )
        );
        // An id that isn't an object path: seek relative to where it is.
        let mut t = (**snap.track.as_ref().unwrap()).clone();
        t.id = "Song\u{1f}x\u{1f}".into();
        snap.track = Some(Arc::new(t));
        assert_eq!(
            plan(&Command::Seek(S * 4), Some(&snap), now),
            Call::Seek(-6_000_000)
        );
        assert_eq!(
            plan(&Command::SetShuffle(true), None, now),
            Call::SetBool("Shuffle", true)
        );
        assert_eq!(
            plan(&Command::SetRepeat(true), None, now),
            Call::SetText("LoopStatus", "Playlist")
        );
        assert_eq!(
            plan(&Command::SetRepeat(false), None, now),
            Call::SetText("LoopStatus", "None")
        );
        assert_eq!(
            plan(&Command::SetVolume(42), None, now),
            Call::SetFloat("Volume", 0.42)
        );
        assert_eq!(
            plan(&Command::PlayUri("spotify:album:x".into()), None, now),
            Call::OpenUri("spotify:album:x".into())
        );
        let in_context = Command::play_in_context("spotify:track:t", "spotify:album:x").unwrap();
        assert_eq!(
            plan(&in_context, None, now),
            Call::OpenUri("spotify:track:t".into())
        );
    }

    #[test]
    fn a_zero_position_while_the_same_track_plays_is_unknown() {
        let t0 = Instant::now();
        let playing = |position: i64, at: Instant| {
            snapshot(
                &props(vec![
                    ("PlaybackStatus", text("Playing")),
                    ("Metadata", Meta::Map(spotify_metadata())),
                    ("Position", Meta::Int(position)),
                ]),
                at,
            )
        };
        let last = playing(5_000_000, t0);
        let mut fresh = playing(0, t0 + S);
        keep_position(&mut fresh, Some(&last));
        assert_eq!(fresh.position, S * 6);
        // A real position, the first read, or a pause: taken as is.
        let mut fresh = playing(1_000_000, t0 + S);
        keep_position(&mut fresh, Some(&last));
        assert_eq!(fresh.position, S);
        let mut fresh = playing(0, t0 + S);
        keep_position(&mut fresh, None);
        assert_eq!(fresh.position, Duration::ZERO);
        // Paused, and the resume after it (our command applied to `last`).
        let mut paused = last.clone();
        paused.apply(&Command::PlayPause, t0 + S);
        let mut fresh = playing(0, t0 + S * 3);
        fresh.status = Status::Paused;
        keep_position(&mut fresh, Some(&paused));
        assert_eq!(fresh.position, S * 6);
        paused.apply(&Command::PlayPause, t0 + S * 3);
        let mut fresh = playing(0, t0 + S * 4);
        keep_position(&mut fresh, Some(&paused));
        assert_eq!(fresh.position, S * 7);
        // A seek sent from here.
        let mut sought = last.clone();
        sought.apply(&Command::Seek(S * 100), t0);
        let mut fresh = playing(0, t0 + S);
        keep_position(&mut fresh, Some(&sought));
        assert_eq!(fresh.position, S * 101);
        // Another track starts at 0; so does a stopped player.
        let mut other = playing(0, t0 + S);
        let mut t = (**other.track.as_ref().unwrap()).clone();
        t.id = "/com/spotify/track/other".into();
        other.track = Some(Arc::new(t));
        keep_position(&mut other, Some(&last));
        assert_eq!(other.position, Duration::ZERO);
        let mut stopped = playing(0, t0 + S);
        stopped.status = Status::Stopped;
        keep_position(&mut stopped, Some(&last));
        assert_eq!(stopped.position, Duration::ZERO);
    }

    #[test]
    fn object_paths() {
        assert!(is_object_path("/"));
        assert!(is_object_path("/com/spotify/track/0DZX_1"));
        for bad in ["", "x", "//", "/a/", "/a-b", "/é", "spotify:track:x"] {
            assert!(!is_object_path(bad), "{bad}");
        }
    }
}
