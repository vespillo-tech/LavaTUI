//! macOS: every player LavaTUI can ask (the Spotify app, Apple Music),
//! and which one to follow.
//!
//! [`Players`] is one [`Backend`] over several: each exchange asks every
//! player (one that isn't running costs a guard check in the shared
//! `osascript`, never a launch) and follows one by the shared rule
//! ([`choice`](super::choice): Spotify while it plays, else whatever
//! plays, else the one in use). Commands go to the player followed;
//! "play this Spotify URI" (the library) goes to Spotify wherever the keys
//! are, pausing the other player first, so Spotify then plays and is
//! followed.
//!
//! The choosing is pure (tests run it on fakes everywhere); only
//! [`Players::detect`] and the change listener are macOS's.

use super::worker::{Backend, Nudge};
use super::{Capabilities, Command, Snapshot, Status, Unavailable};

/// One player and whether it's Spotify.
pub struct Entry {
    backend: Box<dyn Backend>,
    spotify: bool,
}

impl Entry {
    pub fn spotify(backend: impl Backend) -> Self {
        Self {
            backend: Box::new(backend),
            spotify: true,
        }
    }

    pub fn other(backend: impl Backend) -> Self {
        Self {
            backend: Box::new(backend),
            spotify: false,
        }
    }
}

/// Several players, one followed. See the module docs.
pub struct Players {
    players: Vec<Entry>,
    /// The one followed last.
    in_use: Option<usize>,
    /// Each one's status as last read.
    last: Vec<Status>,
    capabilities: Capabilities,
    /// The distributed notifications they post on change.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    notifications: Vec<&'static str>,
}

impl Players {
    pub fn new(players: Vec<Entry>, notifications: &[&'static str]) -> Self {
        Self {
            last: vec![Status::Connecting; players.len()],
            players,
            in_use: None,
            capabilities: Capabilities::NONE,
            notifications: notifications.to_vec(),
        }
    }

    /// Spotify and Apple Music, sharing one `osascript`.
    #[cfg(target_os = "macos")]
    pub fn detect() -> Self {
        use std::sync::{Arc, Mutex};

        use super::applescript::{self, Routed};
        use super::runner::Osascript;
        use super::{apple_music, spotify};

        let shared = Arc::new(Mutex::new(Osascript::new(applescript::script(&[
            (spotify::BUNDLE_ID, spotify::spotify_part()),
            (apple_music::BUNDLE_ID, apple_music::music_part()),
        ]))));
        let spotify = spotify::Spotify::new(Routed::new(spotify::BUNDLE_ID, Arc::clone(&shared)));
        let music = apple_music::AppleMusic::new(Routed::new(apple_music::BUNDLE_ID, shared));
        Self::new(
            vec![Entry::spotify(spotify), Entry::other(music)],
            &[spotify::NOTIFICATION, apple_music::NOTIFICATION],
        )
    }

    /// The commands for each player: the followed one's, and URIs for
    /// Spotify (with a pause for the followed one if it's another player
    /// and playing).
    fn batches(&self, commands: &[Command]) -> Vec<Vec<Command>> {
        let mut batches = vec![Vec::new(); self.players.len()];
        let spotify = self.players.iter().position(|p| p.spotify);
        for command in commands {
            let uri = matches!(command, Command::PlayUri(_) | Command::PlayInContext { .. });
            match (uri, spotify, self.in_use) {
                (true, Some(s), Some(t)) if s != t => {
                    if self.last[t] == Status::Playing && !batches[t].contains(&Command::PlayPause)
                    {
                        batches[t].push(Command::PlayPause);
                    }
                    batches[s].push(command.clone());
                }
                (true, Some(s), None) => batches[s].push(command.clone()),
                (_, _, Some(t)) => batches[t].push(command.clone()),
                // Nothing followed: the UI sends nothing then.
                (_, _, None) => {}
            }
        }
        batches
    }

    /// Which of `snaps` to follow, by the shared rule; `None` when none
    /// answers.
    fn choose(&self, snaps: &[Snapshot]) -> Option<usize> {
        let available: Vec<usize> = (0..snaps.len())
            .filter(|&i| snaps[i].status.is_available())
            .collect();
        // The one in use missing an answer (busy, an error) keeps the
        // widget, unless another is playing: the worker shows its last
        // state for a moment, then the problem.
        if let Some(t) = self.in_use
            && transient(&snaps[t].status)
            && !available
                .iter()
                .any(|&i| snaps[i].status == Status::Playing)
        {
            return Some(t);
        }
        let k = super::choice::pick(available.iter().map(|&i| super::choice::Player {
            spotify: self.players[i].spotify,
            playing: snaps[i].status == Status::Playing,
            current: false,
            in_use: self.in_use == Some(i),
        }))?;
        Some(available[k])
    }
}

/// A failure that may pass by itself.
fn transient(status: &Status) -> bool {
    matches!(
        status,
        Status::Unavailable(Unavailable::NotResponding | Unavailable::Error(_))
    )
}

/// With no player to follow: the problem the user can do something about
/// first (allow control, a player not answering), else "open your music
/// player" (no name: any of them will do).
fn nobody(snaps: Vec<Snapshot>) -> Snapshot {
    let rank = |s: &Snapshot| match &s.status {
        Status::Unavailable(Unavailable::PermissionDenied) => 0,
        Status::Unavailable(Unavailable::NotResponding | Unavailable::Error(_)) => 1,
        Status::Unavailable(Unavailable::Unsupported) => 2,
        _ => 3,
    };
    match snaps.into_iter().min_by_key(rank) {
        Some(snap) if rank(&snap) < 3 => snap,
        Some(snap) => Snapshot::new(
            Status::Unavailable(Unavailable::NotRunning),
            snap.sampled_at,
        ),
        None => Snapshot::new(
            Status::Unavailable(Unavailable::NotRunning),
            std::time::Instant::now(),
        ),
    }
}

impl Backend for Players {
    fn exchange(&mut self, commands: &[Command]) -> Snapshot {
        let batches = self.batches(commands);
        // The followed one last: its reading is the freshest when the
        // answer goes up (the worker's `Baseline` reads it that way).
        let order: Vec<usize> = (0..self.players.len())
            .filter(|&i| Some(i) != self.in_use)
            .chain(self.in_use)
            .collect();
        let mut snaps: Vec<Option<Snapshot>> = vec![None; self.players.len()];
        for i in order {
            snaps[i] = Some(self.players[i].backend.exchange(&batches[i]));
        }
        let snaps: Vec<Snapshot> = snaps.into_iter().flatten().collect();
        self.last = snaps.iter().map(|s| s.status.clone()).collect();
        let Some(i) = self.choose(&snaps) else {
            self.capabilities = Capabilities::NONE;
            return nobody(snaps);
        };
        if self.in_use != Some(i) {
            crate::diag::note(|| format!("player: following {}", snaps[i].player_name()));
        }
        self.in_use = Some(i);
        let mut capabilities = self.players[i].backend.capabilities();
        // Spotify URIs go to Spotify while it's open, whoever is followed.
        if let Some(s) = self.players.iter().position(|p| p.spotify)
            && snaps[s].status.is_available()
        {
            let spotify = self.players[s].backend.capabilities();
            capabilities.uris |= spotify.uris;
            capabilities.contexts |= spotify.contexts;
        }
        self.capabilities = capabilities;
        snaps.into_iter().nth(i).expect("chosen from these")
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    #[cfg(target_os = "macos")]
    fn listen(&mut self, nudge: Nudge) -> Option<Box<dyn Send>> {
        let watcher = super::notify::watch(&self.notifications, nudge)?;
        Some(Box::new(watcher))
    }

    #[cfg(not(target_os = "macos"))]
    fn listen(&mut self, _nudge: Nudge) -> Option<Box<dyn Send>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::media::Track;

    /// A pretend player: answers with its status (and a track while
    /// available), records what it was sent, applies play/pause and URIs.
    #[derive(Clone)]
    struct Fake(Arc<Mutex<FakeState>>);

    struct FakeState {
        name: &'static str,
        status: Status,
        sent: Vec<Vec<Command>>,
        caps: Capabilities,
    }

    impl Fake {
        fn new(name: &'static str, status: Status) -> Self {
            Self(Arc::new(Mutex::new(FakeState {
                name,
                status,
                sent: Vec::new(),
                caps: Capabilities {
                    uris: name == "Spotify",
                    contexts: name == "Spotify",
                    ..Capabilities::ALL
                },
            })))
        }
        fn set(&self, status: Status) {
            self.0.lock().unwrap().status = status;
        }
        fn sent(&self) -> Vec<Vec<Command>> {
            self.0.lock().unwrap().sent.clone()
        }
    }

    impl Backend for Fake {
        fn exchange(&mut self, commands: &[Command]) -> Snapshot {
            let mut fake = self.0.lock().unwrap();
            fake.sent.push(commands.to_vec());
            for command in commands {
                fake.status = match (command, &fake.status) {
                    (Command::PlayPause, Status::Playing) => Status::Paused,
                    (Command::PlayPause, Status::Paused) => Status::Playing,
                    (Command::PlayUri(_) | Command::PlayInContext { .. }, s)
                        if s.is_available() =>
                    {
                        Status::Playing
                    }
                    (_, s) => s.clone(),
                };
            }
            let mut snap = Snapshot::new(fake.status.clone(), Instant::now());
            snap.player = Some(fake.name.into());
            if snap.status.is_available() {
                snap.track = Some(Arc::new(Track {
                    id: format!("{}-song", fake.name),
                    duration: Duration::from_secs(200),
                    ..Track::default()
                }));
            }
            snap
        }

        fn capabilities(&self) -> Capabilities {
            self.0.lock().unwrap().caps
        }
    }

    const NOT_RUNNING: Status = Status::Unavailable(Unavailable::NotRunning);

    fn players(spotify: Status, music: Status) -> (Players, Fake, Fake) {
        let (s, m) = (Fake::new("Spotify", spotify), Fake::new("Music", music));
        let players = Players::new(
            vec![Entry::spotify(s.clone()), Entry::other(m.clone())],
            &[],
        );
        (players, s, m)
    }

    fn name(snap: &Snapshot) -> &str {
        snap.player.as_deref().unwrap_or("-")
    }

    #[test]
    fn music_is_followed_when_spotify_is_closed_or_idle() {
        let (mut p, s, _) = players(NOT_RUNNING, Status::Playing);
        assert_eq!(name(&p.exchange(&[])), "Music");
        s.set(Status::Paused);
        assert_eq!(
            name(&p.exchange(&[])),
            "Music",
            "an idle Spotify never wins"
        );
    }

    #[test]
    fn spotify_comes_first_while_it_plays_and_the_one_in_use_keeps_it() {
        let (mut p, s, m) = players(Status::Paused, Status::Playing);
        assert_eq!(name(&p.exchange(&[])), "Music");
        s.set(Status::Playing);
        assert_eq!(name(&p.exchange(&[])), "Spotify");
        // Spotify paused: the keys stay on it (play resumes it), even with
        // Music paused too.
        s.set(Status::Paused);
        m.set(Status::Paused);
        assert_eq!(name(&p.exchange(&[])), "Spotify");
        // Music starting takes over.
        m.set(Status::Playing);
        assert_eq!(name(&p.exchange(&[])), "Music");
    }

    #[test]
    fn commands_go_to_the_player_followed_only() {
        let (mut p, s, m) = players(Status::Paused, Status::Playing);
        p.exchange(&[]);
        let snap = p.exchange(&[Command::PlayPause, Command::SetVolume(20)]);
        assert_eq!(name(&snap), "Music");
        assert_eq!(
            m.sent().last().unwrap(),
            &[Command::PlayPause, Command::SetVolume(20)]
        );
        assert!(s.sent().iter().all(Vec::is_empty), "Spotify only polled");
        // The followed player is asked last.
        assert!(p.in_use == Some(1));
    }

    #[test]
    fn a_spotify_uri_goes_to_spotify_and_pauses_music() {
        let (mut p, s, m) = players(Status::Paused, Status::Playing);
        p.exchange(&[]);
        assert!(p.capabilities().uris, "Spotify is open: it can play URIs");
        let uri = Command::play_uri("spotify:playlist:0123456789abcdef").unwrap();
        let snap = p.exchange(std::slice::from_ref(&uri));
        assert_eq!(m.sent().last().unwrap(), &[Command::PlayPause]);
        assert_eq!(s.sent().last().unwrap(), &[uri]);
        assert_eq!(name(&snap), "Spotify", "Spotify plays it and is followed");
        assert!(p.capabilities().uris);
    }

    #[test]
    fn without_spotify_open_music_cant_be_told_what_to_play() {
        let (mut p, _, _) = players(NOT_RUNNING, Status::Paused);
        assert_eq!(name(&p.exchange(&[])), "Music");
        assert!(!p.capabilities().uris);
        assert!(p.capabilities().volume);
    }

    #[test]
    fn with_nothing_open_any_player_will_do() {
        let (mut p, _, _) = players(NOT_RUNNING, NOT_RUNNING);
        let snap = p.exchange(&[]);
        assert_eq!(snap.status, NOT_RUNNING);
        assert_eq!(snap.player, None);
        assert_eq!(
            snap.unavailable_message().as_deref(),
            Some("Open your music player to show music")
        );
        assert_eq!(p.capabilities(), Capabilities::NONE);
        // Spotify not installed is no different.
        let (mut p, _, _) = players(Status::Unavailable(Unavailable::NotInstalled), NOT_RUNNING);
        assert_eq!(p.exchange(&[]).status, NOT_RUNNING);
    }

    #[test]
    fn a_problem_the_user_can_fix_is_said() {
        let denied = Status::Unavailable(Unavailable::PermissionDenied);
        let (mut p, _, _) = players(NOT_RUNNING, denied.clone());
        let snap = p.exchange(&[]);
        assert_eq!(snap.status, denied);
        assert_eq!(name(&snap), "Music");
    }

    #[test]
    fn a_hiccup_in_the_player_in_use_doesnt_hand_the_widget_over() {
        let busy = Status::Unavailable(Unavailable::NotResponding);
        let (mut p, s, m) = players(Status::Paused, Status::Paused);
        assert_eq!(name(&p.exchange(&[])), "Spotify");
        s.set(busy.clone());
        let snap = p.exchange(&[]);
        assert_eq!((name(&snap), &snap.status), ("Spotify", &busy));
        // Unless another one is playing.
        m.set(Status::Playing);
        assert_eq!(name(&p.exchange(&[])), "Music");
    }

    /// Against the real apps, read-only (no commands): what each player
    /// says and which one is followed, and that asking never opened Music.
    /// `cargo test --release -- --ignored --nocapture live_players`
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "reads the real Spotify and Music apps"]
    fn live_players() {
        let running = |name: &str| {
            std::process::Command::new("/usr/bin/pgrep")
                .args(["-x", name])
                .stdout(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        let music_before = running("Music");
        let mut p = Players::detect();
        // LAVATUI_POLLS=200 for a CPU figure (the shared osascript's).
        let polls: u32 = std::env::var("LAVATUI_POLLS")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(3);
        let cpu = || {
            let out = std::process::Command::new("/bin/ps")
                .args(["-Aww", "-o", "cputime=,command="])
                .output()
                .ok()?;
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let line = text
                .lines()
                .find(|l| l.contains("property bids"))?
                .trim()
                .to_owned();
            let time = line.split_whitespace().next()?.to_owned();
            let (m, s) = time.split_once(':')?;
            Some(m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?)
        };
        p.exchange(&[]);
        let before = cpu();
        for round in 3..polls {
            p.exchange(&[]);
            if round + 1 == polls
                && let (Some(a), Some(b)) = (before, cpu())
            {
                let n = f64::from(polls - 3);
                println!(
                    "osascript CPU: {:.2} ms a poll ({n} polls)",
                    (b - a) * 1000.0 / n
                );
            }
        }
        for round in 0..3 {
            let start = Instant::now();
            let snap = p.exchange(&[]);
            let track = snap.track.as_ref();
            println!(
                "round {round}: {:>4.0} ms  each {:?}  following {} {:?}  track {}  cover {}",
                start.elapsed().as_secs_f64() * 1000.0,
                p.last,
                snap.player_name(),
                snap.status,
                track.map_or("none", |t| if t.name.is_empty() {
                    "unnamed"
                } else {
                    "named"
                }),
                track.map_or("-", |t| t.artwork_url.split(':').next().unwrap_or("")),
            );
        }
        assert_eq!(running("Music"), music_before, "asking never opens Music");
    }
}
