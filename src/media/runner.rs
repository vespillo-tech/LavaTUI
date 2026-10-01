//! Running a script in a child process, with a timeout.
//!
//! [`Runner`] is the seam the Spotify backend is tested through: tests swap
//! [`Osascript`] for a scripted fake.

use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Runs a script and returns its stdout. Blocking (called on the worker
/// thread only).
pub trait Runner: Send + 'static {
    fn run(&mut self, script: &str, timeout: Duration) -> Result<String, RunError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// Didn't finish in time; the process was killed.
    Timeout,
    /// The interpreter isn't there (not macOS, or a stripped system).
    Missing,
    /// Couldn't start or talk to the process.
    Io(String),
    /// Exited unsuccessfully; stderr, trimmed.
    Failed(String),
}

/// `/usr/bin/osascript`, script on stdin.
#[derive(Clone, Copy, Debug, Default)]
pub struct Osascript;

impl Osascript {
    const PATH: &str = "/usr/bin/osascript";
    /// How often a running script is checked for exit. Small next to
    /// osascript's own ~80 ms start-up.
    const POLL: Duration = Duration::from_millis(2);
}

impl Runner for Osascript {
    fn run(&mut self, script: &str, timeout: Duration) -> Result<String, RunError> {
        let mut child = Command::new(Self::PATH)
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| match err.kind() {
                io::ErrorKind::NotFound => RunError::Missing,
                _ => RunError::Io(err.to_string()),
            })?;
        let fed = child
            .stdin
            .take()
            .map_or(Ok(()), |mut stdin| stdin.write_all(script.as_bytes()));
        // stdin is dropped (closed) here, so osascript sees the whole script.
        if let Err(err) = fed {
            kill(&mut child);
            return Err(RunError::Io(err.to_string()));
        }

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    kill(&mut child);
                    return Err(RunError::Timeout);
                }
                Ok(None) => thread::sleep(Self::POLL),
                Err(err) => {
                    kill(&mut child);
                    return Err(RunError::Io(err.to_string()));
                }
            }
        };
        // The output is a few hundred bytes, far below a pipe's buffer, so
        // the child never blocked on a full pipe; read it now it has exited.
        let stdout = read_all(child.stdout.take());
        if status.success() {
            Ok(stdout)
        } else {
            Err(RunError::Failed(
                read_all(child.stderr.take()).trim().to_owned(),
            ))
        }
    }
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn read_all(pipe: Option<impl Read>) -> String {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(10);

    #[test]
    fn osascript_returns_stdout() {
        let out = Osascript.run("return \"héllo ✓\"", LONG).unwrap();
        assert_eq!(out, "héllo ✓\n");
    }

    #[test]
    fn osascript_errors_carry_stderr() {
        let err = Osascript.run("error \"nope\" number 42", LONG).unwrap_err();
        assert!(
            matches!(&err, RunError::Failed(msg) if msg.contains("(42)")),
            "{err:?}"
        );
    }

    #[test]
    fn osascript_is_killed_at_the_timeout() {
        let start = Instant::now();
        let err = Osascript
            .run("delay 5", Duration::from_millis(300))
            .unwrap_err();
        assert_eq!(err, RunError::Timeout);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
