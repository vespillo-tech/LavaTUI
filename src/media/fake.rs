//! [`FakeSource`]: an in-memory player. Commands take effect at once
//! (no thread, no latency); playback advances with real time and moves on
//! to the next track at the end of one. For tests, and for building the
//! UI without a real player.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::{Capabilities, Command, MediaSource, Snapshot, Status, Track};

/// Previous within this far into a track goes to the previous track;
/// later, it restarts the current one (as Spotify does).
const RESTART_AFTER: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct FakeSource {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    snapshot: Snapshot,
    playlist: Vec<Arc<Track>>,
    index: usize,
    /// Tests only: `--demo` runs for hours.
    #[cfg(test)]
    sent: Vec<Command>,
    capabilities: Capabilities,
    /// Playlists it can play, by URI (`PlayUri` / `PlayInContext`).
    contexts: HashMap<String, Vec<Arc<Track>>>,
}

impl FakeSource {
    /// A player in `snapshot`'s state; `playlist` is what next/previous
    /// walk through (the snapshot's own track if empty).
    pub fn new(snapshot: Snapshot, playlist: Vec<Track>) -> Self {
        let mut playlist: Vec<Arc<Track>> = playlist.into_iter().map(Arc::new).collect();
        if playlist.is_empty() {
            playlist.extend(snapshot.track.clone());
        }
        let index = snapshot
            .track
            .as_ref()
            .and_then(|t| playlist.iter().position(|p| p.id == t.id))
            .unwrap_or(0);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                snapshot,
                playlist,
                index,
                #[cfg(test)]
                sent: Vec::new(),
                capabilities: Capabilities::ALL,
                contexts: HashMap::new(),
            })),
        }
    }

    /// Knows these playlists (URI, tracks): playing one, or a track in
    /// one, walks through it from then on.
    pub fn with_contexts(self, contexts: Vec<(String, Vec<Track>)>) -> Self {
        self.lock().contexts = contexts
            .into_iter()
            .map(|(uri, tracks)| (uri, tracks.into_iter().map(Arc::new).collect()))
            .collect();
        self
    }

    /// Something to look at: three tracks, the first playing.
    #[cfg(test)]
    pub fn demo(now: Instant) -> Self {
        let track = |n: u32, name: &str, artist: &str, album: &str, secs: u64| Track {
            id: format!("fake:track:{n}"),
            uri: None,
            name: name.into(),
            artist: artist.into(),
            album: album.into(),
            duration: Duration::from_secs(secs),
            artwork_url: String::new(),
        };
        let playlist = vec![
            track(1, "Slow Rise", "The Paraffins", "Heat Rises", 214),
            track(2, "Blob Merge", "The Paraffins", "Heat Rises", 187),
            track(
                3,
                "Convection (Long Version)",
                "Wax & Wane",
                "Lamplight",
                402,
            ),
        ];
        let snapshot = Snapshot {
            track: Some(Arc::new(playlist[0].clone())),
            position: Duration::from_secs(42),
            volume: 70,
            ..Snapshot::new(Status::Playing, now)
        };
        Self::new(snapshot, playlist)
    }

    /// Replace the whole state (e.g. to show an unavailable reason).
    #[cfg(test)]
    pub fn set(&self, snapshot: Snapshot) {
        self.lock().snapshot = snapshot;
    }

    /// Act like a player without some controls (e.g. Spotify's
    /// AppleScript, which can't shuffle).
    #[cfg(test)]
    pub fn set_capabilities(&self, capabilities: Capabilities) {
        self.lock().capabilities = capabilities;
    }

    /// Every command sent so far, in order.
    #[cfg(test)]
    pub fn sent(&self) -> Vec<Command> {
        self.lock().sent.clone()
    }

    /// The state at `now` (moves on to the next track at the end of one).
    pub fn snapshot_at(&self, now: Instant) -> Snapshot {
        let mut inner = self.lock();
        inner.roll_over(now);
        inner.snapshot.clone()
    }

    /// Apply `command` as of `now`.
    pub fn send_at(&self, command: Command, now: Instant) {
        let mut inner = self.lock();
        #[cfg(test)]
        inner.sent.push(command.clone());
        if !inner.snapshot.status.is_available() {
            return;
        }
        inner.roll_over(now);
        match &command {
            Command::Next => inner.skip(1, now),
            Command::Previous if inner.snapshot.position_at(now) < RESTART_AFTER => {
                inner.skip(-1, now);
            }
            Command::PlayUri(uri) if inner.contexts.contains_key(uri) => {
                inner.playlist = inner.contexts[uri].clone();
                inner.load(0, now);
                inner.snapshot.status = Status::Playing;
            }
            Command::PlayUri(uri) | Command::PlayInContext { track: uri, .. } => {
                if let Command::PlayInContext { context, .. } = &command
                    && let Some(tracks) = inner.contexts.get(context)
                {
                    inner.playlist = tracks.clone();
                }
                let found = inner.playlist.iter().position(|t| &t.id == uri);
                let index = found.unwrap_or_else(|| {
                    let known = inner.contexts.values().flatten().find(|t| &t.id == uri);
                    let track = known.cloned().unwrap_or_else(|| {
                        Arc::new(Track {
                            id: uri.clone(),
                            uri: super::spotify_track_uri(uri),
                            name: uri.clone(),
                            ..Track::default()
                        })
                    });
                    inner.playlist.push(track);
                    inner.playlist.len() - 1
                });
                inner.load(index, now);
                inner.snapshot.status = Status::Playing;
            }
            _ => inner.snapshot.apply(&command, now),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Inner {
    fn skip(&mut self, by: isize, now: Instant) {
        let len = self.playlist.len().max(1) as isize;
        let index = (self.index as isize + by).rem_euclid(len) as usize;
        self.load(index, now);
    }

    fn load(&mut self, index: usize, now: Instant) {
        self.index = index;
        self.snapshot.track = self.playlist.get(index).cloned();
        self.snapshot.position = Duration::ZERO;
        self.snapshot.sampled_at = now;
    }

    /// Past the end of the track: on to the next, carrying the overflow.
    /// Before that the snapshot stays as it was stamped (its reader
    /// extrapolates): rebasing it to the real clock at every read would
    /// put it ahead of a reader on a fake clock (tests) by however long
    /// they took.
    fn roll_over(&mut self, now: Instant) {
        let Some(duration) = self.snapshot.track.as_ref().map(|t| t.duration) else {
            return;
        };
        if duration.is_zero() || self.snapshot.status != Status::Playing {
            return;
        }
        let mut position =
            self.snapshot.position + now.saturating_duration_since(self.snapshot.sampled_at);
        if position < duration {
            return;
        }
        while position >= duration {
            position -= duration;
            self.skip(1, now);
            let next = self
                .snapshot
                .track
                .as_ref()
                .map_or(duration, |t| t.duration);
            if next.is_zero() {
                position = Duration::ZERO;
                break;
            }
        }
        self.snapshot.position = position;
        self.snapshot.sampled_at = now;
    }
}

impl MediaSource for FakeSource {
    fn snapshot(&self) -> Snapshot {
        self.snapshot_at(Instant::now())
    }

    fn send(&self, command: Command) {
        self.send_at(command, Instant::now());
    }

    fn capabilities(&self) -> Capabilities {
        self.lock().capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::super::Unavailable;
    use super::*;

    const S: Duration = Duration::from_secs(1);

    fn name(snap: &Snapshot) -> &str {
        snap.track.as_ref().map_or("", |t| t.name.as_str())
    }

    #[test]
    fn controls_take_effect_at_once() {
        let t0 = Instant::now();
        let fake = FakeSource::demo(t0);
        assert_eq!(name(&fake.snapshot_at(t0)), "Slow Rise");
        fake.send_at(Command::Next, t0);
        assert_eq!(name(&fake.snapshot_at(t0)), "Blob Merge");
        fake.send_at(Command::Previous, t0 + S * 10); // 10 s in: restart
        let snap = fake.snapshot_at(t0 + S * 10);
        assert_eq!((name(&snap), snap.position), ("Blob Merge", Duration::ZERO));
        fake.send_at(Command::Previous, t0 + S * 11); // 1 s in: back
        assert_eq!(name(&fake.snapshot_at(t0 + S * 11)), "Slow Rise");
        fake.send_at(Command::Previous, t0 + S * 11); // wraps
        assert_eq!(
            name(&fake.snapshot_at(t0 + S * 11)),
            "Convection (Long Version)"
        );

        fake.send_at(Command::PlayPause, t0 + S * 12);
        assert_eq!(fake.snapshot_at(t0 + S * 12).status, Status::Paused);
        fake.send_at(Command::SetVolume(5), t0 + S * 12);
        assert_eq!(fake.snapshot_at(t0 + S * 12).volume, 5);
        fake.send_at(Command::PlayUri("spotify:track:new".into()), t0 + S * 12);
        let snap = fake.snapshot_at(t0 + S * 12);
        assert_eq!(
            (name(&snap), &snap.status),
            ("spotify:track:new", &Status::Playing)
        );
        assert_eq!(fake.sent().len(), 7);
    }

    #[test]
    fn a_read_keeps_the_reading_as_stamped() {
        // A reader on a fake clock (tests) sees playback where its clock
        // says, however long it really took between reads.
        let t0 = Instant::now() - Duration::from_secs(5);
        let fake = FakeSource::new(
            Snapshot {
                track: Some(Arc::new(Track {
                    duration: Duration::from_secs(200),
                    ..Track::default()
                })),
                position: Duration::from_secs(6),
                ..Snapshot::new(Status::Playing, t0)
            },
            Vec::new(),
        );
        let snap = fake.snapshot();
        assert_eq!(snap.sampled_at, t0);
        assert_eq!(snap.position_at(t0), Duration::from_secs(6));
        // Real time still moves it on.
        assert!(snap.position_at(Instant::now()) >= Duration::from_secs(11));
    }

    #[test]
    fn playback_rolls_over_to_the_next_track() {
        let t0 = Instant::now();
        let fake = FakeSource::demo(t0); // 42 s into a 214 s track
        let snap = fake.snapshot_at(t0 + S * (214 - 42 + 5));
        assert_eq!(name(&snap), "Blob Merge");
        assert_eq!(snap.position, S * 5);
    }

    #[test]
    fn unavailable_ignores_commands_but_records_them() {
        let t0 = Instant::now();
        let fake = FakeSource::demo(t0);
        fake.set(Snapshot::new(
            Status::Unavailable(Unavailable::PermissionDenied),
            t0,
        ));
        fake.send_at(Command::Next, t0);
        assert_eq!(fake.snapshot_at(t0).track, None);
        assert_eq!(fake.sent(), vec![Command::Next]);
    }

    #[test]
    fn is_a_media_source() {
        let source: Box<dyn MediaSource> = Box::new(FakeSource::demo(Instant::now()));
        source.set_shuffle(true);
        source.play_uri("not a uri");
        assert!(source.snapshot().shuffle);
    }
}
