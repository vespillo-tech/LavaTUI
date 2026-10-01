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

use std::mem;
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

    /// Which optional controls work (fixed for the backend's life).
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
        }
    }
}

impl Cadence {
    fn after(&self, status: &Status) -> Duration {
        match status {
            Status::Playing | Status::Connecting => self.playing,
            Status::Paused | Status::Stopped => self.idle,
            Status::Unavailable(Unavailable::NotRunning) => self.not_running,
            Status::Unavailable(Unavailable::NotResponding) => self.not_responding,
            Status::Unavailable(_) => self.broken,
        }
    }
}

/// A fresh sample within this of where the old one predicts playback to
/// be is the same playback: the old baseline is kept, so the extrapolated
/// position never jitters back and forth by a poll's latency.
const JITTER: Duration = Duration::from_millis(250);

#[derive(Debug)]
struct State {
    snapshot: Snapshot,
    /// Commands sent to the worker so far.
    sent: u64,
}

/// The handle the UI keeps. Dropping it stops the worker (after any
/// exchange in flight; nothing waits for it).
pub struct Polled {
    state: Arc<Mutex<State>>,
    commands: Option<Sender<Command>>,
    capabilities: Capabilities,
}

impl Polled {
    /// Start a worker thread for `backend`.
    pub fn spawn<B: Backend>(backend: B, cadence: Cadence) -> Self {
        let state = Arc::new(Mutex::new(State {
            snapshot: Snapshot::new(Status::Connecting, Instant::now()),
            sent: 0,
        }));
        let (tx, rx) = mpsc::channel();
        let capabilities = backend.capabilities();
        let worker = Worker {
            backend,
            state: Arc::clone(&state),
            commands: rx,
            handled: 0,
            cadence,
        };
        let spawned = thread::Builder::new()
            .name("lavatui-media".into())
            .spawn(move || worker.run());
        match spawned {
            Ok(_) => Self {
                state,
                commands: Some(tx),
                capabilities,
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
            })),
            commands: None,
            capabilities: Capabilities::NONE,
        }
    }
}

impl MediaSource for Polled {
    fn snapshot(&self) -> Snapshot {
        lock(&self.state).snapshot.clone()
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
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
        let fresh = self.backend.exchange(&batch);
        let mut wait = self.cadence.after(&fresh.status);
        if !batch.is_empty() {
            wait = wait.min(self.cadence.after_command);
        }
        // Stale until the player catches up (see the module docs).
        let settling = !batch.is_empty() && fresh.status.is_available();
        let mut state = lock(&self.state);
        if state.sent == self.handled && !settling {
            let previous = mem::replace(&mut state.snapshot, fresh);
            smooth(&mut state.snapshot, &previous);
        }
        wait
    }
}

/// Keep `previous`'s position baseline when `fresh` agrees with it (see
/// [`JITTER`]), and its `Arc<Track>` when the track is the same, so the UI
/// can tell "new track" by pointer.
fn smooth(fresh: &mut Snapshot, previous: &Snapshot) {
    if fresh.continues(previous) {
        let predicted = previous.position_at(fresh.sampled_at);
        if fresh.position.abs_diff(predicted) <= JITTER {
            fresh.position = previous.position;
            fresh.sampled_at = previous.sampled_at;
        }
    }
    if fresh.track.is_some() && fresh.track == previous.track {
        fresh.track.clone_from(&previous.track);
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
        }));
        let handle = Polled {
            state: Arc::clone(&state),
            commands: Some(tx),
            capabilities: Capabilities::ALL,
        };
        let worker = Worker {
            backend: Scripted(answer, Arc::clone(&log)),
            state,
            commands: rx,
            handled: 0,
            cadence: Cadence::default(),
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
        assert_eq!(cadence.after(&Status::Paused), cadence.idle);
        assert!(cadence.after(&Status::Unavailable(Unavailable::NotRunning)) > cadence.playing);
        assert!(cadence.after(&Status::Unavailable(Unavailable::PermissionDenied)) > cadence.idle);
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

    #[test]
    fn smoothing_keeps_the_baseline_within_jitter() {
        let t0 = Instant::now();
        let old = playing("a", MS * 1000, t0);
        // 1 s later the player says 2.1 s: 100 ms off the prediction, kept.
        let mut fresh = playing("a", MS * 2100, t0 + MS * 1000);
        smooth(&mut fresh, &old);
        assert_eq!((fresh.position, fresh.sampled_at), (MS * 1000, t0));
        assert!(Arc::ptr_eq(
            fresh.track.as_ref().unwrap(),
            old.track.as_ref().unwrap()
        ));
        // A real jump (seek elsewhere) resyncs.
        let mut fresh = playing("a", MS * 9000, t0 + MS * 1000);
        smooth(&mut fresh, &old);
        assert_eq!(fresh.position, MS * 9000);
        // A new track resyncs too.
        let mut fresh = playing("b", MS * 2000, t0 + MS * 1000);
        smooth(&mut fresh, &old);
        assert_eq!(fresh.sampled_at, t0 + MS * 1000);
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
