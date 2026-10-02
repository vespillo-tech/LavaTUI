//! [`LyricsService`]: lookups off the frame loop.
//!
//! The app calls [`request`](LyricsService::request) on track change and
//! [`poll`](LyricsService::poll) each frame (never blocks). One worker
//! thread answers from the cache, else LRCLIB (then caches the answer),
//! parsing on the worker too. Skipping through tracks queues requests; the
//! worker only looks up the newest, and `poll` only returns answers to it.
//! Transient failures are retried with backoff, abandoned as soon as a newer
//! request arrives; if they persist, a stale cached answer is used, else
//! [`Answer::Offline`]. Each new entry is followed by a prune of the
//! cache ([`Cache::prune`]), on the worker too. Dropping the service ends the thread (after any
//! fetch in flight; nothing waits for it).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime};

use super::cache::Cache;
use super::client::{FetchError, Http, Lrclib};
use super::{Lyrics, RawLyrics, Track};

/// Waits before each retry of a transient failure.
pub const RETRY_AFTER: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];

#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Lyrics(Lyrics),
    /// LRCLIB has nothing (or only empty records) for the track.
    NotFound,
    /// No answer: LRCLIB unreachable and nothing cached.
    Offline(FetchError),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    /// The [`LyricsService::request`] this answers.
    pub id: u64,
    pub track: Track,
    pub answer: Answer,
}

struct Request {
    id: u64,
    track: Track,
}

pub struct LyricsService {
    tx: Sender<Request>,
    rx: Receiver<Response>,
    latest: u64,
}

impl LyricsService {
    /// The real thing: LRCLIB over HTTPS, cached in the default cache dir
    /// (uncached if the platform has none).
    pub fn start() -> std::io::Result<Self> {
        Self::spawn(
            Lrclib::new(),
            Cache::default_dir().map(Cache::new),
            RETRY_AFTER.to_vec(),
        )
    }

    pub fn spawn<H: Http + 'static>(
        client: Lrclib<H>,
        cache: Option<Cache>,
        retry_after: Vec<Duration>,
    ) -> std::io::Result<Self> {
        let (tx, requests) = mpsc::channel();
        let (responses, rx) = mpsc::channel();
        let worker = Worker {
            client,
            cache,
            retry_after,
            requests,
            responses,
        };
        thread::Builder::new()
            .name("lyrics".into())
            .spawn(move || {
                crate::thread_qos::worker();
                worker.run()
            })?;
        Ok(Self { tx, rx, latest: 0 })
    }

    /// Looks `track` up; supersedes any earlier request. Returns its id.
    pub fn request(&mut self, track: Track) -> u64 {
        self.latest += 1;
        // The worker only goes away if it panicked; `poll` then stays empty.
        let _ = self.tx.send(Request {
            id: self.latest,
            track,
        });
        self.latest
    }

    /// The answer to the latest request, once it's ready.
    pub fn poll(&mut self) -> Option<Response> {
        loop {
            match self.rx.try_recv() {
                Ok(response) if response.id == self.latest => return Some(response),
                Ok(_) => {}
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
            }
        }
    }
}

struct Worker<H> {
    client: Lrclib<H>,
    cache: Option<Cache>,
    retry_after: Vec<Duration>,
    requests: Receiver<Request>,
    responses: Sender<Response>,
}

impl<H: Http> Worker<H> {
    fn run(self) {
        let Ok(mut next) = self.requests.recv() else {
            return;
        };
        loop {
            next = self.newest(next);
            let (answer, superseded) = self.lookup(&next.track);
            if let Some(newer) = superseded {
                next = newer;
                continue;
            }
            let Some(answer) = answer else { return };
            let response = Response {
                id: next.id,
                track: next.track,
                answer,
            };
            if self.responses.send(response).is_err() {
                return;
            }
            match self.requests.recv() {
                Ok(request) => next = request,
                Err(_) => return,
            }
        }
    }

    /// Skips queued requests that a later one replaces.
    fn newest(&self, mut request: Request) -> Request {
        while let Ok(newer) = self.requests.try_recv() {
            request = newer;
        }
        request
    }

    /// The answer, or the newer request that interrupted a retry wait.
    /// `(None, None)`: the service is gone.
    fn lookup(&self, track: &Track) -> (Option<Answer>, Option<Request>) {
        let cached = self
            .cache
            .as_ref()
            .and_then(|c| c.get(track, SystemTime::now()));
        if let Some(hit) = cached.as_ref().filter(|hit| !hit.stale) {
            return (Some(answer(hit.value.as_ref())), None);
        }
        let mut waits = self.retry_after.iter();
        loop {
            match self.client.fetch(track) {
                Ok(value) => {
                    if let Some(cache) = &self.cache {
                        // Best effort: a failed write only costs a refetch.
                        let now = SystemTime::now();
                        let _ = cache.put(track, value.as_ref(), now);
                        cache.prune(now);
                    }
                    return (Some(answer(value.as_ref())), None);
                }
                Err(error) => {
                    let wait = waits.next().filter(|_| transient(&error));
                    let Some(&wait) = wait else {
                        crate::diag::note(|| {
                            format!(
                                "lyrics: lookup for {} failed: {error:?}{}",
                                crate::diag::tag(&format!("{}\u{1f}{}", track.artist, track.title)),
                                if cached.is_some() {
                                    " (old answer used)"
                                } else {
                                    ""
                                }
                            )
                        });
                        let answer = match cached {
                            Some(stale) => answer(stale.value.as_ref()),
                            None => Answer::Offline(error),
                        };
                        return (Some(answer), None);
                    };
                    match self.requests.recv_timeout(wait) {
                        Ok(newer) => return (None, Some(newer)),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return (None, None),
                    }
                }
            }
        }
    }
}

fn answer(value: Option<&RawLyrics>) -> Answer {
    value
        .and_then(RawLyrics::lyrics)
        .map_or(Answer::NotFound, Answer::Lyrics)
}

/// Worth retrying: no reply, rate limiting, or a server-side error.
fn transient(error: &FetchError) -> bool {
    match error {
        FetchError::Network(_) => true,
        FetchError::Status(code) => *code == 429 || *code >= 500,
        FetchError::Malformed(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::lyrics::cache::tests::{TempDir, track};
    use crate::lyrics::client::tests::{Mock, RECORD, ok, status};

    const FAST: [Duration; 2] = [Duration::from_millis(5), Duration::from_millis(5)];

    fn service(mock: &Mock, cache: Option<Cache>) -> LyricsService {
        LyricsService::spawn(
            Lrclib::with_http(mock.clone(), "http://test"),
            cache,
            FAST.to_vec(),
        )
        .unwrap()
    }

    /// Polls until an answer arrives (or panics after 5 s).
    fn wait(service: &mut LyricsService) -> Response {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(response) = service.poll() {
                return response;
            }
            assert!(Instant::now() < deadline, "no answer");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn is_synced(answer: &Answer) -> bool {
        matches!(answer, Answer::Lyrics(Lyrics::Synced(_)))
    }

    #[test]
    fn fetches_parses_and_caches() {
        let tmp = TempDir::new("worker-cache");
        let mock = Mock::new([ok(RECORD)]);
        let mut s = service(&mock, Some(Cache::new(tmp.0.clone())));
        let id = s.request(track("Song", 200));
        let r = wait(&mut s);
        assert_eq!(r.id, id);
        assert!(is_synced(&r.answer));
        // Second time: from the cache, no request.
        s.request(track("Song", 200));
        assert!(is_synced(&wait(&mut s).answer));
        assert_eq!(mock.urls().len(), 1);
    }

    #[test]
    fn not_found_is_cached_too() {
        let tmp = TempDir::new("worker-negative");
        let mock = Mock::new([status(404), ok("[]")]);
        let mut s = service(&mock, Some(Cache::new(tmp.0.clone())));
        s.request(track("Nothing", 100));
        assert_eq!(wait(&mut s).answer, Answer::NotFound);
        s.request(track("Nothing", 100));
        assert_eq!(wait(&mut s).answer, Answer::NotFound);
        assert_eq!(mock.urls().len(), 2, "one get + one search, then cached");
    }

    #[test]
    fn transient_failures_retry_then_report_offline() {
        let mock = Mock::new([status(503), Err("down".into()), ok(RECORD)]);
        let mut s = service(&mock, None);
        s.request(track("Song", 200));
        assert!(is_synced(&wait(&mut s).answer));
        assert_eq!(mock.urls().len(), 3);

        let mock = Mock::new([Err("a".into()), Err("b".into()), Err("c".into())]);
        let mut s = service(&mock, None);
        s.request(track("Song", 200));
        assert_eq!(
            wait(&mut s).answer,
            Answer::Offline(FetchError::Network("c".into()))
        );
        // A malformed reply isn't retried.
        let mock = Mock::new([ok("nope")]);
        let mut s = service(&mock, None);
        s.request(track("Song", 200));
        assert!(matches!(
            wait(&mut s).answer,
            Answer::Offline(FetchError::Malformed(_))
        ));
        assert_eq!(mock.urls().len(), 1);
    }

    #[test]
    fn offline_falls_back_to_a_stale_entry() {
        let tmp = TempDir::new("worker-stale");
        let mut cache = Cache::new(tmp.0.clone());
        let raw = RawLyrics {
            plain: Some("old".into()),
            ..RawLyrics::default()
        };
        cache
            .put(&track("Song", 200), Some(&raw), SystemTime::now())
            .unwrap();
        cache.policy.plain = Duration::ZERO;
        let mock = Mock::new([]);
        let mut s = service(&mock, Some(cache));
        s.request(track("Song", 200));
        assert_eq!(
            wait(&mut s).answer,
            Answer::Lyrics(Lyrics::Plain(vec!["old".into()]))
        );
        assert_eq!(
            mock.urls().len(),
            3,
            "tried (and retried) before falling back"
        );
    }

    #[test]
    fn only_the_newest_request_is_answered() {
        // The first lookup hangs in retries until superseded.
        let mock = Mock::new([Err("slow".into()), ok(RECORD)]);
        let mut s = LyricsService::spawn(
            Lrclib::with_http(mock.clone(), "http://test"),
            None,
            vec![Duration::from_secs(30)],
        )
        .unwrap();
        s.request(track("First", 200));
        let deadline = Instant::now() + Duration::from_secs(5);
        while mock.urls().is_empty() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let second = s.request(track("Second", 200));
        let r = wait(&mut s);
        assert_eq!((r.id, r.track.title.as_str()), (second, "Second"));
        assert!(is_synced(&r.answer));
        assert!(s.poll().is_none());
    }

    #[test]
    fn stale_answers_are_dropped_by_poll() {
        let mock = Mock::new([ok(RECORD), ok(RECORD)]);
        let mut s = service(&mock, None);
        s.request(track("A", 200));
        // Let the first answer land before asking again.
        let deadline = Instant::now() + Duration::from_secs(5);
        while mock.urls().is_empty() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        thread::sleep(Duration::from_millis(20));
        let b = s.request(track("B", 200));
        assert_eq!(wait(&mut s).id, b);
    }

    #[test]
    fn transient_classification() {
        assert!(transient(&FetchError::Status(503)));
        assert!(transient(&FetchError::Status(429)));
        assert!(!transient(&FetchError::Status(400)));
        assert!(transient(&FetchError::Network(String::new())));
    }
}
