//! What the macOS players driven through AppleScript share (Spotify,
//! Apple Music; the module only exists on macOS).
//!
//! One `osascript` process ([`Osascript`](super::runner::Osascript))
//! runs [`script`] for all of them: a loop that reads a request line,
//! asks the app it names and writes the answer back as one record.
//! Starting osascript is what costs (~80 ms of CPU); a poll on the running
//! process is a few Apple events and a few ms.
//!
//! Never launching a player: compiling a `tell application` block launches
//! the app to load its dictionary, and sending it an event launches it
//! too. So each request first checks `application id … is running` (the
//! id in a variable, resolved at run time) and only then compiles that
//! app's part (`run script`, once, kept until the app is seen gone) and
//! calls it. Each part is a script object whose `poll(fields)` runs the
//! request's commands and reads the state.
//!
//! A request line is `bundle id ␝ lavatui1 ␞ known track id ␞ …`: the
//! [`Routed`] runner puts the bundle id and U+001D (group separator) in
//! front, so each player's own requests and replies look the same as when
//! it had a process to itself. The reply is `lavatui1 ␞ state ␞ position
//! ms ␞ shuffle ␞ repeat ␞ volume [␞ id [␞ duration ms ␞ artwork ␞ artist
//! ␞ album ␞ name]]` ([`parse`]), fields split by U+001E (record
//! separator), numbers as integers (no locale decimal separators). The
//! track's details come only when its id isn't the known one, its free
//! text last, name last of all, so a stray separator inside a name only
//! ever lands in the name. Not running is `lavatui1 ␞ not running`; an
//! AppleScript error is `lavatui1 ␞ error ␞ number ␞ message`.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::runner::{RunError, Runner};
use super::{Snapshot, Status, Track, Unavailable};

pub const HEADER: &str = "lavatui1";
pub const SEP: char = '\u{1e}';
/// Between the bundle id and the player's request ([`Routed`]).
pub const ROUTE: char = '\u{1d}';
const NOT_RUNNING: &str = "not running";
const ERROR: &str = "error";
/// One request, from writing it to the end of the reply (the first one
/// includes starting osascript).
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// Apple event timeout inside the scripts, below [`TIMEOUT`] so a busy
/// player reports -1712 rather than being killed.
pub const EVENT_TIMEOUT_SECS: u32 = 4;

/// The long-lived script: a request loop around the guard and each app's
/// part, `apps` being (bundle id, part source).
pub fn script(apps: &[(&str, String)]) -> String {
    let mut sources = String::new();
    for (bundle, part) in apps {
        sources += &format!(
            "if bid is \"{}\" then return \"{}\"\n",
            escape(bundle),
            escape(part)
        );
    }
    format!(
        "use framework \"Foundation\"\n\
         use scripting additions\n\
         property bids : {{}}\n\
         property objs : {{}}\n\
         on reply(t)\n\
         set s to current application's NSString's stringWithString:t\n\
         set s to s's stringByReplacingOccurrencesOfString:(character id 4) withString:\"\"\n\
         set s to s's stringByAppendingString:((character id 4) & linefeed)\n\
         (current application's NSFileHandle's fileHandleWithStandardOutput())'s writeData:(s's dataUsingEncoding:4)\n\
         end reply\n\
         on source(bid)\n\
         {sources}\
         error \"no script for \" & bid number -2700\n\
         end source\n\
         on part(bid)\n\
         repeat with i from 1 to count of bids\n\
         if item i of bids is bid then return item i of objs\n\
         end repeat\n\
         set o to run script (my source(bid))\n\
         set end of bids to bid\n\
         set end of objs to o\n\
         return o\n\
         end part\n\
         on forget(bid)\n\
         set kb to {{}}\n\
         set ko to {{}}\n\
         repeat with i from 1 to count of bids\n\
         if item i of bids is not bid then\n\
         set end of kb to item i of bids\n\
         set end of ko to item i of objs\n\
         end if\n\
         end repeat\n\
         set bids to kb\n\
         set objs to ko\n\
         end forget\n\
         on answer(msg)\n\
         set rs to character id 30\n\
         set bid to \"\"\n\
         try\n\
         set o to offset of (character id 29) in msg\n\
         set bid to text 1 thru (o - 1) of msg\n\
         set req to text (o + 1) thru -1 of msg\n\
         set AppleScript's text item delimiters to rs\n\
         set fields to text items of req\n\
         set AppleScript's text item delimiters to \"\"\n\
         if application id bid is running then\n\
         set p to my part(bid)\n\
         return p's poll(fields)\n\
         end if\n\
         my forget(bid)\n\
         return \"{HEADER}\" & rs & \"{NOT_RUNNING}\"\n\
         on error m number n\n\
         set AppleScript's text item delimiters to \"\"\n\
         my forget(bid)\n\
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
         end repeat\n"
    )
}

/// The `act(c)` handler a part runs each command through: `c` split at
/// its first space into `v` (the verb) and `a` (the rest), then `body`
/// (inside a `tell` to the app).
pub fn act_handler(bundle: &str, body: &str) -> String {
    format!(
        "on act(c)\n\
         set v to c\n\
         set a to \"\"\n\
         if c contains \" \" then\n\
         set o to offset of \" \" in c\n\
         set v to text 1 thru (o - 1) of c\n\
         set a to text (o + 1) thru -1 of c\n\
         end if\n\
         tell application id \"{bundle}\"\n\
         {body}\
         end tell\n\
         end act\n"
    )
}

/// The AppleScript reading track detail `var` (`expr`, a read of the
/// track `t`), allowed to fail (ads, local files and streams lack some).
pub fn detail(var: &str, expr: &str) -> String {
    format!(
        "set {var} to \"\"\n\
         try\n\
         set {var} to {expr}\n\
         if {var} is missing value then set {var} to \"\"\n\
         end try\n"
    )
}

/// Escape text for an AppleScript string literal.
pub fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// One request line: the header, the known track id, then `rest` (extra
/// fields and commands).
pub fn request(known_id: &str, rest: impl IntoIterator<Item = String>) -> String {
    let mut out = format!("{HEADER}{SEP}{}", field(known_id));
    for item in rest {
        out.push(SEP);
        out += &item;
    }
    out
}

/// `text`, or nothing when it would break the framing.
pub fn field(text: &str) -> &str {
    if text.contains([SEP, ROUTE, '\n', '\r']) {
        ""
    } else {
        text
    }
}

/// A player's runner: its requests go to the shared process prefixed with
/// its bundle id.
pub struct Routed<R> {
    bundle: &'static str,
    inner: R,
}

impl<R> Routed<R> {
    pub fn new(bundle: &'static str, inner: R) -> Self {
        Self { bundle, inner }
    }
}

impl<R: Runner> Runner for Routed<R> {
    fn run(&mut self, request: &str, timeout: Duration) -> Result<String, RunError> {
        self.inner
            .run(&format!("{}{ROUTE}{request}", self.bundle), timeout)
    }
}

/// One process for several players (all on the media worker's thread, so
/// the lock is never contended).
impl<R: Runner> Runner for Arc<Mutex<R>> {
    fn run(&mut self, request: &str, timeout: Duration) -> Result<String, RunError> {
        self.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .run(request, timeout)
    }
}

/// The app a reply is from: bundle id and display name.
#[derive(Clone, Copy, Debug)]
pub struct App {
    pub bundle: &'static str,
    pub name: &'static str,
}

/// Turn `app`'s reply into a snapshot sampled at `now`. `known` is the
/// track whose id the request named: a reply with only that id means it.
pub fn parse(out: &str, now: Instant, known: Option<&Arc<Track>>, app: App) -> Snapshot {
    let name = app.name;
    let unexpected = || {
        let shown: String = out.trim().chars().take(60).collect();
        Snapshot::new(
            Status::Unavailable(Unavailable::Error(format!(
                "unexpected answer from {name}: {shown:?}"
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
            let reason = classify(&RunError::Failed(message), app);
            return Snapshot::new(Status::Unavailable(reason), now);
        }
        // Music also fast-forwards and rewinds: still playing.
        Some(&("playing" | "fast forwarding" | "rewinding")) => Status::Playing,
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
            uri: super::spotify_track_uri(fields[6]),
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

/// The last track read in full, and whether to read it in full again.
///
/// A track that came back with details missing (the player still loading
/// them as it changes track, a read that failed) is read in full again on
/// up to [`REREADS`] polls (about as many seconds) before it's taken as it
/// is (local files and ads lack some for good). Without this, a song's
/// cover and lyrics stayed missing until the next song (lava-75z.22).
#[derive(Debug, Default)]
pub struct Known {
    pub track: Option<Arc<Track>>,
    rereads: u8,
}

/// Full re-reads of a track whose details are incomplete.
pub const REREADS: u8 = 8;

impl Known {
    /// The id to name in the request: the known track's, or nothing when
    /// it's to be read in full again (`incomplete` says which details a
    /// track should have).
    pub fn named(&self, incomplete: impl Fn(&Track) -> bool) -> &str {
        match &self.track {
            Some(track) if !self.rereading(&incomplete) => &track.id,
            _ => "",
        }
    }

    fn rereading(&self, incomplete: impl Fn(&Track) -> bool) -> bool {
        self.rereads > 0 && self.track.as_deref().is_some_and(incomplete)
    }

    /// Take `snapshot`'s track as the known one. A re-read of the same
    /// track keeps what the earlier read had and this one lacks; a new
    /// track starts the re-read count again. Returns whether the track was
    /// read in full just now (not the known one named back).
    pub fn update(&mut self, snapshot: &mut Snapshot, incomplete: impl Fn(&Track) -> bool) -> bool {
        let reread = self.rereading(&incomplete);
        let Some(track) = &mut snapshot.track else {
            return false;
        };
        let fresh = !self.track.as_ref().is_some_and(|k| Arc::ptr_eq(k, track));
        match &self.track {
            Some(known) if known.id == track.id => {
                if reread {
                    self.rereads -= 1;
                    *track = fill_in(track, known);
                }
            }
            _ => self.rereads = REREADS,
        }
        self.track = Some(Arc::clone(track));
        fresh
    }

    /// Full re-reads left.
    #[cfg(test)]
    pub fn rereads(&self) -> u8 {
        self.rereads
    }
}

/// `fresh`, with any detail it lacks taken from `old` (the same track read
/// earlier), so a re-read never loses what an earlier one had.
fn fill_in(fresh: &Arc<Track>, old: &Track) -> Arc<Track> {
    let pick = |new: &str, old: &str| if new.trim().is_empty() { old } else { new }.to_owned();
    Arc::new(Track {
        id: fresh.id.clone(),
        uri: fresh.uri.clone(),
        name: pick(&fresh.name, &old.name),
        artist: pick(&fresh.artist, &old.artist),
        album: pick(&fresh.album, &old.album),
        duration: if fresh.duration.is_zero() {
            old.duration
        } else {
            fresh.duration
        },
        artwork_url: pick(&fresh.artwork_url, &old.artwork_url),
    })
}

/// Map a failure (osascript's, or an error the script reported) of `app`
/// to a reason the UI can explain.
pub fn classify(err: &RunError, app: App) -> Unavailable {
    let App { bundle, name } = app;
    match err {
        RunError::Timeout => Unavailable::NotResponding,
        RunError::Missing => Unavailable::Unsupported,
        RunError::Io(message) => Unavailable::Error(message.clone()),
        RunError::Failed(stderr) => match error_number(stderr) {
            // errAEEventNotPermitted: Automation permission refused.
            Some(-1743) => Unavailable::PermissionDenied,
            // The guard can't find an app with that bundle id.
            Some(-1728) if stderr.contains(bundle) => Unavailable::NotInstalled,
            // procNotFound / connectionInvalid: quit while we were asking.
            Some(-600 | -609) => Unavailable::NotRunning,
            // errAETimeout.
            Some(-1712) => Unavailable::NotResponding,
            _ => Unavailable::Error(format!("{name}: {}", error_text(stderr))),
        },
    }
}

/// The `(-1743)` at the end of an osascript error.
fn error_number(stderr: &str) -> Option<i32> {
    let text = stderr.trim_end();
    let open = text.strip_suffix(')')?.rfind('(')?;
    text[open + 1..text.len() - 1].parse().ok()
}

/// The message without osascript's `12:34: execution error: ` prefix.
fn error_text(stderr: &str) -> &str {
    let text = stderr.trim();
    text.find("error: ")
        .map_or(text, |at| &text[at + "error: ".len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routed_requests_name_the_app_first() {
        struct Echo;
        impl Runner for Echo {
            fn run(&mut self, request: &str, _: Duration) -> Result<String, RunError> {
                Ok(request.to_owned())
            }
        }
        let shared = Arc::new(Mutex::new(Echo));
        let mut a = Routed::new("com.example.a", Arc::clone(&shared));
        let mut b = Routed::new("com.example.b", shared);
        assert_eq!(
            a.run("lavatui1\u{1e}x", TIMEOUT).unwrap(),
            "com.example.a\u{1d}lavatui1\u{1e}x"
        );
        assert!(
            b.run("r", TIMEOUT)
                .unwrap()
                .starts_with("com.example.b\u{1d}")
        );
    }

    #[test]
    fn fields_that_would_break_the_framing_are_dropped() {
        assert_eq!(field("persistent-ID"), "persistent-ID");
        for bad in ["a\u{1e}b", "a\u{1d}b", "a\nb", "a\rb"] {
            assert_eq!(field(bad), "", "{bad:?}");
        }
        assert_eq!(request("a\nb", ["x".to_owned()]), "lavatui1\u{1e}\u{1e}x");
    }

    #[test]
    fn the_loop_has_a_part_per_app_and_nothing_compiled_outside_it() {
        let s = script(&[
            (
                "com.example.a",
                "script\ntell application id \"com.example.a\"\nend tell\nend script\n".into(),
            ),
            (
                "com.example.b",
                "script\ntell application id \"com.example.b\"\nend tell\nend script\n".into(),
            ),
        ]);
        let guard = s.find("application id bid is running").unwrap();
        assert!(guard < s.find("my part(bid)").unwrap());
        assert!(s.contains("if bid is \"com.example.a\" then return \"script"));
        // Every `tell` is inside a run-script string (its quotes escaped).
        let tells = s.matches("tell application").count();
        assert_eq!(tells, s.matches("tell application id \\\"").count());
        assert_eq!(tells, 2);
    }

    const X: App = App {
        bundle: "com.example.x",
        name: "X",
    };

    #[test]
    fn reported_errors_are_classified_for_their_app() {
        let cases = [
            (
                "-1743\u{1e}Not authorized to send Apple events to X.",
                Unavailable::PermissionDenied,
            ),
            (
                "-1728\u{1e}Can’t get application id \"com.example.x\".",
                Unavailable::NotInstalled,
            ),
            ("-609\u{1e}Connection is invalid.", Unavailable::NotRunning),
            (
                "-1712\u{1e}AppleEvent timed out.",
                Unavailable::NotResponding,
            ),
            (
                "-2753\u{1e}The variable x is not defined.",
                Unavailable::Error("X: The variable x is not defined. (-2753)".into()),
            ),
        ];
        for (rest, want) in cases {
            let out = format!("lavatui1\u{1e}error\u{1e}{rest}");
            let snap = parse(&out, Instant::now(), None, X);
            assert_eq!(snap.status, Status::Unavailable(want), "{rest}");
        }
    }

    #[test]
    fn music_states_map_onto_playing() {
        for (state, want) in [
            ("fast forwarding", Status::Playing),
            ("rewinding", Status::Playing),
            ("paused", Status::Paused),
        ] {
            let out = format!("lavatui1\u{1e}{state}\u{1e}0\u{1e}0\u{1e}0\u{1e}50");
            assert_eq!(parse(&out, Instant::now(), None, X).status, want);
        }
    }

    /// The real loop with both parts, in a real osascript: each player's
    /// requests reach the app they name. Bundle ids that don't exist, so
    /// nothing can launch; each says its own app is missing.
    #[test]
    fn the_shared_loop_routes_each_request_to_its_app() {
        use super::super::{apple_music, runner::Osascript, spotify};

        const A: App = App {
            bundle: "com.lavatui.nonexistent-a",
            name: "A",
        };
        const B: App = App {
            bundle: "com.lavatui.nonexistent-b",
            name: "B",
        };
        let mut shared = Arc::new(Mutex::new(Osascript::new(script(&[
            (A.bundle, spotify::spotify_part()),
            (B.bundle, apple_music::music_part()),
        ]))));
        // Starting osascript can be slow on a loaded machine (lava-9b3).
        let patient = Duration::from_secs(120);
        for app in [A, B, A] {
            let mut runner = Routed::new(app.bundle, Arc::clone(&shared));
            let out = runner
                .run(&request("", ["next".to_owned()]), patient)
                .unwrap();
            let snap = parse(&out, Instant::now(), None, app);
            assert_eq!(
                snap.status,
                Status::Unavailable(Unavailable::NotInstalled),
                "{}: {out:?}",
                app.name
            );
        }
        // A request naming no app is an error, not the end of the loop.
        let out = shared.run("lavatui1", patient).unwrap();
        assert!(out.starts_with("lavatui1\u{1e}error"), "{out:?}");
        let mut runner = Routed::new(A.bundle, shared);
        assert!(runner.run(&request("", []), patient).is_ok());
    }
}
