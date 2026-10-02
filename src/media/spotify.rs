//! The Spotify desktop app on macOS, driven through AppleScript (the
//! module only exists on macOS).
//!
//! One `osascript` process stays up for the backend's life
//! ([`Osascript`](super::runner::Osascript)) running [`script`]: a loop
//! that reads a request line, runs its commands, reads the state and
//! writes it back as one record. Starting osascript is what costs (~80 ms
//! of CPU); a poll on the running process is two Apple events and ~3 ms.
//!
//! Never launching Spotify: compiling a `tell application` block launches
//! the app to load its dictionary, and sending it an event launches it
//! too. So each request first checks `application id … is running` (the
//! id in a variable, resolved at run time) and only then compiles the
//! Spotify part (`run script`, once, kept until Spotify is seen gone) and
//! calls it.
//!
//! A request is `lavatui1 ␞ known track id ␞ command…`, a command being a
//! verb and its arguments (`seek 61250`, `uri spotify:album:…`,
//! `context spotify:track:… spotify:playlist:…`).
//!
//! The reply is `lavatui1 ␞ state ␞ position ms ␞ shuffle ␞ repeat ␞
//! volume [␞ id [␞ duration ms ␞ artwork url ␞ artist ␞ album ␞ name]]`,
//! fields split by U+001E (record separator), numbers as integers (no
//! locale decimal separators). The track's details come only when its id
//! isn't the known one (they cost five more Apple events), its free text
//! last, name last of all, so a stray separator inside a name only ever
//! lands in the name. Not running is `lavatui1 ␞ not running`; an
//! AppleScript error is `lavatui1 ␞ error ␞ number ␞ message`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::runner::{RunError, Runner};
use super::worker::Backend;
use super::{Capabilities, Command, Snapshot, Status, Track, Unavailable};

const BUNDLE_ID: &str = "com.spotify.client";
const HEADER: &str = "lavatui1";
const SEP: char = '\u{1e}';
const NOT_RUNNING: &str = "not running";
const ERROR: &str = "error";
/// One request, from writing it to the end of the reply (the first one
/// includes starting osascript).
const TIMEOUT: Duration = Duration::from_secs(5);
/// Apple event timeout inside the script, below [`TIMEOUT`] so a busy
/// Spotify reports -1712 rather than being killed.
const EVENT_TIMEOUT_SECS: u32 = 4;

/// The Spotify backend, over a [`Runner`] (an [`Osascript`] running
/// [`script`] in the app).
///
/// [`Osascript`]: super::runner::Osascript
pub struct Spotify<R> {
    runner: R,
    name: Arc<str>,
    /// The last track read in full: its details aren't asked for again
    /// while it plays.
    known: Option<Arc<Track>>,
}

impl<R: Runner> Spotify<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            name: Arc::from("Spotify"),
            known: None,
        }
    }
}

impl<R: Runner> Backend for Spotify<R> {
    fn exchange(&mut self, commands: &[Command]) -> Snapshot {
        let known = self.known.as_ref().map_or("", |track| track.id.as_str());
        let result = self.runner.run(&request(commands, known), TIMEOUT);
        let now = Instant::now();
        let mut snapshot = match result {
            Ok(out) => parse(&out, now, self.known.as_ref()),
            Err(err) => Snapshot::new(Status::Unavailable(classify(&err)), now),
        };
        if snapshot.track.is_some() {
            self.known.clone_from(&snapshot.track);
        }
        snapshot.player = Some(Arc::clone(&self.name));
        snapshot
    }

    /// Spotify 1.2's `set shuffling` / `set repeating` are no-ops (they
    /// read back unchanged, lava-75z.9), so the UI doesn't offer them.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            volume: true,
            ..Capabilities::NONE
        }
    }
}

/// The long-lived script: a request loop around the guard and the
/// Spotify part (see the module docs).
pub fn script() -> String {
    format!(
        "use framework \"Foundation\"\n\
         use scripting additions\n\
         property spot : missing value\n\
         on reply(t)\n\
         set s to current application's NSString's stringWithString:t\n\
         set s to s's stringByReplacingOccurrencesOfString:(character id 4) withString:\"\"\n\
         set s to s's stringByAppendingString:((character id 4) & linefeed)\n\
         (current application's NSFileHandle's fileHandleWithStandardOutput())'s writeData:(s's dataUsingEncoding:4)\n\
         end reply\n\
         on answer(req)\n\
         set rs to character id 30\n\
         set AppleScript's text item delimiters to rs\n\
         set parts to text items of req\n\
         set AppleScript's text item delimiters to \"\"\n\
         set bid to \"{BUNDLE_ID}\"\n\
         try\n\
         if application id bid is running then\n\
         if spot is missing value then set spot to run script \"{}\"\n\
         return spot's poll(parts)\n\
         end if\n\
         set spot to missing value\n\
         return \"{HEADER}\" & rs & \"{NOT_RUNNING}\"\n\
         on error m number n\n\
         set spot to missing value\n\
         return \"{HEADER}\" & rs & \"{ERROR}\" & rs & n & rs & m\n\
         end try\n\
         end answer\n\
         set stdin to current application's NSFileHandle's fileHandleWithStandardInput()\n\
         set buf to current application's NSMutableData's |data|()\n\
         repeat\n\
         set d to stdin's availableData()\n\
         if (d's |length|()) as integer is 0 then exit repeat\n\
         buf's appendData:d\n\
         set s to current application's NSString's alloc()'s initWithData:buf encoding:4\n\
         if s is not missing value then\n\
         set req to s as text\n\
         if req ends with linefeed then\n\
         my reply(answer(text 1 thru -2 of req))\n\
         set buf to current application's NSMutableData's |data|()\n\
         end if\n\
         end if\n\
         end repeat\n",
        escape(&spotify_part())
    )
}

/// The Spotify part, compiled only once Spotify is known to be running: a
/// script object whose `poll(parts)` runs the request's commands and reads
/// the state.
fn spotify_part() -> String {
    let mut s = format!(
        "script\n\
         on act(c)\n\
         set v to c\n\
         set a to \"\"\n\
         if c contains \" \" then\n\
         set o to offset of \" \" in c\n\
         set v to text 1 thru (o - 1) of c\n\
         set a to text (o + 1) thru -1 of c\n\
         end if\n\
         tell application id \"{BUNDLE_ID}\"\n\
         if v is \"playpause\" then\n\
         playpause\n\
         else if v is \"next\" then\n\
         next track\n\
         else if v is \"previous\" then\n\
         previous track\n\
         else if v is \"seek\" then\n\
         set player position to (a as integer) / 1000\n\
         else if v is \"shuffle\" then\n\
         set shuffling to (a is \"true\")\n\
         else if v is \"repeat\" then\n\
         set repeating to (a is \"true\")\n\
         else if v is \"volume\" then\n\
         set sound volume to (a as integer)\n\
         else if v is \"uri\" then\n\
         play track a\n\
         else if v is \"context\" then\n\
         set o to offset of \" \" in a\n\
         play track (text 1 thru (o - 1) of a) in context (text (o + 1) thru -1 of a)\n\
         end if\n\
         end tell\n\
         end act\n\
         on poll(parts)\n\
         set rs to character id 30\n\
         tell application id \"{BUNDLE_ID}\"\n\
         with timeout of {EVENT_TIMEOUT_SECS} seconds\n\
         repeat with i from 3 to count of parts\n\
         try\n\
         my act(item i of parts)\n\
         end try\n\
         end repeat\n\
         set p to properties\n\
         set out to \"{HEADER}\" & rs & ((player state of p) as text)\n\
         set pos to 0\n\
         try\n\
         set pos to ((player position of p) * 1000) as integer\n\
         end try\n\
         set out to out & rs & pos & rs & ((shuffling of p) as integer) & rs & ((repeating of p) as integer) & rs & ((sound volume of p) as integer)\n\
         try\n\
         set t to current track\n\
         set tid to id of t\n\
         if tid is missing value then set tid to \"\"\n\
         considering case\n\
         set same to tid is item 2 of parts\n\
         end considering\n\
         if same then\n\
         set out to out & rs & tid\n\
         else if tid is not \"\" then\n"
    );
    // One read per detail, each allowed to fail (ads and local files lack
    // some), in record order.
    for (var, expr) in [
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
    s += "set out to out & rs & tid & rs & dur & rs & art & rs & ar & rs & al & rs & nm\n\
          end if\n\
          end try\n\
          return out\n\
          end timeout\n\
          end tell\n\
          end poll\n\
          end script\n";
    s
}

/// One request line: the header, the known track id, the commands.
pub fn request(commands: &[Command], known_id: &str) -> String {
    // An id that would break the framing is just not known.
    let known_id = if known_id.contains([SEP, '\n', '\r']) {
        ""
    } else {
        known_id
    };
    let mut out = format!("{HEADER}{SEP}{known_id}");
    for command in commands {
        out.push(SEP);
        out += &command_word(command);
    }
    out
}

fn command_word(command: &Command) -> String {
    match command {
        Command::PlayPause => "playpause".into(),
        Command::Next => "next".into(),
        Command::Previous => "previous".into(),
        // Whole milliseconds: AppleScript's text-to-real coercion would
        // follow the locale's decimal separator.
        Command::Seek(to) => format!("seek {}", to.as_millis()),
        Command::SetShuffle(on) => format!("shuffle {on}"),
        Command::SetRepeat(on) => format!("repeat {on}"),
        // Spotify stores one less than it's given (set 68, read 67), so
        // ask for one more; 0 stays 0 and 100 can't go higher.
        Command::SetVolume(volume) => {
            let ask = match (*volume).min(100) {
                0 => 0,
                volume => (volume + 1).min(100),
            };
            format!("volume {ask}")
        }
        // Validated by `Command::play_uri`: no spaces, separators or
        // newlines.
        Command::PlayUri(uri) => format!("uri {uri}"),
        // Spotify carries on through the context, without Premium.
        Command::PlayInContext { track, context } => format!("context {track} {context}"),
    }
}

/// Escape text for an AppleScript string literal.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Turn a reply into a snapshot sampled at `now`. `known` is the track
/// whose id the request named: a reply with only that id means it.
pub fn parse(out: &str, now: Instant, known: Option<&Arc<Track>>) -> Snapshot {
    let unexpected = || {
        let shown: String = out.trim().chars().take(60).collect();
        Snapshot::new(
            Status::Unavailable(Unavailable::Error(format!(
                "unexpected answer from Spotify: {shown:?}"
            ))),
            now,
        )
    };
    let fields: Vec<&str> = out.split(SEP).collect();
    if fields.first() != Some(&HEADER) {
        return unexpected();
    }
    let status = match fields.get(1) {
        Some(&NOT_RUNNING) if fields.len() == 2 => {
            return Snapshot::new(Status::Unavailable(Unavailable::NotRunning), now);
        }
        Some(&ERROR) if fields.len() >= 4 => {
            // As osascript itself would print it: "message (number)".
            let message = format!("{} ({})", fields[3..].join(" "), fields[2].trim());
            return Snapshot::new(
                Status::Unavailable(classify(&RunError::Failed(message))),
                now,
            );
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
    let track = match fields.len() {
        7 => match known {
            Some(known) if known.id == fields[6] => Some(Arc::clone(known)),
            _ => return unexpected(),
        },
        12.. => Some(Arc::new(Track {
            id: fields[6].to_owned(),
            duration: millis(fields[7].trim().parse().unwrap_or(0)),
            artwork_url: fields[8].to_owned(),
            artist: fields[9].to_owned(),
            album: fields[10].to_owned(),
            // Anything past the album is the name (it held a separator).
            name: fields[11..].join(" "),
        })),
        // None loaded, or a short record: ignored rather than misread.
        _ => None,
    };
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

/// Map a failure (osascript's, or an error the script reported) to a
/// reason the UI can explain.
pub fn classify(err: &RunError) -> Unavailable {
    match err {
        RunError::Timeout => Unavailable::NotResponding,
        RunError::Missing => Unavailable::Unsupported,
        RunError::Io(message) => Unavailable::Error(message.clone()),
        RunError::Failed(stderr) => match error_number(stderr) {
            // errAEEventNotPermitted: Automation permission refused.
            Some(-1743) => Unavailable::PermissionDenied,
            // The guard can't find an app with that bundle id.
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
    use super::super::runner::Osascript;
    use super::super::worker::{Polled, testing};
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// Recorded from Spotify 1.2 (macOS 26), playing.
    const PLAYING: &str = "lavatui1\u{1e}playing\u{1e}240497\u{1e}0\u{1e}1\u{1e}100\u{1e}\
        spotify:track:0DZXVpUtPUom1VO6h5a0SU\u{1e}303440\u{1e}\
        https://i.scdn.co/image/ab67616d0000b273cb5ed04a1191ccec6717e76d\u{1e}\
        Dreamcatcher\u{1e}Dreamcatcher\u{1e}Life";

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn parses_a_playing_record() {
        let t = now();
        let snap = parse(PLAYING, t, None);
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
    fn the_known_track_is_reused_when_only_its_id_comes_back() {
        let full = parse(PLAYING, now(), None).track.unwrap();
        let short = "lavatui1\u{1e}paused\u{1e}241000\u{1e}0\u{1e}1\u{1e}100\u{1e}\
                     spotify:track:0DZXVpUtPUom1VO6h5a0SU";
        let snap = parse(short, now(), Some(&full));
        assert_eq!(snap.status, Status::Paused);
        assert!(Arc::ptr_eq(snap.track.as_ref().unwrap(), &full));
        // An id we didn't name is a misunderstanding, not a track.
        let other = short.replace("0DZX", "XXXX");
        let snap = parse(&other, now(), Some(&full));
        assert!(matches!(
            snap.status,
            Status::Unavailable(Unavailable::Error(_))
        ));
        let snap = parse(short, now(), None);
        assert!(matches!(
            snap.status,
            Status::Unavailable(Unavailable::Error(_))
        ));
    }

    #[test]
    fn unicode_commas_quotes_and_stray_separators_survive() {
        let out = "lavatui1\u{1e}paused\u{1e}0\u{1e}1\u{1e}0\u{1e}37\u{1e}spotify:local:x\u{1e}0\u{1e}\
                   \u{1e}Sigur Rós, \"Jónsi\"\u{1e}( ), |\t\u{1e}Hoppípolla\u{1e}2";
        let snap = parse(out, now(), None);
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
        let out = format!("{PLAYING}\n");
        assert_eq!(parse(&out, now(), None).track.unwrap().name, "Life\n");
    }

    #[test]
    fn stopped_without_a_track() {
        let snap = parse(
            "lavatui1\u{1e}stopped\u{1e}0\u{1e}0\u{1e}0\u{1e}64",
            now(),
            None,
        );
        assert_eq!(snap.status, Status::Stopped);
        assert_eq!(snap.track, None);
        assert_eq!(snap.volume, 64);
    }

    #[test]
    fn not_running() {
        let snap = parse("lavatui1\u{1e}not running", now(), None);
        assert_eq!(snap.status, Status::Unavailable(Unavailable::NotRunning));
    }

    #[test]
    fn reported_errors_are_classified() {
        let cases = [
            (
                "-1743\u{1e}Not authorized to send Apple events to Spotify.",
                Unavailable::PermissionDenied,
            ),
            (
                "-1728\u{1e}Can’t get application id \"com.spotify.client\".",
                Unavailable::NotInstalled,
            ),
            ("-609\u{1e}Connection is invalid.", Unavailable::NotRunning),
            (
                "-2753\u{1e}The variable x is not defined.",
                Unavailable::Error("Spotify: The variable x is not defined. (-2753)".into()),
            ),
        ];
        for (rest, want) in cases {
            let out = format!("lavatui1\u{1e}error\u{1e}{rest}");
            assert_eq!(
                parse(&out, now(), None).status,
                Status::Unavailable(want),
                "{rest}"
            );
        }
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for out in [
            "",
            "\n",
            "playing, Life, Dreamcatcher",
            "lavatui1",
            "lavatui1\u{1e}error",
            "lavatui1\u{1e}dancing\u{1e}0\u{1e}0\u{1e}0\u{1e}0",
            "lavatui1\u{1e}playing\u{1e}x\u{1e}0\u{1e}0\u{1e}0",
            "lavatui1\u{1e}playing\u{1e}0\u{1e}0",
        ] {
            let snap = parse(out, now(), None);
            assert!(
                matches!(snap.status, Status::Unavailable(Unavailable::Error(_))),
                "{out:?}"
            );
        }
        // A short track record is ignored rather than misread.
        let snap = parse(
            "lavatui1\u{1e}playing\u{1e}5\u{1e}0\u{1e}0\u{1e}300\u{1e}id\u{1e}1",
            now(),
            None,
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
        let s = script();
        let guard = s.find("is running").unwrap();
        let tell = s.find("run script").unwrap();
        assert!(guard < tell);
        // The guard's id is a variable, so a missing app is a run-time
        // error the loop reports, not a script that won't start.
        assert!(s.contains("application id bid is running"));
        // Every `tell` is inside the run-script string (its quotes escaped).
        let tells = s.matches("tell application").count();
        assert!(tells > 0);
        assert_eq!(
            s.matches("tell application id \\\"com.spotify.client\\\"")
                .count(),
            tells
        );
    }

    #[test]
    fn requests_carry_the_known_id_then_the_commands_in_order() {
        let r = request(
            &[
                Command::PlayPause,
                Command::Next,
                Command::Previous,
                Command::Seek(Duration::from_millis(61_250)),
                Command::SetShuffle(true),
                Command::SetRepeat(false),
                Command::SetVolume(42),
                Command::SetVolume(0),
                Command::SetVolume(100),
                Command::PlayUri("spotify:album:abc".into()),
                Command::play_in_context("spotify:track:t", "spotify:playlist:p").unwrap(),
            ],
            "spotify:track:a",
        );
        let fields: Vec<&str> = r.split(SEP).collect();
        assert_eq!(
            fields,
            [
                "lavatui1",
                "spotify:track:a",
                "playpause",
                "next",
                "previous",
                "seek 61250",
                "shuffle true",
                "repeat false",
                "volume 43",
                "volume 0",
                "volume 100",
                "uri spotify:album:abc",
                "context spotify:track:t spotify:playlist:p",
            ]
        );
        assert_eq!(request(&[], ""), "lavatui1\u{1e}");
        assert_eq!(request(&[], "bad\nid"), "lavatui1\u{1e}");
        assert!(!request(&[], "a\u{1e}b").contains('b'));
    }

    #[test]
    fn escaping_keeps_quotes_inside_the_literal() {
        assert_eq!(escape(r#"a"b\c"#), r#"a\"b\\c"#);
    }

    /// A runner that answers from a list and records the requests.
    struct Canned(VecDeque<Result<String, RunError>>, Vec<String>);

    impl Runner for Canned {
        fn run(&mut self, request: &str, _: Duration) -> Result<String, RunError> {
            self.1.push(request.to_owned());
            self.0.pop_front().unwrap_or(Err(RunError::Timeout))
        }
    }

    #[test]
    fn exchange_maps_output_and_errors() {
        let same = "lavatui1\u{1e}playing\u{1e}241000\u{1e}0\u{1e}1\u{1e}100\u{1e}\
                    spotify:track:0DZXVpUtPUom1VO6h5a0SU";
        let mut spotify = Spotify::new(Canned(
            [
                Ok(PLAYING.to_owned()),
                Ok(same.to_owned()),
                Err(RunError::Failed(
                    "execution error: Not authorized (-1743)".into(),
                )),
                Err(RunError::Timeout),
            ]
            .into(),
            Vec::new(),
        ));
        let first = spotify.exchange(&[]);
        assert_eq!(first.status, Status::Playing);
        let second = spotify.exchange(&[]);
        assert_eq!(second.track.unwrap().name, "Life");
        assert_eq!(
            spotify.exchange(&[Command::Next]).status,
            Status::Unavailable(Unavailable::PermissionDenied)
        );
        assert_eq!(
            spotify.exchange(&[]).status,
            Status::Unavailable(Unavailable::NotResponding)
        );
        let requests = &spotify.runner.1;
        assert_eq!(requests[0], "lavatui1\u{1e}");
        // From the first full read on, the track is known.
        assert_eq!(
            requests[1],
            "lavatui1\u{1e}spotify:track:0DZXVpUtPUom1VO6h5a0SU"
        );
        assert!(requests[2].ends_with("\u{1e}next"));
    }

    // The worker on its thread, through this backend and a fake osascript.

    /// A pretend Spotify behind a fake osascript: `next` changes the track;
    /// `fail` makes the next runs fail.
    #[derive(Default)]
    struct FakeSpotify {
        track: u32,
        fail: Vec<RunError>,
        requests: Vec<String>,
    }

    #[derive(Clone, Default)]
    struct FakeRunner(Arc<Mutex<FakeSpotify>>);

    impl Runner for FakeRunner {
        fn run(&mut self, request: &str, _: Duration) -> Result<String, RunError> {
            let mut fake = self.0.lock().unwrap();
            fake.requests.push(request.to_owned());
            if !fake.fail.is_empty() {
                return Err(fake.fail.remove(0));
            }
            fake.track += request.split(SEP).filter(|c| *c == "next").count() as u32;
            Ok(format!(
                "lavatui1\u{1e}playing\u{1e}1000\u{1e}0\u{1e}0\u{1e}80\u{1e}spotify:track:{n}\
                 \u{1e}200000\u{1e}\u{1e}Artist\u{1e}Album\u{1e}Song {n}",
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
            fake.requests
                .iter()
                .filter(|s| s.ends_with("\u{1e}next"))
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
        assert!(fake.requests.len() >= 3);
    }

    #[test]
    fn thread_stops_when_the_handle_drops() {
        let runner = FakeRunner::default();
        let source = Polled::spawn(Spotify::new(runner.clone()), testing::fast());
        testing::wait_for(&source, "first poll", |s| s.status == Status::Playing);
        drop(source);
        thread::sleep(MS * 60);
        let runs = runner.0.lock().unwrap().requests.len();
        thread::sleep(MS * 100);
        assert_eq!(runner.0.lock().unwrap().requests.len(), runs);
    }

    /// A real runner with all the time it needs: starting osascript and
    /// compiling the script can take longer than the app's [`TIMEOUT`] on
    /// a heavily loaded machine (lava-9b3), and that's not what's tested.
    struct Patient(Osascript);

    impl Runner for Patient {
        fn run(&mut self, request: &str, _: Duration) -> Result<String, RunError> {
            self.0.run(request, Duration::from_secs(120))
        }
    }

    /// Against a real osascript (not Spotify): the script compiles and
    /// keeps answering. Uses a bundle id that doesn't exist, so it can't
    /// launch anything, and checks that maps to `NotInstalled`.
    #[test]
    fn script_runs_and_a_missing_app_is_not_installed() {
        let s = script().replace(BUNDLE_ID, "com.lavatui.nonexistent");
        let mut spotify = Spotify::new(Patient(Osascript::new(s)));
        for commands in [&[][..], &[Command::Next, Command::SetVolume(3)], &[]] {
            let snap = spotify.exchange(commands);
            // The bundle id differs, so this is a plain error naming it.
            let Status::Unavailable(Unavailable::Error(message)) = &snap.status else {
                panic!("{:?}", snap.status)
            };
            assert!(message.contains("com.lavatui.nonexistent"), "{message}");
            assert!(message.ends_with("(-1728)"), "{message}");
        }
    }

    /// Against the real Spotify app (must be running, with a track loaded):
    /// poll cost, then every control once, timed, then the original state
    /// restored.
    /// `cargo test --release -- --ignored --nocapture live_spotify`
    #[test]
    #[ignore = "drives the real Spotify app"]
    fn live_spotify() {
        use super::super::worker::Cadence;
        use std::thread::sleep;

        const SETTLE: Duration = Duration::from_millis(400);

        type Live = Spotify<Osascript>;

        fn exchange(spotify: &mut Live, label: &str, commands: &[Command]) -> (Snapshot, f64) {
            let start = Instant::now();
            let snap = spotify.exchange(commands);
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
        fn settled(spotify: &mut Live, label: &str, commands: &[Command]) -> Snapshot {
            exchange(spotify, label, commands);
            sleep(SETTLE);
            exchange(spotify, "  settled", &[]).0
        }

        /// Puts back play state, volume, shuffle and repeat even if an
        /// assertion fails.
        struct Restore(Snapshot);
        impl Drop for Restore {
            fn drop(&mut self) {
                let spotify = &mut Spotify::new(Osascript::new(script()));
                let now = exchange(spotify, "restore check", &[]).0;
                let mut fix = Vec::new();
                if now.status != self.0.status {
                    fix.push(Command::PlayPause);
                }
                fix.push(Command::SetVolume(self.0.volume));
                settled(spotify, "restore", &fix);
            }
        }

        let spotify = &mut Spotify::new(Osascript::new(script()));
        exchange(spotify, "start", &[]);
        let mut polls: Vec<f64> = (0..9).map(|_| exchange(spotify, "poll", &[]).1).collect();
        polls.sort_by(f64::total_cmp);
        println!(
            "poll latency: min {:.0} ms, median {:.0} ms, max {:.0} ms",
            polls[0], polls[4], polls[8]
        );
        let (orig, _) = exchange(spotify, "original", &[]);
        assert!(orig.status.is_available(), "{:?}", orig.status);
        let orig_track = orig.track.clone().expect("a track loaded");
        let _restore = Restore(orig.clone());

        let s = settled(spotify, "play/pause", &[Command::PlayPause]);
        assert_ne!(s.status, orig.status);
        let s = settled(spotify, "play/pause", &[Command::PlayPause]);
        assert_eq!(s.status, orig.status);
        assert_eq!(s.track.as_ref().map(|t| &t.id), Some(&orig_track.id));

        let quieter = orig.volume.saturating_sub(10).max(1);
        let s = settled(spotify, "volume", &[Command::SetVolume(quieter)]);
        assert_eq!(s.volume, quieter);
        let s = settled(spotify, "volume back", &[Command::SetVolume(orig.volume)]);
        assert_eq!(s.volume, orig.volume);

        // Through the worker: optimistic at once, confirmed after.
        let source = Polled::spawn(Spotify::new(Osascript::new(script())), Cadence::default());
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
