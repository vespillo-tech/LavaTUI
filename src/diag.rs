//! Opt-in notes on what the music widgets went through, for problems that
//! come and go (lava-75z.22): `LAVATUI_MEDIA_LOG=<file>` appends one line
//! per event (a track read with details missing, a cover or lyrics lookup
//! that failed, a Spotify Web API error) with the time. Off (the default),
//! [`note`] costs one atomic load and never formats anything.
//!
//! No names: a track is told apart by a short hash of its id, so a log can
//! be shared without saying what was playing.
//!
//! Callers on the frame loop only hand a line to a channel; the file is
//! opened and written on the writer's own thread.

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender};
use std::thread;

/// The environment variable naming the log file.
pub const ENV: &str = "LAVATUI_MEDIA_LOG";

static LOG: OnceLock<Option<Sender<String>>> = OnceLock::new();

fn sender() -> Option<&'static Sender<String>> {
    LOG.get_or_init(|| {
        let path = std::env::var_os(ENV).filter(|p| !p.is_empty())?;
        let (tx, rx) = mpsc::channel::<String>();
        thread::Builder::new()
            .name("lavatui-diag".into())
            .spawn(move || {
                crate::thread_qos::worker();
                let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
                    return;
                };
                for line in rx {
                    let _ = writeln!(file, "{} {line}", jiff::Timestamp::now());
                }
            })
            .ok()?;
        Some(tx)
    })
    .as_ref()
}

/// Log `line()` when the log is on (from any thread; never blocks).
pub fn note(line: impl FnOnce() -> String) {
    if let Some(tx) = sender() {
        let _ = tx.send(line());
    }
}

/// A short, stable stand-in for a track id (or any name) in the log.
pub fn tag(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(text.as_bytes());
    hash[..3].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_short_stable_and_differ() {
        assert_eq!(tag("spotify:track:a"), tag("spotify:track:a"));
        assert_ne!(tag("spotify:track:a"), tag("spotify:track:b"));
        assert_eq!(tag("x").len(), 6);
    }
}
