//! The saved lyrics and covers, as the settings screen shows them: how
//! much there is, and `clear saved lyrics and covers`.
//!
//! Measuring and clearing read the disk, so they run on a worker thread,
//! started the first time they're asked for; the frame only sends jobs
//! and picks up answers ([`SavedFiles::poll`]).

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crate::disk_cache::{self, Usage};
use crate::lyrics::cache as lyrics_cache;
use crate::media::art;

/// How often an open `music & lyrics` page measures again.
pub const REMEASURE: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    Measure,
    Clear,
}

/// What a job found: lyrics, covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    pub lyrics: Usage,
    pub covers: Usage,
}

impl Saved {
    pub fn total(self) -> Usage {
        self.lyrics + self.covers
    }
}

/// A cache folder and its files' extension.
type Folder = (PathBuf, &'static str);
/// Jobs to the worker, and its answers.
type Channel = (Sender<Job>, Receiver<(Job, Saved)>);

pub struct SavedFiles {
    /// Lyrics, then covers (`None`: no cache folder on this platform).
    folders: [Option<Folder>; 2],
    worker: Option<Channel>,
    /// Jobs sent and not answered yet.
    pending: usize,
    /// The latest measurement, and when it was asked for.
    pub saved: Option<Saved>,
    asked: Option<Instant>,
}

impl Default for SavedFiles {
    /// The real cache folders (none in tests: they never touch the disk
    /// unless given folders by [`SavedFiles::in_folders`]).
    fn default() -> Self {
        #[cfg(not(test))]
        let folders = [
            lyrics_cache::Cache::default_dir().map(|d| (d, lyrics_cache::EXT)),
            art::cache_dir().map(|d| (d, art::CACHE_EXT)),
        ];
        #[cfg(test)]
        let folders = [None, None];
        Self::with(folders)
    }
}

impl SavedFiles {
    fn with(folders: [Option<Folder>; 2]) -> Self {
        Self {
            folders,
            worker: None,
            pending: 0,
            saved: None,
            asked: None,
        }
    }

    /// Lyrics in `lyrics`, covers in `covers` (tests).
    #[cfg(test)]
    pub fn in_folders(lyrics: PathBuf, covers: PathBuf) -> Self {
        Self::with([
            Some((lyrics, lyrics_cache::EXT)),
            Some((covers, art::CACHE_EXT)),
        ])
    }

    /// Whether there are cache folders at all.
    pub fn exist(&self) -> bool {
        self.folders.iter().any(Option::is_some)
    }

    /// A job is on its way.
    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// Measure, unless a job is under way or the last measurement is
    /// newer than [`REMEASURE`].
    pub fn measure(&mut self, now: Instant) {
        let due = self.asked.is_none_or(|at| now - at >= REMEASURE);
        if due && !self.busy() {
            self.asked = Some(now);
            self.send(Job::Measure);
        }
    }

    /// Delete them all (then measure).
    pub fn clear(&mut self) {
        self.send(Job::Clear);
    }

    /// Answers that have arrived. `true` once a clear is done.
    pub fn poll(&mut self) -> bool {
        let mut cleared = false;
        while let Some((_, rx)) = &self.worker {
            match rx.try_recv() {
                Ok((job, saved)) => {
                    self.pending = self.pending.saturating_sub(1);
                    self.saved = Some(saved);
                    cleared |= job == Job::Clear;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.pending = 0;
                }
            }
        }
        cleared
    }

    fn send(&mut self, job: Job) {
        if !self.exist() {
            self.saved = Some(Saved::default());
            return;
        }
        if self.worker.is_none() {
            self.worker = self.spawn();
        }
        if let Some((tx, _)) = &self.worker
            && tx.send(job).is_ok()
        {
            self.pending += 1;
        }
    }

    fn spawn(&self) -> Option<Channel> {
        let (tx, jobs) = mpsc::channel::<Job>();
        let (answers, rx) = mpsc::channel();
        let folders = self.folders.clone();
        thread::Builder::new()
            .name("lavatui-saved".into())
            .spawn(move || {
                crate::thread_qos::worker();
                let usage = |f: &Option<Folder>| {
                    f.as_ref()
                        .map_or(Usage::default(), |(dir, ext)| disk_cache::usage(dir, ext))
                };
                for job in jobs {
                    if job == Job::Clear {
                        for (dir, ext) in folders.iter().flatten() {
                            disk_cache::clear(dir, ext);
                        }
                    }
                    let saved = Saved {
                        lyrics: usage(&folders[0]),
                        covers: usage(&folders[1]),
                    };
                    if answers.send((job, saved)).is_err() {
                        return;
                    }
                }
            })
            .ok()?;
        Some((tx, rx))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::lyrics::cache::tests::TempDir;

    /// Polls until the jobs are answered (or panics after 5 s).
    pub(crate) fn settle(files: &mut SavedFiles) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut cleared = false;
        while files.busy() {
            cleared |= files.poll();
            assert!(Instant::now() < deadline, "no answer");
            thread::sleep(Duration::from_millis(2));
        }
        cleared
    }

    #[test]
    fn measures_then_clears_on_the_worker() {
        let tmp = TempDir::new("saved-files");
        let (lyrics, covers) = (tmp.0.join("lyrics"), tmp.0.join("art"));
        fs::create_dir_all(&lyrics).unwrap();
        fs::create_dir_all(&covers).unwrap();
        fs::write(lyrics.join("a.json"), [0; 100]).unwrap();
        fs::write(covers.join("b.img"), [0; 2000]).unwrap();
        fs::write(covers.join("note.txt"), "mine").unwrap();
        let mut files = SavedFiles::in_folders(lyrics, covers.clone());
        let t0 = Instant::now();
        files.measure(t0);
        assert!(!settle(&mut files));
        let saved = files.saved.unwrap();
        assert_eq!(
            saved.lyrics,
            Usage {
                files: 1,
                bytes: 100
            }
        );
        assert_eq!(
            saved.total(),
            Usage {
                files: 2,
                bytes: 2100
            }
        );
        // Not again until it's due.
        files.measure(t0 + Duration::from_secs(1));
        assert!(!files.busy());

        files.clear();
        assert!(settle(&mut files));
        assert_eq!(files.saved.unwrap().total(), Usage::default());
        assert!(covers.join("note.txt").exists());
    }

    #[test]
    fn no_folders_is_nothing_saved() {
        let mut files = SavedFiles::default();
        assert!(!files.exist());
        files.measure(Instant::now());
        assert!(!files.busy());
        assert_eq!(files.saved, Some(Saved::default()));
    }
}
