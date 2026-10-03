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
//! ms however slow any one poll was. Between polls the position is
//! predicted, so polling only has to catch what changes in the player
//! itself: where the player says so ([`Backend::listen`]: a notification
//! or signal on change), the worker polls at once ([`Nudge`]), then once
//! more [`Cadence::after_event`] later, for a player that tells before
//! its state reads the new way.

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

    /// Which optional controls work. Read after every exchange: a backend
    /// may learn that its player ignores one (Spotify over MPRIS ignores
    /// shuffle / repeat).
    fn capabilities(&self) -> Capabilities {
        Capabilities::ALL
    }

    /// Start listening for the player's own change events, if it has any,
    /// calling `nudge` on each. Called once, on the worker thread; what it
    /// returns is dropped (stopping the listening) when the worker ends.
    fn listen(&mut self, _nudge: Nudge) -> Option<Box<dyn Send>> {
        None
    }
}

/// Polls brought on by change events are at least this far apart.
const EVENT_GAP: Duration = Duration::from_millis(250);

/// What the worker is sent: a command from the UI, or word that the
/// player changed.
#[derive(Debug)]
pub(crate) enum Msg {
    Command(Command),
    Changed,
    /// The handle is gone (a listener's [`Nudge`] keeps the channel open,
    /// so its closing can't say so).
    Stop,
}

/// Tells the worker the player changed on its own (a notification or
/// signal from it): it polls at once. Cheap and safe from any thread.
#[derive(Clone, Debug)]
pub struct Nudge(Sender<Msg>);

impl Nudge {
    /// A nudge and what it sends, for testing a listener.
    #[cfg(test)]
    pub(crate) fn channel() -> (Self, Receiver<Msg>) {
        let (tx, rx) = mpsc::channel();
        (Self(tx), rx)
    }

    /// Whether the worker is still there to hear it (a listener stops
    /// when it isn't).
    pub fn changed(&self) -> bool {
        self.0.send(Msg::Changed).is_ok()
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
    /// The re-read after a change event: the player may say it changed a
    /// moment before its state reads the new way.
    pub after_event: Duration,
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
            after_event: Duration::from_millis(300),
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
    commands: Option<Sender<Msg>>,
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
        let nudge = Nudge(tx.clone());
        let worker = Worker {
            backend,
            state: Arc::clone(&state),
            commands: rx,
            handled: 0,
            cadence,
            misses: 0,
            baseline: Baseline::new(Instant::now()),
        };
        let spawned = thread::Builder::new()
            .name("lavatui-media".into())
            .spawn(move || {
                crate::thread_qos::worker();
                worker.run(nudge)
            });
        match spawned {
            Ok(_) => Self {
                state,
                commands: Some(tx),
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

    fn send(&self, command: Command) {
        let mut state = lock(&self.state);
        if !state.snapshot.status.is_available() {
            return;
        }
        state.snapshot.apply(&command, Instant::now());
        // Under the lock, so the worker sees the count and the optimistic
        // snapshot change together.
        if let Some(tx) = &self.commands
            && tx.send(Msg::Command(command)).is_ok()
        {
            state.sent += 1;
        }
    }
}

impl Drop for Polled {
    fn drop(&mut self) {
        if let Some(tx) = &self.commands {
            let _ = tx.send(Msg::Stop);
        }
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Worker<B> {
    backend: B,
    state: Arc<Mutex<State>>,
    commands: Receiver<Msg>,
    /// Commands taken off the channel so far.
    handled: u64,
    cadence: Cadence,
    /// Failed polls in a row (see [`GRACE`]).
    misses: u32,
    baseline: Baseline,
}

impl<B: Backend> Worker<B> {
    fn run(mut self, nudge: Nudge) {
        // Kept until the worker ends; dropping it stops the listening.
        let _listening = self.backend.listen(nudge);
        let mut next_poll = Instant::now();
        // The last poll an event brought on, and an event held back.
        let mut evented: Option<Instant> = None;
        let mut held = false;
        loop {
            let wait = next_poll.saturating_duration_since(Instant::now());
            let (batch, changed, stop) = match self.commands.recv_timeout(wait) {
                Ok(first) => split(std::iter::once(first).chain(self.commands.try_iter())),
                Err(RecvTimeoutError::Timeout) => (Vec::new(), false, false),
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if stop {
                return;
            }
            let now = Instant::now();
            // An event polls at once, then once more after_event later;
            // more events within EVENT_GAP of it (a player whose timeline
            // ticks) are held back and folded into one poll at the gap.
            let fresh = changed && batch.is_empty();
            if fresh && evented.is_some_and(|at| now < at + EVENT_GAP) {
                next_poll = next_poll.min(evented.map_or(now, |at| at + EVENT_GAP));
                held = true;
                continue;
            }
            if mem::take(&mut held) || fresh {
                crate::diag::note(|| "player: change event".to_owned());
                evented = Some(now);
            }
            let mut wait = self.turn(batch);
            if fresh {
                wait = wait.min(self.cadence.after_event);
            }
            next_poll = Instant::now() + wait;
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
        let mut wait = self.cadence.after(&fresh.status);
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

/// The commands among `msgs`, whether a change event was too, and
/// whether the handle is gone.
fn split(msgs: impl Iterator<Item = Msg>) -> (Vec<Command>, bool, bool) {
    let (mut changed, mut stop) = (false, false);
    let commands = msgs
        .filter_map(|m| match m {
            Msg::Command(c) => Some(c),
            Msg::Changed => {
                changed = true;
                None
            }
            Msg::Stop => {
                stop = true;
                None
            }
        })
        .collect();
    (commands, changed, stop)
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
        let handle = Polled {
            state: Arc::clone(&state),
            commands: Some(tx),
        };
        let worker = Worker {
            backend: Scripted(answer, Arc::clone(&log)),
            state,
            commands: rx,
            handled: 0,
            cadence: Cadence::default(),
            misses: 0,
            baseline: Baseline::new(Instant::now()),
        };
        (handle, worker, log)
    }

    /// The commands waiting for `worker`, as its loop would batch them.
    fn drain<B: Backend>(worker: &Worker<B>) -> Vec<Command> {
        split(worker.commands.try_iter()).0
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

    /// A backend that hands its [`Nudge`] to the test and counts reads.
    struct Evented(Arc<Mutex<Option<Nudge>>>, Arc<Mutex<Vec<Instant>>>);

    impl Backend for Evented {
        fn exchange(&mut self, _: &[Command]) -> Snapshot {
            self.1.lock().unwrap().push(Instant::now());
            playing("a", MS, Instant::now())
        }

        fn listen(&mut self, nudge: Nudge) -> Option<Box<dyn Send>> {
            *self.0.lock().unwrap() = Some(nudge);
            None
        }
    }

    #[test]
    fn a_change_event_polls_at_once_and_once_more_after() {
        let (nudge, reads) = (Arc::default(), Arc::default());
        let slow = Cadence {
            playing: Duration::from_secs(60),
            // Far apart from "at once", even on a busy machine.
            after_event: MS * 400,
            ..testing::fast()
        };
        let source = Polled::spawn(Evented(Arc::clone(&nudge), Arc::clone(&reads)), slow);
        let count = || reads.lock().unwrap().len();
        let wait = |n: usize| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while count() < n {
                assert!(Instant::now() < deadline, "only {} reads", count());
                thread::sleep(MS);
            }
        };
        wait(1); // the first poll
        let nudge = loop {
            if let Some(n) = nudge.lock().unwrap().clone() {
                break n;
            }
            thread::sleep(MS);
        };
        thread::sleep(MS * 100);
        assert_eq!(count(), 1, "nothing until the player says so");
        let sent = Instant::now();
        // A burst of events is one read (and one re-read).
        for _ in 0..3 {
            assert!(nudge.changed());
        }
        wait(3);
        let reads = reads.lock().unwrap().clone();
        assert!(reads[1] - sent < MS * 300, "at once: {:?}", reads[1] - sent);
        assert!(reads[2] - reads[1] >= MS * 400, "then once more");
        thread::sleep(MS * 150);
        assert!(count() <= 4, "{}", count());
        // A stream of events (a timeline ticking): read every EVENT_GAP
        // at most, not once per event.
        let before = count();
        let start = Instant::now();
        while start.elapsed() < MS * 600 {
            nudge.changed();
            thread::sleep(MS * 10);
        }
        thread::sleep(MS * 100);
        let polls = count() - before;
        assert!((1..=6).contains(&polls), "{polls} reads for ~60 events");
        // Gone with the source: the listener hears so.
        drop(source);
        let deadline = Instant::now() + Duration::from_secs(2);
        while nudge.changed() {
            assert!(Instant::now() < deadline, "the worker never stopped");
            thread::sleep(MS);
        }
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
        let batch = drain(&worker);
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
        let batch = drain(&worker);
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
        let batch = drain(&worker);
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
        assert!(drain(&worker).is_empty());
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
    /// back-to-back readings, taken as the app's own every 1000 ms (or
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
            .unwrap_or(1000.0);
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
        // The worker owns the backend: once it's dropped, nothing runs.
        testing::released(&log);
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
            after_event: MS * 5,
        }
    }

    /// The first snapshot `ok` accepts; panics after 5 s.
    /// Wait until `shared` has no other owner: the worker that held it
    /// (through its backend) has ended.
    pub fn released<T>(shared: &Arc<T>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(shared) > 1 {
            assert!(Instant::now() < deadline, "the worker never stopped");
            thread::sleep(MS);
        }
    }

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
