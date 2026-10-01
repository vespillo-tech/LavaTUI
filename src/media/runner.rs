//! A long-lived script process, asked one request at a time.
//!
//! Starting `osascript` costs ~50-90 ms of CPU (loading the OSA machinery
//! and compiling), far more than the few Apple events a poll sends, so the
//! Spotify backend keeps one running: [`Osascript`] starts it on first use,
//! writes each request as a line on its stdin and reads the reply up to
//! [`END`]. A reply that doesn't come in time kills the process; the next
//! request starts a fresh one.
//!
//! No orphans: the script exits when its stdin closes, which happens when
//! the [`Osascript`] is dropped (it also kills the child) and, should the
//! app die without unwinding, when the kernel closes our end of the pipe.
//!
//! [`Runner`] is the seam the Spotify backend is tested through: tests swap
//! [`Osascript`] for a scripted fake.

use std::io::{self, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

/// Ends every reply (the script removes it from the text it sends).
pub const END: &str = "\u{4}\n";

/// Sends one request to a script and returns its reply. Blocking (called on
/// the worker thread only).
pub trait Runner: Send + 'static {
    /// `request` is one line (no newline in it). The reply comes without
    /// its [`END`].
    fn run(&mut self, request: &str, timeout: Duration) -> Result<String, RunError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunError {
    /// No reply in time; the process was killed.
    Timeout,
    /// The interpreter isn't there (not macOS, or a stripped system).
    Missing,
    /// Couldn't start or talk to the process.
    Io(String),
    /// The process exited (a script that doesn't compile, say); its
    /// stderr, trimmed.
    Failed(String),
}

/// `/usr/bin/osascript` running `script`, a loop that reads a request line
/// from stdin and writes one reply ending in [`END`] to stdout until stdin
/// closes.
pub struct Osascript {
    script: String,
    process: Option<Process>,
}

struct Process {
    child: Child,
    stdin: ChildStdin,
    /// Replies, split off stdout by a reader thread; disconnected at EOF.
    replies: Receiver<String>,
}

impl Osascript {
    const PATH: &str = "/usr/bin/osascript";

    pub fn new(script: impl Into<String>) -> Self {
        Self {
            script: script.into(),
            process: None,
        }
    }

    fn start(&self) -> Result<Process, RunError> {
        let mut command = Command::new(Self::PATH);
        // One `-e` per line: the script's own stdin stays free for requests.
        for line in self.script.lines() {
            command.arg("-e").arg(line);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| match err.kind() {
                io::ErrorKind::NotFound => RunError::Missing,
                _ => RunError::Io(err.to_string()),
            })?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            kill(&mut child);
            return Err(RunError::Io("osascript pipes missing".into()));
        };
        let (tx, replies) = mpsc::channel();
        let reader = thread::Builder::new()
            .name("lavatui-osascript".into())
            .spawn(move || split_replies(stdout, |reply| tx.send(reply).is_ok()));
        if let Err(err) = reader {
            kill(&mut child);
            return Err(RunError::Io(err.to_string()));
        }
        Ok(Process {
            child,
            stdin,
            replies,
        })
    }

    /// Kill the process and say why it went away.
    fn stop(&mut self, why: RunError) -> RunError {
        let Some(mut process) = self.process.take() else {
            return why;
        };
        kill(&mut process.child);
        if why != RunError::Timeout
            && let Some(mut stderr) = process.child.stderr.take()
        {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            if !text.trim().is_empty() {
                return RunError::Failed(text.trim().to_owned());
            }
        }
        why
    }
}

impl Runner for Osascript {
    fn run(&mut self, request: &str, timeout: Duration) -> Result<String, RunError> {
        if self.process.is_none() {
            self.process = Some(self.start()?);
        }
        let process = self.process.as_mut().expect("just started");
        // A late reply to a request that timed out can't be here: a timeout
        // kills the process.
        let line = format!("{request}\n");
        if let Err(err) = process.stdin.write_all(line.as_bytes()) {
            // A broken pipe means the script exited: its stderr says why.
            return Err(self.stop(RunError::Io(err.to_string())));
        }
        match process.replies.recv_timeout(timeout) {
            Ok(reply) => Ok(reply),
            Err(RecvTimeoutError::Timeout) => Err(self.stop(RunError::Timeout)),
            Err(RecvTimeoutError::Disconnected) => Err(self.stop(RunError::Failed(
                "osascript exited without an answer".into(),
            ))),
        }
    }
}

impl Drop for Osascript {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            kill(&mut process.child);
        }
    }
}

/// Read `from` to EOF, handing each [`END`]-terminated reply to `send`
/// (until it returns false).
fn split_replies(mut from: impl Read, mut send: impl FnMut(String) -> bool) {
    let mut pending = Vec::new();
    let mut buf = [0; 4096];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(at) = find(&pending, END.as_bytes()) {
            let reply = String::from_utf8_lossy(&pending[..at]).into_owned();
            pending.drain(..at + END.len());
            if !send(reply) {
                return;
            }
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// An `osascript` loop answering each request with `reply(request)`,
/// built around a handler body; for the tests here and in `spotify`.
#[cfg(test)]
pub(crate) fn echo_script(body: &str) -> String {
    format!(
        "use framework \"Foundation\"\n\
         use scripting additions\n\
         on reply(t)\n\
         set s to current application's NSString's stringWithString:(t & (character id 4) & linefeed)\n\
         (current application's NSFileHandle's fileHandleWithStandardOutput())'s writeData:(s's dataUsingEncoding:4)\n\
         end reply\n\
         on answer(req)\n\
         {body}\n\
         end answer\n\
         set stdin to current application's NSFileHandle's fileHandleWithStandardInput()\n\
         repeat\n\
         set d to stdin's availableData()\n\
         if (d's |length|()) as integer is 0 then exit repeat\n\
         set req to (current application's NSString's alloc()'s initWithData:d encoding:4) as text\n\
         my reply(answer(text 1 thru -2 of req))\n\
         end repeat\n"
    )
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    const LONG: Duration = Duration::from_secs(10);

    #[test]
    fn replies_are_split_at_the_end_marker_across_reads() {
        let stream = "one\u{4}\ntw\no\u{4}\n\u{4}\ntail";
        // Hand the bytes over a few at a time.
        struct Chunks<'a>(&'a [u8]);
        impl Read for Chunks<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = self.0.len().min(3).min(buf.len());
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let mut got = Vec::new();
        split_replies(Chunks(stream.as_bytes()), |r| {
            got.push(r);
            true
        });
        assert_eq!(got, ["one", "tw\no", ""]);
    }

    #[test]
    fn one_process_answers_many_requests() {
        let mut osa = Osascript::new(echo_script("return \"héllo ✓ \" & req"));
        let first = Instant::now();
        assert_eq!(osa.run("a", LONG).unwrap(), "héllo ✓ a");
        let started = first.elapsed();
        let pid = osa.process.as_ref().unwrap().child.id();
        let again = Instant::now();
        for i in 0..5 {
            assert_eq!(
                osa.run(&i.to_string(), LONG).unwrap(),
                format!("héllo ✓ {i}")
            );
        }
        assert_eq!(osa.process.as_ref().unwrap().child.id(), pid);
        // Not a fresh osascript each time.
        assert!(
            again.elapsed() < started * 5,
            "{:?} vs {started:?}",
            again.elapsed()
        );
    }

    #[test]
    fn a_script_that_does_not_compile_fails_with_its_stderr() {
        let mut osa = Osascript::new("this is not applescript (");
        let err = osa.run("x", LONG).unwrap_err();
        assert!(
            matches!(&err, RunError::Failed(msg) if msg.contains("error")),
            "{err:?}"
        );
    }

    #[test]
    fn a_slow_reply_kills_the_process_and_the_next_request_restarts_it() {
        let mut osa = Osascript::new(echo_script("if req is \"slow\" then delay 5\nreturn req"));
        assert_eq!(osa.run("fast", LONG).unwrap(), "fast");
        let pid = osa.process.as_ref().unwrap().child.id();
        let start = Instant::now();
        assert_eq!(
            osa.run("slow", Duration::from_millis(300)),
            Err(RunError::Timeout)
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(osa.process.is_none());
        assert!(!alive(pid));
        assert_eq!(osa.run("fast", LONG).unwrap(), "fast");
    }

    #[test]
    fn dropping_it_ends_the_process() {
        let mut osa = Osascript::new(echo_script("return req"));
        osa.run("x", LONG).unwrap();
        let pid = osa.process.as_ref().unwrap().child.id();
        drop(osa);
        assert!(!alive(pid));
    }

    /// The script quits on its own when stdin closes (our process dying
    /// without running `Drop`).
    #[test]
    fn the_script_exits_when_stdin_closes() {
        let mut osa = Osascript::new(echo_script("return req"));
        osa.run("x", LONG).unwrap();
        let mut process = osa.process.take().unwrap();
        drop(process.stdin);
        let start = Instant::now();
        while process.child.try_wait().unwrap().is_none() {
            assert!(start.elapsed() < Duration::from_secs(5), "still running");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn alive(pid: u32) -> bool {
        Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}
