//! [`Polled`]: a [`MediaSource`] backed by a background thread.
//!
//! The worker owns a blocking [`Backend`]. It waits for a command or the
//! next poll, whichever comes first; on a command it drains the queue and
//! runs the batch plus a state read in one exchange, then reads again
//! shortly after. Polls are paced by what the player is doing
//! ([`Cadence`]).
//!
//! Players apply commands asynchronously (Spotify still reports "playing"
//! for ~200 ms after a pause), so the read in the command's own exchange
//! is only used if it says the player is gone; otherwise the optimistic
//! state stands until the follow-up read.
//!
//! The UI thread only ever takes a short lock: [`Polled::send`] applies the
//! command optimistically and counts it as sent; the worker publishes an
//! answer only when it has handled every command sent so far, so a poll
//! that was already in flight can't undo a key press the user just saw.
//!
//! The position is pinned down over polls ([`Baseline`]): each reading
//! was taken between sending the request and getting the reply, so
//! readings of the same playback narrow down when it really was, to a few
//! ms however slow any one poll was. While synced lyrics are on screen
//! ([`MediaSource::follow_closely`]) the player is polled more often
//! ([`Cadence::close`]), so a pause, resume or seek made in the player
//! itself shows sooner.

use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::{Capabilities, Command, MediaSource, Snapshot, Status, Unavailable};

/// A player that can be asked, blocking, for its state.
pub trait Backend: Send + 'static {
    /// Run `commands` in order (none for a plain poll), then read the
    /// state, stamped with the instant it was read. Failures come back as
    /// an `Unavailable` status, never a panic.
    fn exchange(&mut self, commands: &[Command]) -> Snapshot;

    /// Which optional controls work. Read after every exchange: a backend
    /// may learn that its player ignores one (Spotify over MPRIS ignores
    /// shuffle / repeat).
    fn capabilities(&self) -> Capabilities {
        Capabilities::ALL
    }
}

/// How long to wait before the next poll, by what the player is doing.
#[derive(Clone, Copy, Debug)]
pub struct Cadence {
    pub playing: Duration,
    /// Paused or stopped.
    pub idle: Duration,
    pub not_running: Duration,
    pub not_responding: Duration,
    /// Not installed, permission denied, other errors: only the user can
    /// fix these, so just check back now and then.
    pub broken: Duration,
    /// The re-read after a command: long enough for the player to have
    /// applied it.
    pub after_command: Duration,
    /// Playing or paused while something follows playback closely (synced
    /// lyrics on screen).
    pub close: Duration,
}

impl Default for Cadence {
    fn default() -> Self {
        Self {
            playing: Duration::from_secs(1),
            idle: Duration::from_secs(2),
            not_running: Duration::from_secs(4),
            not_responding: Duration::from_secs(5),
            broken: Duration::from_secs(15),
            after_command: Duration::from_millis(400),
            close: Duration::from_millis(250),
        }
    }
}

impl Cadence {
    fn after(&self, status: &Status, close: bool) -> Duration {
        match status {
            Status::Playing | Status::Paused if close => self.close,
            Status::Playing | Status::Connecting => self.playing,
            Status::Paused | Status::Stopped => self.idle,
            Status::Unavailable(Unavailable::NotRunning) => self.not_running,
            Status::Unavailable(Unavailable::NotResponding) => self.not_responding,
            Status::Unavailable(_) => self.broken,
        }
    }
}

/// A reading that misses an agreed [`Baseline`] window by less than this
/// is noise (a player that reports coarse positions): the window stands,
/// up to [`MISSES`] times in a row. Further off, it's different playback
/// (a pause, a seek, a stall) and the window starts again. A window only
/// one reading has set is never held: after a seek Spotify's position
/// stands still for 250-400 ms (buffering), and that is followed at once.
const JITTER: Duration = Duration::from_millis(250);
/// Readings in a row that may miss the window before it starts again.
const MISSES: u8 = 3;
/// Where in a wide window playback is put: this far above its low end (a
/// reading taken just before the reply; backends read the position last),
/// at most half way.
const READ: Duration = Duration::from_millis(10);

/// Where playback is, pinned down over polls. A position read somewhere
/// between sending the request and getting the reply bounds the
/// playback's baseline (position − time) from both sides: it's between
/// `position − reply` and `position − sent`. Readings of the same
/// playback intersect, so the window narrows to the quickest round trip
/// (Spotify's reported position is exact: 2,000 readings fit one line to
/// ±4 ms), and a slow poll (seconds, at a track change) costs nothing
/// once a quick one follows.
#[derive(Debug)]
struct Baseline {
    epoch: Instant,
    /// The window, in ns relative to `epoch`.
    window: Option<(i128, i128)>,
    /// Two readings or more agree on it.
    agreed: bool,
    misses: u8,
}

impl Baseline {
    fn new(epoch: Instant) -> Self {
        Self {
            epoch,
            window: None,
            agreed: false,
            misses: 0,
        }
    }

    fn ns(&self, t: Instant) -> i128 {
        match t.checked_duration_since(self.epoch) {
            Some(d) => d.as_nanos() as i128,
            None => -(self.epoch.duration_since(t).as_nanos() as i128),
        }
    }

    /// Put `fresh` (asked for at `sent`) where the window says, after
    /// `previous` (what was published). Also keeps `previous`'s
    /// `Arc<Track>` when the track is the same, so the UI can tell "new
    /// track" by pointer.
    fn settle(&mut self, fresh: &mut Snapshot, previous: &Snapshot, sent: Instant) {
        if fresh.track.is_some() && fresh.track == previous.track {
            fresh.track.clone_from(&previous.track);
        }
        if fresh.status != Status::Playing {
            self.window = None;
            return;
        }
        let pos = fresh.position.as_nanos() as i128;
        let reading = (
            pos - self.ns(fresh.sampled_at),
            pos - self.ns(sent.min(fresh.sampled_at)),
        );
        let window = match self.window.filter(|_| fresh.continues(previous)) {
            None => {
                self.agreed = false;
                reading
            }
            Some((lo, hi)) => {
                let (l, h) = (lo.max(reading.0), hi.min(reading.1));
                if l <= h {
                    self.misses = 0;
                    self.agreed = true;
                    (l, h)
                } else if self.agreed
                    && l - h <= JITTER.as_nanos() as i128
                    && self.misses + 1 < MISSES
                {
                    self.misses += 1;
                    (lo, hi)
                } else {
                    self.misses = 0;
                    self.agreed = false;
                    reading
                }
            }
        };
        self.window = Some(window);
        let (lo, hi) = window;
        let baseline = lo + ((hi - lo) / 2).min(READ.as_nanos() as i128);
        let at = baseline + self.ns(fresh.sampled_at);
        fresh.position = Duration::from_nanos(u64::try_from(at).unwrap_or(0));
    }
}

/// A player that was answering and then misses (busy as it changes track,
/// a loaded machine: Spotify can take a second over a track change) keeps
/// its last state on screen for this many failed polls in a row, each
/// tried again at the playing pace, before the failure shows. One missed
/// answer used to blank the music, lyrics and cover for 5-15 s
/// (lava-75z.22).
const GRACE: u32 = 2;

#[derive(Debug)]
struct State {
    snapshot: Snapshot,
    /// Commands sent to the worker so far.
    sent: u64,
    /// The backend's, as of its last exchange.
    capabilities: Capabilities,
}

/// The handle the UI keeps. Dropping it stops the worker (after any
/// exchange in flight; nothing waits for it).
pub struct Polled {
    state: Arc<Mutex<State>>,
    commands: Option<Sender<Command>>,
    /// [`MediaSource::follow_closely`], read by the worker.
    close: Arc<AtomicBool>,
}

impl Polled {
    /// Start a worker thread for `backend`.
    pub fn spawn<B: Backend>(backend: B, cadence: Cadence) -> Self {
        let state = Arc::new(Mutex::new(State {
            snapshot: Snapshot::new(Status::Connecting, Instant::now()),
            sent: 0,
            capabilities: backend.capabilities(),
        }));
        let (tx, rx) = mpsc::channel();
        let close = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            backend,
            state: Arc::clone(&state),
            commands: rx,
            handled: 0,
            cadence,
            misses: 0,
            close: Arc::clone(&close),
            baseline: Baseline::new(Instant::now()),
        };
        let spawned = thread::Builder::new()
            .name("lavatui-media".into())
            .spawn(move || {
                crate::thread_qos::worker();
                worker.run()
            });
        match spawned {
            Ok(_) => Self {
                state,
                commands: Some(tx),
                close,
            },
            Err(err) => Self::unavailable(Unavailable::Error(err.to_string())),
        }
    }

    /// A source that never answers, for a platform without a backend.
    pub fn unavailable(reason: Unavailable) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                snapshot: Snapshot::new(Status::Unavailable(reason), Instant::now()),
                sent: 0,
                capabilities: Capabilities::NONE,
            })),
            commands: None,
            close: Arc::default(),
        }
    }
}

impl MediaSource for Polled {
    fn snapshot(&self) -> Snapshot {
        lock(&self.state).snapshot.clone()
    }

    fn capabilities(&self) -> Capabilities {
        lock(&self.state).capabilities
    }

    fn follow_closely(&self, on: bool) {
        self.close.store(on, Ordering::Relaxed);
    }

    fn send(&self, command: Command) {
        let mut state = lock(&self.state);
        if !state.snapshot.status.is_available() {
            return;
        }
        state.snapshot.apply(&command, Instant::now());
        // Under the lock, so the worker sees the count and the optimistic
        // snapshot change together.
        if let Some(tx) = &self.commands
            && tx.send(command).is_ok()
        {
            state.sent += 1;
        }
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Worker<B> {
    backend: B,
    state: Arc<Mutex<State>>,
    commands: Receiver<Command>,
    /// Commands taken off the channel so far.
    handled: u64,
    cadence: Cadence,
    /// Failed polls in a row (see [`GRACE`]).
    misses: u32,
    /// [`Cadence::close`] wanted (set by the UI).
    close: Arc<AtomicBool>,
    baseline: Baseline,
}

impl<B: Backend> Worker<B> {
    fn run(mut self) {
        let mut next_poll = Instant::now();
        loop {
            let wait = next_poll.saturating_duration_since(Instant::now());
            let batch = match self.commands.recv_timeout(wait) {
                Ok(first) => {
                    let mut batch = vec![first];
                    batch.extend(self.commands.try_iter());
                    batch
                }
                Err(RecvTimeoutError::Timeout) => Vec::new(),
                Err(RecvTimeoutError::Disconnected) => return,
            };
            next_poll = Instant::now() + self.turn(batch);
        }
    }

    /// One exchange: run `batch` (maybe empty) and publish the answer.
    /// Returns how long until the next poll.
    fn turn(&mut self, batch: Vec<Command>) -> Duration {
        self.handled += batch.len() as u64;
        let batch = coalesce(batch);
        let sent = Instant::now();
        let mut fresh = self.backend.exchange(&batch);
        let transient = matches!(
            fresh.status,
            Status::Unavailable(Unavailable::NotResponding | Unavailable::Error(_))
        );
        if !transient {
            self.misses = 0;
        } else {
            self.misses += 1;
            crate::diag::note(|| {
                format!(
                    "player: no answer ({:?}), miss {}",
                    fresh.status, self.misses
                )
            });
            let shown = lock(&self.state).snapshot.status.is_available();
            if shown && self.misses <= GRACE {
                // What's shown stays (its clock runs on); ask again soon.
                return self.cadence.playing;
            }
        }
        let close = self.close.load(Ordering::Relaxed);
        let mut wait = self.cadence.after(&fresh.status, close);
        if !batch.is_empty() {
            wait = wait.min(self.cadence.after_command);
        }
        // Stale until the player catches up (see the module docs).
        let settling = !batch.is_empty() && fresh.status.is_available();
        let capabilities = self.backend.capabilities();
        let mut state = lock(&self.state);
        state.capabilities = capabilities;
        if state.sent == self.handled && !settling {
            self.baseline.settle(&mut fresh, &state.snapshot, sent);
            state.snapshot = fresh;
        }
        wait
    }
}

/// Setting the same thing twice in a row only needs the last value (ten
/// volume-up presses are one exchange with one `set sound volume`).
fn coalesce(batch: Vec<Command>) -> Vec<Command> {
    let setter = |c: &Command| {
        matches!(
            c,
            Command::Seek(_)
                | Command::SetShuffle(_)
                | Command::SetRepeat(_)
                | Command::SetVolume(_)
        )
    };
    let mut out: Vec<Command> = Vec::with_capacity(batch.len());
    for command in batch {
        match out.last_mut() {
            Some(last)
                if setter(&command) && mem::discriminant(last) == mem::discriminant(&command) =>
            {
                *last = command;
            }
            _ => out.push(command),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::Track;
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn track(id: &str) -> Option<Arc<Track>> {
        Some(Arc::new(Track {
            id: id.into(),
            duration: MS * 300_000,
            ..Track::default()
        }))
    }

    fn playing(id: &str, position: Duration, at: Instant) -> Snapshot {
        Snapshot {
            track: track(id),
            position,
            volume: 50,
            ..Snapshot::new(Status::Playing, at)
        }
    }

    /// A backend answering from a closure, recording each batch.
    struct Scripted<F>(F, Log);

    impl<F: FnMut(&[Command]) -> Snapshot + Send + 'static> Backend for Scripted<F> {
        fn exchange(&mut self, commands: &[Command]) -> Snapshot {
            self.1.lock().unwrap().push(commands.to_vec());
            (self.0)(commands)
        }
    }

    type Log = Arc<Mutex<Vec<Vec<Command>>>>;

    fn worker<F>(answer: F) -> (Polled, Worker<Scripted<F>>, Log)
    where
        F: FnMut(&[Command]) -> Snapshot + Send + 'static,
    {
        let log = Arc::default();
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State {
            snapshot: Snapshot::new(Status::Connecting, Instant::now()),
            sent: 0,
            capabilities: Capabilities::ALL,
        }));
        let close = Arc::new(AtomicBool::new(false));
        let handle = Polled {
            state: Arc::clone(&state),
            commands: Some(tx),
            close: Arc::clone(&close),
        };
        let worker = Worker {
            backend: Scripted(answer, Arc::clone(&log)),
            state,
            commands: rx,
            handled: 0,
            cadence: Cadence::default(),
            misses: 0,
            close,
            baseline: Baseline::new(Instant::now()),
        };
        (handle, worker, log)
    }

    #[test]
    fn a_poll_publishes_and_paces_by_status() {
        let (handle, mut worker, _) = worker(|_| playing("a", MS * 10, Instant::now()));
        assert_eq!(handle.snapshot().status, Status::Connecting);
        assert_eq!(worker.turn(Vec::new()), Cadence::default().playing);
        assert_eq!(handle.snapshot().status, Status::Playing);

        let cadence = Cadence::default();
        assert_eq!(cadence.after(&Status::Paused, false), cadence.idle);
        assert!(
            cadence.after(&Status::Unavailable(Unavailable::NotRunning), false) > cadence.playing
        );
        assert!(
            cadence.after(&Status::Unavailable(Unavailable::PermissionDenied), false)
                > cadence.idle
        );
        // Synced lyrics on screen: playing and paused are polled closely,
        // nothing else.
        handle.follow_closely(true);
        assert_eq!(worker.turn(Vec::new()), cadence.close);
        assert!(cadence.close < cadence.playing);
        assert_eq!(cadence.after(&Status::Paused, true), cadence.close);
        assert_eq!(cadence.after(&Status::Stopped, true), cadence.idle);
        assert_eq!(
            cadence.after(&Status::Unavailable(Unavailable::NotRunning), true),
            cadence.not_running
        );
    }

    /// lava-75z.22: a missed answer or two (Spotify busy over a track
    /// change) keeps what's shown, asked again at the playing pace; only
    /// a third in a row shows the problem. A player that's gone shows at
    /// once.
    #[test]
    fn a_missed_answer_or_two_keeps_what_is_shown() {
        let answers = Arc::new(Mutex::new(Vec::<Status>::new()));
        let script = Arc::clone(&answers);
        let (handle, mut worker, _) = worker(move |_| {
            let status = script.lock().unwrap().remove(0);
            Snapshot {
                status,
                ..playing("a", MS, Instant::now())
            }
        });
        let busy = Status::Unavailable(Unavailable::NotResponding);
        let odd = Status::Unavailable(Unavailable::Error("Spotify: odd".into()));
        answers.lock().unwrap().extend([
            Status::Playing,
            busy.clone(),
            odd.clone(),
            Status::Playing,
            busy.clone(),
            busy.clone(),
            busy.clone(),
            Status::Playing,
            Status::Unavailable(Unavailable::NotRunning),
        ]);
        let cadence = Cadence::default();
        worker.turn(Vec::new());
        // Two misses: still playing, asked again soon.
        for _ in 0..2 {
            assert_eq!(worker.turn(Vec::new()), cadence.playing);
            assert_eq!(handle.snapshot().status, Status::Playing);
        }
        // An answer resets the count.
        worker.turn(Vec::new());
        worker.turn(Vec::new());
        worker.turn(Vec::new());
        assert_eq!(handle.snapshot().status, Status::Playing);
        // The third miss in a row shows.
        assert_eq!(worker.turn(Vec::new()), cadence.not_responding);
        assert_eq!(handle.snapshot().status, busy);
        worker.turn(Vec::new());
        assert_eq!(handle.snapshot().status, Status::Playing);
        // Quit: at once.
        worker.turn(Vec::new());
        assert_eq!(
            handle.snapshot().status,
            Status::Unavailable(Unavailable::NotRunning)
        );
    }

    /// Before the first answer there's nothing to keep: a failure shows.
    #[test]
    fn a_first_failure_shows_at_once() {
        let (handle, mut worker, _) = worker(|_| {
            Snapshot::new(
                Status::Unavailable(Unavailable::NotResponding),
                Instant::now(),
            )
        });
        worker.turn(Vec::new());
        assert_eq!(
            handle.snapshot().status,
            Status::Unavailable(Unavailable::NotResponding)
        );
    }

    #[test]
    fn commands_are_optimistic_and_then_confirmed() {
        let mut paused = false;
        let (handle, mut worker, log) = worker(move |commands| {
            paused ^= commands.contains(&Command::PlayPause);
            let mut snap = playing("a", MS * 10, Instant::now());
            if paused {
                snap.status = Status::Paused;
            }
            snap
        });
        worker.turn(Vec::new());
        handle.play_pause();
        // Seen at once, before the worker has done anything.
        assert_eq!(handle.snapshot().status, Status::Paused);
        let batch: Vec<_> = worker.commands.try_iter().collect();
        assert_eq!(worker.turn(batch), Cadence::default().after_command);
        assert_eq!(handle.snapshot().status, Status::Paused);
        assert_eq!(log.lock().unwrap()[1], vec![Command::PlayPause]);
        worker.turn(Vec::new()); // the follow-up read confirms
        assert_eq!(handle.snapshot().status, Status::Paused);
    }

    #[test]
    fn the_read_right_after_a_command_is_not_trusted() {
        // Like Spotify: the pause shows only on the next read, and shuffle
        // never takes (its setter is a no-op in Spotify 1.2).
        let mut reads = 0;
        let (handle, mut worker, _) = worker(move |_| {
            reads += 1;
            let mut snap = playing("a", MS, Instant::now());
            snap.status = if reads >= 3 {
                Status::Paused
            } else {
                Status::Playing
            };
            snap
        });
        worker.turn(Vec::new());
        handle.play_pause();
        handle.set_shuffle(true);
        let batch: Vec<_> = worker.commands.try_iter().collect();
        worker.turn(batch);
        let snap = handle.snapshot();
        assert_eq!((snap.status, snap.shuffle), (Status::Paused, true));
        worker.turn(Vec::new());
        let snap = handle.snapshot();
        assert_eq!((snap.status, snap.shuffle), (Status::Paused, false));
    }

    #[test]
    fn a_command_that_finds_the_player_gone_says_so_at_once() {
        let mut reads = 0;
        let (handle, mut worker, _) = worker(move |_| {
            reads += 1;
            match reads {
                1 => playing("a", MS, Instant::now()),
                _ => Snapshot::new(Status::Unavailable(Unavailable::NotRunning), Instant::now()),
            }
        });
        worker.turn(Vec::new());
        handle.next();
        let batch: Vec<_> = worker.commands.try_iter().collect();
        worker.turn(batch);
        assert_eq!(
            handle.snapshot().status,
            Status::Unavailable(Unavailable::NotRunning)
        );
    }

    #[test]
    fn a_stale_poll_never_overwrites_a_newer_command() {
        // Pressed while a poll was in flight: the poll's (playing) answer is
        // dropped, the optimistic pause stays until the command's own answer.
        let (handle, mut worker, _) = worker(|_| playing("a", MS, Instant::now()));
        worker.turn(Vec::new());
        handle.play_pause();
        worker.turn(Vec::new()); // the in-flight poll
        assert_eq!(handle.snapshot().status, Status::Paused);
    }

    #[test]
    fn commands_are_ignored_while_unavailable() {
        let (handle, mut worker, _) =
            worker(|_| Snapshot::new(Status::Unavailable(Unavailable::NotRunning), Instant::now()));
        worker.turn(Vec::new());
        handle.next();
        assert_eq!(worker.commands.try_iter().count(), 0);
        let none = Polled::unavailable(Unavailable::Unsupported);
        none.play_pause();
        assert_eq!(
            none.snapshot().status,
            Status::Unavailable(Unavailable::Unsupported)
        );
    }

    #[test]
    fn repeated_setters_coalesce() {
        let batch = vec![
            Command::SetVolume(10),
            Command::SetVolume(20),
            Command::SetVolume(30),
            Command::PlayPause,
            Command::PlayPause,
            Command::Seek(MS),
            Command::Seek(MS * 2),
            Command::SetVolume(40),
        ];
        assert_eq!(
            coalesce(batch),
            vec![
                Command::SetVolume(30),
                Command::PlayPause,
                Command::PlayPause,
                Command::Seek(MS * 2),
                Command::SetVolume(40),
            ]
        );
    }

    /// A reading of `pos` ms, asked for at `sent` ms and answered at
    /// `reply` ms after `t0`, settled after `previous`.
    fn read(
        b: &mut Baseline,
        previous: &Snapshot,
        t0: Instant,
        pos: u64,
        sent: u64,
        reply: u64,
    ) -> Snapshot {
        let mut fresh = playing("a", MS * pos as u32, t0 + MS * reply as u32);
        b.settle(&mut fresh, previous, t0 + MS * sent as u32);
        fresh
    }

    /// Where `snap` puts playback at `t` ms after `t0`, in ms.
    fn at(snap: &Snapshot, t0: Instant, t: u64) -> f64 {
        snap.position_at(t0 + MS * t as u32).as_secs_f64() * 1000.0
    }

    #[test]
    fn the_baseline_narrows_to_the_quickest_reading() {
        // The truth: position = time + 5000 ms. Each reading was taken
        // somewhere inside its round trip.
        let t0 = Instant::now();
        let mut b = Baseline::new(t0);
        let start = Snapshot::new(Status::Connecting, t0);
        // A slow first poll (a track change: 2 s), read near its end.
        let one = read(&mut b, &start, t0, 6990, 0, 2000);
        assert!(
            (at(&one, t0, 3000) - 8000.0).abs() <= 20.0,
            "{}",
            at(&one, t0, 3000)
        );
        // Quick polls pin it down, whatever their own latency.
        let two = read(&mut b, &one, t0, 8004, 3000, 3020);
        let three = read(&mut b, &two, t0, 9012, 4000, 4015);
        assert!(
            (at(&three, t0, 5000) - 10_000.0).abs() <= 8.0,
            "{}",
            at(&three, t0, 5000)
        );
        // The window only narrows: a slow reading doesn't move it.
        let four = read(&mut b, &three, t0, 10_100, 5000, 5300);
        assert!((at(&four, t0, 6000) - at(&three, t0, 6000)).abs() < 1.0);
        assert!(Arc::ptr_eq(
            four.track.as_ref().unwrap(),
            three.track.as_ref().unwrap()
        ));
    }

    #[test]
    fn the_baseline_starts_again_after_a_jump_and_rides_out_noise() {
        let t0 = Instant::now();
        let mut b = Baseline::new(t0);
        let start = Snapshot::new(Status::Connecting, t0);
        let one = read(&mut b, &start, t0, 5005, 0, 10);
        let two = read(&mut b, &one, t0, 6005, 1000, 1010);
        // A player with coarse positions: 100 ms off, kept.
        let three = read(&mut b, &two, t0, 7100, 2000, 2010);
        assert!((at(&three, t0, 2000) - 7005.0).abs() < 6.0);
        // A seek elsewhere (or a pause and resume): followed at once.
        let four = read(&mut b, &three, t0, 60_000, 3000, 3010);
        assert!((at(&four, t0, 3010) - 60_000.0).abs() < 11.0);
        // Spotify stands still a moment after a seek: followed too, though
        // it's only 240 ms off (one reading doesn't make a window to hold).
        let still = read(&mut b, &four, t0, 60_010, 3250, 3260);
        assert!((at(&still, t0, 3260) - 60_010.0).abs() < 11.0);
        // A new track starts again too.
        let mut other = playing("b", MS * 1000, t0 + MS * 4010);
        b.settle(&mut other, &still, t0 + MS * 4000);
        assert!((at(&other, t0, 4010) - 1000.0).abs() < 11.0);
        // Paused: as read.
        let mut paused = playing("b", MS * 1500, t0 + MS * 5010);
        paused.status = Status::Paused;
        b.settle(&mut paused, &other, t0 + MS * 5000);
        assert_eq!(paused.position, MS * 1500);
    }

    #[test]
    fn noise_that_persists_is_followed() {
        let t0 = Instant::now();
        let mut b = Baseline::new(t0);
        let mut snap = Snapshot::new(Status::Connecting, t0);
        snap = read(&mut b, &snap, t0, 5005, 0, 10);
        // Playback slips 200 ms (a stall): after a few readings, followed.
        for i in 1..=4 {
            snap = read(&mut b, &snap, t0, 4805 + i * 1000, i * 1000, i * 1000 + 10);
        }
        assert!(
            (at(&snap, t0, 5000) - 9805.0).abs() < 11.0,
            "{}",
            at(&snap, t0, 5000)
        );
    }

    /// Replays a `live_timing_audit` CSV (`LAVATUI_TIMING_CSV`): its
    /// back-to-back readings, taken as the app's own every 250 ms (or
    /// `LAVATUI_REPLAY_MS`), through [`Baseline`], against the truth (the
    /// median of each moment's nearby readings). Prints the error and how
    /// long each jump took to settle within 50 ms.
    #[test]
    #[ignore = "needs a recorded CSV"]
    fn baseline_replay() {
        let csv = std::fs::read_to_string(std::env::var("LAVATUI_TIMING_CSV").expect("CSV"))
            .expect("read");
        let every: f64 = std::env::var("LAVATUI_REPLAY_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(250.0);
        // (send, recv, playing, pos, track)
        let reads: Vec<(f64, f64, bool, f64, String)> = csv
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split(',').collect();
                (f[0] == "truth").then(|| {
                    (
                        f[2].parse().unwrap(),
                        f[3].parse().unwrap(),
                        f[4] == "true",
                        f[5].parse().unwrap(),
                        f[6].to_owned(),
                    )
                })
            })
            .collect();
        let truth = |t: f64| -> Option<f64> {
            let near: Vec<&(f64, f64, bool, f64, String)> =
                reads.iter().filter(|r| (r.0 - t).abs() < 100.0).collect();
            if near.is_empty() || near.iter().any(|r| !r.2) {
                return None;
            }
            let mut b: Vec<f64> = near.iter().map(|r| r.3 - (r.0 + r.1) / 2.0).collect();
            b.sort_by(f64::total_cmp);
            // Only where they agree (not across a seek or a stall).
            (b[b.len() - 1] - b[0] < 30.0).then(|| t + b[b.len() / 2])
        };
        let t0 = Instant::now();
        let at = |ms: f64| t0 + Duration::from_secs_f64(ms.max(0.0) / 1000.0);
        let mut b = Baseline::new(t0);
        let mut snap = Snapshot::new(Status::Connecting, t0);
        let mut errors = Vec::new();
        let mut next = 0.0;
        let mut out_since: Option<f64> = None;
        let mut settles = Vec::new();
        let end = reads.last().map_or(0.0, |r| r.1);
        let mut t = 0.0;
        while t < end {
            if t >= next
                && let Some(r) = reads.iter().find(|r| r.0 >= t)
            {
                let mut fresh = Snapshot {
                    track: Some(Arc::new(Track {
                        id: r.4.clone(),
                        ..Track::default()
                    })),
                    position: Duration::from_secs_f64(r.3 / 1000.0),
                    ..Snapshot::new(if r.2 { Status::Playing } else { Status::Paused }, at(r.1))
                };
                b.settle(&mut fresh, &snap, at(r.0));
                snap = fresh;
                next = r.1 + every;
            }
            if let Some(truth) = truth(t) {
                let err = snap.position_at(at(t)).as_secs_f64() * 1000.0 - truth;
                errors.push(err);
                match (out_since, err.abs() > 50.0) {
                    (None, true) => out_since = Some(t),
                    (Some(from), false) => {
                        settles.push(t - from);
                        out_since = None;
                    }
                    _ => {}
                }
            }
            t += 10.0;
        }
        errors.sort_by(f64::total_cmp);
        let q = |p: f64| errors[((errors.len() - 1) as f64 * p) as usize];
        println!(
            "polled every {every} ms: error p1 {:+.1} p50 {:+.1} p99 {:+.1} ms; {} frames",
            q(0.01),
            q(0.5),
            q(0.99),
            errors.len()
        );
        println!("off by > 50 ms for (ms): {settles:.0?}");
    }

    // On a real thread.

    #[test]
    fn thread_command_then_refresh() {
        let log = Log::default();
        let mut track = 0;
        let backend = Scripted(
            move |commands: &[Command]| {
                track += commands.iter().filter(|c| **c == Command::Next).count();
                playing(&format!("t{track}"), MS, Instant::now())
            },
            Arc::clone(&log),
        );
        let source = Polled::spawn(backend, testing::fast());
        testing::wait_for(&source, "first poll", |s| s.status == Status::Playing);
        source.next();
        testing::wait_for(&source, "next track", |s| {
            s.track.as_ref().is_some_and(|t| t.id == "t1")
        });
        let batches = log.lock().unwrap();
        let nexts = batches.iter().flatten().filter(|c| **c == Command::Next);
        assert_eq!(nexts.count(), 1);
    }

    #[test]
    fn thread_stops_when_the_handle_drops() {
        let log = Log::default();
        let backend = Scripted(
            |_: &[Command]| playing("a", MS, Instant::now()),
            Arc::clone(&log),
        );
        let source = Polled::spawn(backend, testing::fast());
        testing::wait_for(&source, "first poll", |s| s.status == Status::Playing);
        drop(source);
        thread::sleep(MS * 60);
        let runs = log.lock().unwrap().len();
        thread::sleep(MS * 100);
        assert_eq!(log.lock().unwrap().len(), runs);
    }
}

/// Helpers for backend tests that drive a [`Polled`] on its thread.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// Polls every 20 ms, re-reads 5 ms after a command.
    pub fn fast() -> Cadence {
        Cadence {
            playing: MS * 20,
            idle: MS * 20,
            not_running: MS * 20,
            not_responding: MS * 20,
            broken: MS * 20,
            after_command: MS * 5,
            close: MS * 20,
        }
    }

    /// The first snapshot `ok` accepts; panics after 5 s.
    pub fn wait_for(source: &Polled, what: &str, ok: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snap = source.snapshot();
            if ok(&snap) {
                return snap;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {snap:?}"
            );
            thread::sleep(MS);
        }
    }
}
