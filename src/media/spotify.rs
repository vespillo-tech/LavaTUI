//! The Spotify desktop app on macOS, driven through AppleScript (the
//! module only exists on macOS).
//!
//! Every exchange is one `osascript` run: the queued commands, then a read
//! of the whole state as one delimited record. The script first checks
//! `application id … is running` and only then compiles the Spotify part
//! (`run script`): compiling a `tell application` block launches the app
//! to load its dictionary, and we must never launch Spotify.
//!
//! The record is `lavatui1 ␞ state ␞ position ms ␞ shuffle ␞ repeat ␞
//! volume [␞ id ␞ duration ms ␞ artwork url ␞ artist ␞ album ␞ name]`,
//! fields split by U+001E (record separator), numbers as integers (no
//! locale decimal separators). The track's free text comes last, name
//! last of all, so a stray separator inside a name only ever lands in the
//! name. Not running is `lavatui1 ␞ not running`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::runner::{RunError, Runner};
use super::worker::Backend;
use super::{Command, Snapshot, Status, Track, Unavailable};

const BUNDLE_ID: &str = "com.spotify.client";
const HEADER: &str = "lavatui1";
const SEP: char = '\u{1e}';
const NOT_RUNNING: &str = "not running";
/// The whole `osascript` run, start-up included.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Apple event timeout inside the script, below [`TIMEOUT`] so a busy
/// Spotify reports -1712 rather than being killed.
const EVENT_TIMEOUT_SECS: u32 = 4;

/// The Spotify backend, over a [`Runner`] (`osascript` in the app).
pub struct Spotify<R> {
    runner: R,
    name: Arc<str>,
}

impl<R: Runner> Spotify<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            name: Arc::from("Spotify"),
        }
    }
}

impl<R: Runner> Backend for Spotify<R> {
    fn exchange(&mut self, commands: &[Command]) -> Snapshot {
        let result = self.runner.run(&script(commands), TIMEOUT);
        let now = Instant::now();
        let mut snapshot = match result {
            Ok(out) => parse(&out, now),
            Err(err) => Snapshot::new(Status::Unavailable(classify(&err)), now),
        };
        snapshot.player = Some(Arc::clone(&self.name));
        snapshot
    }
}

/// The full script: guard, then (if running) commands + state read.
pub fn script(commands: &[Command]) -> String {
    let inner = inner_script(commands);
    format!(
        "if application id \"{BUNDLE_ID}\" is running then\n\
         \treturn run script \"{}\"\n\
         end if\n\
         return \"{HEADER}\" & (character id 30) & \"{NOT_RUNNING}\"\n",
        escape(&inner)
    )
}

/// The Spotify part, compiled only once Spotify is known to be running.
fn inner_script(commands: &[Command]) -> String {
    let mut s = format!(
        "tell application id \"{BUNDLE_ID}\"\n\
         with timeout of {EVENT_TIMEOUT_SECS} seconds\n"
    );
    for command in commands {
        // A command that fails (a bad URI, say) mustn't lose the state read;
        // a real problem (permission, quit) fails the read too.
        s += &format!("try\n{}\nend try\n", command_line(command));
    }
    s += "set rs to character id 30\n\
          set out to \"lavatui1\" & rs & (player state as text)\n\
          set pos to 0\n\
          try\n\
          set pos to (player position * 1000) as integer\n\
          end try\n\
          set out to out & rs & pos & rs & (shuffling as integer) & rs & (repeating as integer) & rs & (sound volume as integer)\n\
          try\n\
          set t to current track\n";
    for (var, expr) in [
        ("tid", "id of t"),
        ("dur", "(duration of t) as integer"),
        ("art", "artwork url of t"),
        ("ar", "artist of t"),
        ("al", "album of t"),
        ("nm", "name of t"),
    ] {
        s += &format!(
            "set {var} to \"\"\n\
             try\n\
             set {var} to {expr}\n\
             if {var} is missing value then set {var} to \"\"\n\
             end try\n"
        );
    }
    s += "if tid is not \"\" then set out to out & rs & tid & rs & dur & rs & art & rs & ar & rs & al & rs & nm\n\
          end try\n\
          return out\n\
          end timeout\n\
          end tell\n";
    s
}

fn command_line(command: &Command) -> String {
    match command {
        Command::PlayPause => "playpause".into(),
        Command::Next => "next track".into(),
        Command::Previous => "previous track".into(),
        // Seconds with a '.' decimal point: AppleScript source is
        // locale-independent.
        Command::Seek(to) => format!("set player position to {:.3}", to.as_secs_f64()),
        Command::SetShuffle(on) => format!("set shuffling to {on}"),
        Command::SetRepeat(on) => format!("set repeating to {on}"),
        // Spotify stores one less than it's given (set 68, read 67), so
        // ask for one more; 0 stays 0 and 100 can't go higher.
        Command::SetVolume(volume) => {
            let ask = match (*volume).min(100) {
                0 => 0,
                volume => (volume + 1).min(100),
            };
            format!("set sound volume to {ask}")
        }
        // Validated by `Command::play_uri`; escaped anyway.
        Command::PlayUri(uri) => format!("play track \"{}\"", escape(uri)),
    }
}

/// Escape text for an AppleScript string literal.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Turn the script's output into a snapshot sampled at `now`.
pub fn parse(out: &str, now: Instant) -> Snapshot {
    let unexpected = || {
        let shown: String = out.trim().chars().take(60).collect();
        Snapshot::new(
            Status::Unavailable(Unavailable::Error(format!(
                "unexpected answer from Spotify: {shown:?}"
            ))),
            now,
        )
    };
    let out = out.strip_suffix('\n').unwrap_or(out);
    let fields: Vec<&str> = out.split(SEP).collect();
    if fields.first() != Some(&HEADER) {
        return unexpected();
    }
    let status = match fields.get(1) {
        Some(&NOT_RUNNING) if fields.len() == 2 => {
            return Snapshot::new(Status::Unavailable(Unavailable::NotRunning), now);
        }
        Some(&"playing") => Status::Playing,
        Some(&"paused") => Status::Paused,
        Some(&"stopped") => Status::Stopped,
        _ => return unexpected(),
    };
    let int = |i: usize| fields.get(i).and_then(|f| f.trim().parse::<i64>().ok());
    let (Some(position), Some(shuffle), Some(repeat), Some(volume)) =
        (int(2), int(3), int(4), int(5))
    else {
        return unexpected();
    };
    let track = (fields.len() >= 12).then(|| {
        Arc::new(Track {
            id: fields[6].to_owned(),
            duration: millis(fields[7].trim().parse().unwrap_or(0)),
            artwork_url: fields[8].to_owned(),
            artist: fields[9].to_owned(),
            album: fields[10].to_owned(),
            // Anything past the album is the name (it held a separator).
            name: fields[11..].join(" "),
        })
    });
    Snapshot {
        player: None,
        status,
        track,
        position: millis(position),
        sampled_at: now,
        shuffle: shuffle != 0,
        repeat: repeat != 0,
        volume: volume.clamp(0, 100) as u8,
    }
}

fn millis(ms: i64) -> Duration {
    Duration::from_millis(ms.max(0) as u64)
}

/// Map a failed run to a reason the UI can explain.
pub fn classify(err: &RunError) -> Unavailable {
    match err {
        RunError::Timeout => Unavailable::NotResponding,
        RunError::Missing => Unavailable::Unsupported,
        RunError::Io(message) => Unavailable::Error(message.clone()),
        RunError::Failed(stderr) => match error_number(stderr) {
            // errAEEventNotPermitted: Automation permission refused.
            Some(-1743) => Unavailable::PermissionDenied,
            // The guard can't even compile: no app with that bundle id.
            Some(-1728) if stderr.contains(BUNDLE_ID) => Unavailable::NotInstalled,
            // procNotFound / connectionInvalid: quit while we were asking.
            Some(-600 | -609) => Unavailable::NotRunning,
            // errAETimeout.
            Some(-1712) => Unavailable::NotResponding,
            _ => Unavailable::Error(format!("Spotify: {}", error_text(stderr))),
        },
    }
}

/// The `(-1743)` at the end of an osascript error.
fn error_number(stderr: &str) -> Option<i32> {
    let open = stderr.trim_end().strip_suffix(')')?.rfind('(')?;
    stderr.trim_end()[open + 1..stderr.trim_end().len() - 1]
        .parse()
        .ok()
}

/// The message without osascript's `12:34: execution error: ` prefix.
fn error_text(stderr: &str) -> &str {
    let text = stderr.trim();
    text.find("error: ")
        .map_or(text, |at| &text[at + "error: ".len()..])
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::thread;

    use super::super::MediaSource;
    use super::super::worker::{Polled, testing};
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// Recorded from Spotify 1.2 (macOS 26), playing.
    const PLAYING: &str = "lavatui1\u{1e}playing\u{1e}240497\u{1e}0\u{1e}1\u{1e}100\u{1e}\
        spotify:track:0DZXVpUtPUom1VO6h5a0SU\u{1e}303440\u{1e}\
        https://i.scdn.co/image/ab67616d0000b273cb5ed04a1191ccec6717e76d\u{1e}\
        Dreamcatcher\u{1e}Dreamcatcher\u{1e}Life\n";

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn parses_a_playing_record() {
        let t = now();
        let snap = parse(PLAYING, t);
        assert_eq!(snap.status, Status::Playing);
        assert_eq!(snap.position, Duration::from_millis(240_497));
        assert_eq!(snap.sampled_at, t);
        assert!(!snap.shuffle && snap.repeat);
        assert_eq!(snap.volume, 100);
        let track = snap.track.unwrap();
        assert_eq!(track.id, "spotify:track:0DZXVpUtPUom1VO6h5a0SU");
        assert_eq!(track.name, "Life");
        assert_eq!(track.artist, "Dreamcatcher");
        assert_eq!(track.duration, Duration::from_millis(303_440));
        assert!(track.artwork_url.starts_with("https://i.scdn.co/"));
    }

    #[test]
    fn unicode_commas_quotes_and_stray_separators_survive() {
        let out = "lavatui1\u{1e}paused\u{1e}0\u{1e}1\u{1e}0\u{1e}37\u{1e}spotify:local:x\u{1e}0\u{1e}\
                   \u{1e}Sigur Rós, \"Jónsi\"\u{1e}( ), |\t\u{1e}Hoppípolla\u{1e}2\n";
        let snap = parse(out, now());
        assert_eq!(snap.status, Status::Paused);
        assert!(snap.shuffle && !snap.repeat);
        let track = snap.track.unwrap();
        assert_eq!(track.artist, "Sigur Rós, \"Jónsi\"");
        assert_eq!(track.album, "( ), |\t");
        assert_eq!(track.name, "Hoppípolla 2");
        assert_eq!(track.artwork_url, "");
        assert_eq!(track.duration, Duration::ZERO);
    }

    #[test]
    fn a_name_with_a_trailing_newline_keeps_it() {
        let out = PLAYING.replace("Life\n", "Life\n\n");
        assert_eq!(parse(&out, now()).track.unwrap().name, "Life\n");
    }

    #[test]
    fn stopped_without_a_track() {
        let snap = parse(
            "lavatui1\u{1e}stopped\u{1e}0\u{1e}0\u{1e}0\u{1e}64\n",
            now(),
        );
        assert_eq!(snap.status, Status::Stopped);
        assert_eq!(snap.track, None);
        assert_eq!(snap.volume, 64);
    }

    #[test]
    fn not_running() {
        let snap = parse("lavatui1\u{1e}not running\n", now());
        assert_eq!(snap.status, Status::Unavailable(Unavailable::NotRunning));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for out in [
            "",
            "\n",
            "playing, Life, Dreamcatcher",
            "lavatui1",
            "lavatui1\u{1e}dancing\u{1e}0\u{1e}0\u{1e}0\u{1e}0",
            "lavatui1\u{1e}playing\u{1e}x\u{1e}0\u{1e}0\u{1e}0",
            "lavatui1\u{1e}playing\u{1e}0\u{1e}0",
        ] {
            let snap = parse(out, now());
            assert!(
                matches!(snap.status, Status::Unavailable(Unavailable::Error(_))),
                "{out:?}"
            );
        }
        // A short track record is ignored rather than misread.
        let snap = parse(
            "lavatui1\u{1e}playing\u{1e}5\u{1e}0\u{1e}0\u{1e}300\u{1e}id",
            now(),
        );
        assert_eq!(snap.track, None);
        assert_eq!(snap.volume, 100);
    }

    #[test]
    fn classifies_recorded_osascript_errors() {
        let cases = [
            (
                "79:113: execution error: Not authorized to send Apple events to Spotify. (-1743)",
                Unavailable::PermissionDenied,
            ),
            (
                "17:52: syntax error: Can’t get application id \"com.spotify.client\". (-1728)",
                Unavailable::NotInstalled,
            ),
            (
                "execution error: Spotify got an error: Application isn’t running. (-600)",
                Unavailable::NotRunning,
            ),
            (
                "execution error: Spotify got an error: AppleEvent timed out. (-1712)",
                Unavailable::NotResponding,
            ),
            (
                "12:20: execution error: Spotify got an error: Can’t get current track. (-1728)",
                Unavailable::Error(
                    "Spotify: Spotify got an error: Can’t get current track. (-1728)".into(),
                ),
            ),
            ("weird", Unavailable::Error("Spotify: weird".into())),
        ];
        for (stderr, want) in cases {
            assert_eq!(classify(&RunError::Failed(stderr.into())), want, "{stderr}");
        }
        assert_eq!(classify(&RunError::Timeout), Unavailable::NotResponding);
        assert_eq!(classify(&RunError::Missing), Unavailable::Unsupported);
    }

    #[test]
    fn script_guards_before_touching_spotify() {
        let s = script(&[]);
        let guard = s.find("is running").unwrap();
        let tell = s.find("run script").unwrap();
        assert!(guard < tell);
        // The only unescaped `tell` is inside the run-script string.
        assert!(!s.contains("\ntell application"));
        assert!(s.contains("tell application id \\\"com.spotify.client\\\""));
    }

    #[test]
    fn commands_go_in_order_each_in_a_try() {
        let s = inner_script(&[
            Command::PlayPause,
            Command::Next,
            Command::Previous,
            Command::Seek(Duration::from_millis(61_250)),
            Command::SetShuffle(true),
            Command::SetRepeat(false),
            Command::SetVolume(42),
            Command::PlayUri("spotify:album:abc".into()),
        ]);
        let lines = [
            "try\nplaypause\nend try",
            "try\nnext track\nend try",
            "try\nprevious track\nend try",
            "try\nset player position to 61.250\nend try",
            "try\nset shuffling to true\nend try",
            "try\nset repeating to false\nend try",
            "try\nset sound volume to 43\nend try",
            "try\nplay track \"spotify:album:abc\"\nend try",
        ];
        let mut at = 0;
        for line in lines {
            let found = s[at..]
                .find(line)
                .unwrap_or_else(|| panic!("{line} in\n{s}"));
            at += found + line.len();
        }
        assert!(s[at..].contains("player state"), "state read comes after");
    }

    #[test]
    fn escaping_keeps_quotes_inside_the_literal() {
        assert_eq!(escape(r#"a"b\c"#), r#"a\"b\\c"#);
        let s = script(&[Command::PlayUri("spotify:x".into())]);
        assert!(s.contains(r#"play track \"spotify:x\""#));
    }

    /// A runner that answers from a list and records the scripts it ran.
    struct Canned(VecDeque<Result<String, RunError>>, Vec<String>);

    impl Runner for Canned {
        fn run(&mut self, script: &str, _: Duration) -> Result<String, RunError> {
            self.1.push(script.to_owned());
            self.0.pop_front().unwrap_or(Err(RunError::Timeout))
        }
    }

    #[test]
    fn exchange_maps_output_and_errors() {
        let mut spotify = Spotify::new(Canned(
            [
                Ok(PLAYING.to_owned()),
                Err(RunError::Failed(
                    "execution error: Not authorized (-1743)".into(),
                )),
                Err(RunError::Timeout),
            ]
            .into(),
            Vec::new(),
        ));
        assert_eq!(spotify.exchange(&[]).status, Status::Playing);
        assert_eq!(
            spotify.exchange(&[Command::Next]).status,
            Status::Unavailable(Unavailable::PermissionDenied)
        );
        assert_eq!(
            spotify.exchange(&[]).status,
            Status::Unavailable(Unavailable::NotResponding)
        );
        assert!(spotify.runner.1[1].contains("next track"));
    }

    // The worker on its thread, through this backend and a fake osascript.

    /// A pretend Spotify behind a fake osascript: `next track` changes the
    /// track; `fail` makes the next runs fail.
    #[derive(Default)]
    struct FakeSpotify {
        track: u32,
        fail: Vec<RunError>,
        scripts: Vec<String>,
    }

    #[derive(Clone, Default)]
    struct FakeRunner(Arc<Mutex<FakeSpotify>>);

    impl Runner for FakeRunner {
        fn run(&mut self, script: &str, _: Duration) -> Result<String, RunError> {
            let mut fake = self.0.lock().unwrap();
            fake.scripts.push(script.to_owned());
            if !fake.fail.is_empty() {
                return Err(fake.fail.remove(0));
            }
            if script.contains("next track") {
                fake.track += 1;
            }
            Ok(format!(
                "lavatui1\u{1e}playing\u{1e}1000\u{1e}0\u{1e}0\u{1e}80\u{1e}spotify:track:{n}\
                 \u{1e}200000\u{1e}\u{1e}Artist\u{1e}Album\u{1e}Song {n}\n",
                n = fake.track
            ))
        }
    }

    fn track_id(snap: &Snapshot) -> &str {
        snap.track.as_ref().map_or("", |t| t.id.as_str())
    }

    #[test]
    fn thread_command_then_refresh() {
        let runner = FakeRunner::default();
        let source = Polled::spawn(Spotify::new(runner.clone()), testing::fast());
        testing::wait_for(&source, "first poll", |s| s.status == Status::Playing);
        source.next();
        let snap = testing::wait_for(&source, "next track", |s| track_id(s) == "spotify:track:1");
        assert_eq!(snap.track.unwrap().name, "Song 1");
        let fake = runner.0.lock().unwrap();
        assert_eq!(
            fake.scripts
                .iter()
                .filter(|s| s.contains("next track"))
                .count(),
            1
        );
    }

    #[test]
    fn thread_timeouts_and_errors_recover() {
        let runner = FakeRunner::default();
        runner.0.lock().unwrap().fail = vec![
            RunError::Timeout,
            RunError::Failed("execution error: Not authorized (-1743)".into()),
        ];
        let source = Polled::spawn(Spotify::new(runner.clone()), testing::fast());
        testing::wait_for(&source, "permission denied", |s| {
            s.status == Status::Unavailable(Unavailable::PermissionDenied)
        });
        testing::wait_for(&source, "recovery", |s| s.status == Status::Playing);
        let fake = runner.0.lock().unwrap();
        assert!(fake.scripts.len() >= 3);
    }

    #[test]
    fn thread_stops_when_the_handle_drops() {
        let runner = FakeRunner::default();
        let source = Polled::spawn(Spotify::new(runner.clone()), testing::fast());
        testing::wait_for(&source, "first poll", |s| s.status == Status::Playing);
        drop(source);
        thread::sleep(MS * 60);
        let runs = runner.0.lock().unwrap().scripts.len();
        thread::sleep(MS * 100);
        assert_eq!(runner.0.lock().unwrap().scripts.len(), runs);
    }

    /// Against a real osascript (not Spotify): the generated script must
    /// compile. Uses a bundle id that doesn't exist, so it can't launch
    /// anything, and checks that maps to `NotInstalled`.
    #[test]
    fn script_compiles_and_a_missing_app_is_not_installed() {
        use super::super::runner::Osascript;
        let s = script(&[Command::Next, Command::SetVolume(3)])
            .replace(BUNDLE_ID, "com.lavatui.nonexistent");
        let err = Osascript.run(&s, TIMEOUT).unwrap_err();
        let RunError::Failed(stderr) = &err else {
            panic!("{err:?}")
        };
        assert_eq!(error_number(stderr), Some(-1728), "{stderr}");
        assert!(stderr.contains("com.lavatui.nonexistent"));
    }

    /// Against the real Spotify app (must be running, with a track loaded):
    /// every control once, timed, then the original state restored.
    /// `cargo test --release -- --ignored --nocapture live_spotify`
    #[test]
    #[ignore = "drives the real Spotify app"]
    fn live_spotify() {
        use super::super::MediaSource;
        use super::super::runner::Osascript;
        use super::super::worker::{Cadence, Polled};
        use std::thread::sleep;

        const SETTLE: Duration = Duration::from_millis(400);

        fn exchange(label: &str, commands: &[Command]) -> (Snapshot, f64) {
            let start = Instant::now();
            let snap = Spotify::new(Osascript).exchange(commands);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            let name = snap.track.as_ref().map_or("-", |t| t.name.as_str());
            println!(
                "{label:<16} {ms:>5.0} ms  {:?} {name:?} pos {:.1}s shuffle {} repeat {} vol {}",
                snap.status,
                snap.position.as_secs_f64(),
                snap.shuffle,
                snap.repeat,
                snap.volume
            );
            (snap, ms)
        }
        /// Send, give Spotify time to apply it, read.
        fn settled(label: &str, commands: &[Command]) -> Snapshot {
            exchange(label, commands);
            sleep(SETTLE);
            exchange("  settled", &[]).0
        }

        /// Puts back play state, volume, shuffle and repeat even if an
        /// assertion fails.
        struct Restore(Snapshot);
        impl Drop for Restore {
            fn drop(&mut self) {
                let now = exchange("restore check", &[]).0;
                let mut fix = Vec::new();
                if now.status != self.0.status {
                    fix.push(Command::PlayPause);
                }
                fix.push(Command::SetVolume(self.0.volume));
                fix.push(Command::SetShuffle(self.0.shuffle));
                fix.push(Command::SetRepeat(self.0.repeat));
                settled("restore", &fix);
            }
        }

        let mut polls: Vec<f64> = (0..7).map(|_| exchange("poll", &[]).1).collect();
        polls.sort_by(f64::total_cmp);
        println!(
            "poll latency: min {:.0} ms, median {:.0} ms, max {:.0} ms",
            polls[0], polls[3], polls[6]
        );
        let (orig, _) = exchange("original", &[]);
        assert!(orig.status.is_available(), "{:?}", orig.status);
        let orig_track = orig.track.clone().expect("a track loaded");
        let _restore = Restore(orig.clone());

        let s = settled("play/pause", &[Command::PlayPause]);
        assert_ne!(s.status, orig.status);
        let s = settled("play/pause", &[Command::PlayPause]);
        assert_eq!(s.status, orig.status);

        let s = settled("next", &[Command::Next]);
        assert_ne!(s.track.as_ref().map(|t| &t.id), Some(&orig_track.id));
        // Just skipped, so under 3 s in: previous goes back a track.
        let s = settled("previous", &[Command::Previous]);
        assert_eq!(s.track.as_ref().map(|t| &t.id), Some(&orig_track.id));

        let s = settled("shuffle", &[Command::SetShuffle(!orig.shuffle)]);
        println!("shuffle setter took: {}", s.shuffle != orig.shuffle);
        let s = settled("repeat", &[Command::SetRepeat(!orig.repeat)]);
        println!("repeat setter took: {}", s.repeat != orig.repeat);

        let quieter = orig.volume.saturating_sub(10).max(1);
        let s = settled("volume", &[Command::SetVolume(quieter)]);
        assert_eq!(s.volume, quieter);
        let s = settled("volume back", &[Command::SetVolume(orig.volume)]);
        assert_eq!(s.volume, orig.volume);

        // Back to where it was, plus the time this took (unless the track
        // would have ended by now: then let the next one play).
        let resume = orig.position + orig.sampled_at.elapsed();
        if resume + Duration::from_secs(5) < orig_track.duration {
            let s = settled("seek", &[Command::Seek(resume)]);
            let expected = resume + Duration::from_millis(800);
            assert!(s.position.abs_diff(expected) < Duration::from_secs(1));
        }

        // Through the worker: optimistic at once, confirmed after.
        let source = Polled::spawn(Spotify::new(Osascript), Cadence::default());
        let start = Instant::now();
        while !source.snapshot().status.is_available() {
            assert!(start.elapsed() < Duration::from_secs(5));
            sleep(Duration::from_millis(1));
        }
        println!("worker: first snapshot after {:?}", start.elapsed());
        for _ in 0..2 {
            let send = Instant::now();
            source.play_pause();
            let optimistic = source.snapshot().status;
            println!(
                "worker play/pause: {optimistic:?} shown after {:?}",
                send.elapsed()
            );
            let mut flickered = false;
            while send.elapsed() < Duration::from_millis(1500) {
                flickered |= source.snapshot().status != optimistic;
                sleep(Duration::from_millis(1));
            }
            assert!(!flickered, "the snapshot went back and forth");
            assert_eq!(source.snapshot().status, optimistic, "confirmed by Spotify");
        }
    }
}
