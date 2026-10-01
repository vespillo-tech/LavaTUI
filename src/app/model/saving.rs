//! One config writer owns Store (including its known-file baseline).
//! Frames only try_send/try_recv; file I/O and fsync happen on the worker.
use std::io;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};

use crate::config::{Settings, store::Store};

type Saved = (Settings, Result<(), String>);

pub(super) struct Saver {
    requests: SyncSender<Settings>,
    results: Receiver<Saved>,
    worker: JoinHandle<Store>,
    pending: Option<Settings>,
    inflight: usize,
    stopped: bool,
}

impl Saver {
    pub fn start(store: Store) -> io::Result<Self> {
        let (requests, incoming) = mpsc::sync_channel::<Settings>(1);
        let (outgoing, results) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("config-save".into())
            .spawn(move || {
                crate::thread_qos::worker();
                let mut store = store;
                while let Ok(settings) = incoming.recv() {
                    let result = store.save(&settings);
                    if outgoing.send((settings, result)).is_err() {
                        break;
                    }
                }
                store
            })?;
        Ok(Self {
            requests,
            results,
            worker,
            pending: None,
            inflight: 0,
            stopped: false,
        })
    }

    pub fn submit(&mut self, settings: Settings) {
        // Newer snapshots supersede an unsent one. Never wait for the writer.
        self.pending = Some(settings);
        self.pump();
    }

    fn pump(&mut self) {
        if let Some(settings) = self.pending.take() {
            match self.requests.try_send(settings) {
                Ok(()) => self.inflight += 1,
                Err(TrySendError::Full(settings)) => self.pending = Some(settings),
                Err(TrySendError::Disconnected(_)) => {}
            }
        }
    }

    pub fn poll(&mut self) -> Result<Option<Saved>, String> {
        self.pump();
        match self.results.try_recv() {
            Ok(saved) => {
                self.inflight -= 1;
                Ok(Some(saved))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) if !self.stopped => {
                self.stopped = true;
                self.inflight = 0;
                self.pending = None;
                Err("config: save worker stopped".into())
            }
            Err(TryRecvError::Disconnected) => Ok(None),
        }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.inflight > 0
    }

    /// Only after the frame loop has stopped: finish the newest snapshot,
    /// drain results and join before exiting (never lose a last-key save).
    pub fn finish(mut self) -> Result<(Store, Vec<Saved>), String> {
        if let Some(settings) = self.pending.take() {
            self.requests
                .send(settings)
                .map_err(|_| "config: save worker stopped")?;
        }
        drop(self.requests);
        let saved = self.results.iter().collect();
        let store = self
            .worker
            .join()
            .map_err(|_| "config: save worker panicked")?;
        Ok((store, saved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_flushes_the_latest_snapshot_and_preserves_hand_edits() {
        let path = std::env::temp_dir().join(format!("lavatui-worker-{}.toml", std::process::id()));
        std::fs::write(&path, "# keep\n[lamp]\nheat = 2\n").unwrap();
        let mut store = Store::new(Some(path.clone()));
        let mut settings = store.load().settings;
        std::fs::write(
            &path,
            "# keep\n[lamp]\nheat = 2\n[pomodoro]\nfocus_min = 50\n",
        )
        .unwrap();
        let mut saver = Saver::start(store).unwrap();
        for heat in 0..=5 {
            settings.lamp.heat = heat;
            saver.submit(settings.clone());
        }
        let (mut store, results) = saver.finish().unwrap();
        assert!(results.iter().all(|(_, r)| r.is_ok()));
        let loaded = store.load().settings;
        assert_eq!(loaded.lamp.heat, 5);
        assert_eq!(loaded.pomodoro.focus_min, 50);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("# keep")
        );
        std::fs::remove_file(path).unwrap();
    }
}
