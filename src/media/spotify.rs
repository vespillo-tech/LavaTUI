//! The Spotify desktop app on macOS, driven through AppleScript (the
//! module only exists on macOS).
//!
//! [`spotify_part`] is the Spotify part of the shared AppleScript loop
//! ([`applescript`](super::applescript): one long-lived `osascript`, the
//! never-launch guard, the record format). A request is `lavatui1 ␞ known
//! track id ␞ command…`, a command being a verb and its arguments (`seek
//! 61250`, `uri spotify:album:…`, `context spotify:track:…
//! spotify:playlist:…`). The track's details come only when its id isn't
//! the known one (they cost five more Apple events). When the details are
//! read the position is read again after them, so it's always taken just
//! before the reply (the worker's `Baseline` puts it there).

use std::sync::Arc;
use std::time::Instant;

use super::applescript::{self, App, EVENT_TIMEOUT_SECS, HEADER, Known, TIMEOUT};
use super::runner::{RunError, Runner};
use super::worker::{Backend, Nudge};
use super::{Capabilities, Command, Snapshot, Status, Track, Unavailable};

pub const BUNDLE_ID: &str = "com.spotify.client";
pub const APP: App = App {
    bundle: BUNDLE_ID,
    name: "Spotify",
};

/// The Spotify backend, over a [`Runner`] (an [`Osascript`] running
/// [`script`], or the shared one through a
/// [`Routed`](super::applescript::Routed) runner).
///
/// [`Osascript`]: super::runner::Osascript
pub struct Spotify<R> {
    runner: R,
    name: Arc<str>,
    /// The last track read in full: its details aren't asked for again
    /// while it plays (unless some were missing).
    known: Known,
    /// Listen for the app's change notifications ([`watching`](Self::watching)).
    watch: bool,
}

/// What the Spotify app posts when it plays, pauses or changes track.
pub const NOTIFICATION: &str = "com.spotify.client.PlaybackStateChanged";

impl<R: Runner> Spotify<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            name: Arc::from(APP.name),
            known: Known::default(),
            watch: false,
        }
    }

    /// Also listen for the app's change notifications ([`NOTIFICATION`],
    /// through [`super::notify`]), polling at once on each.
    #[cfg(test)]
    pub fn watching(mut self) -> Self {
        self.watch = true;
        self
    }
}

impl<R: Runner> Backend for Spotify<R> {
    fn listen(&mut self, nudge: Nudge) -> Option<Box<dyn Send>> {
        if !self.watch {
            return None;
        }
        let watcher = super::notify::watch(&[NOTIFICATION], nudge)?;
        Some(Box::new(watcher))
    }

    fn exchange(&mut self, commands: &[Command]) -> Snapshot {
        let named = self.known.named(incomplete).to_owned();
        let result = self.runner.run(&request(commands, &named), TIMEOUT);
        let now = Instant::now();
        let mut snapshot = match result {
            Ok(out) => parse(&out, now, self.known.track.as_ref()),
            Err(err) => Snapshot::new(Status::Unavailable(classify(&err)), now),
        };
        if self.known.update(&mut snapshot, incomplete)
            && let Some(track) = snapshot.track.as_deref().filter(|t| incomplete(t))
        {
            crate::diag::note(|| {
                format!(
                    "spotify: track {} read without {}",
                    crate::diag::tag(&track.id),
                    missing(track).join(", "),
                )
            });
        }
        snapshot.player = Some(Arc::clone(&self.name));
        snapshot
    }

    /// Spotify 1.2's `set shuffling` / `set repeating` are no-ops (they
    /// read back unchanged, lava-75z.9), so the UI doesn't offer them.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            volume: true,
            uris: true,
            contexts: true,
            ..Capabilities::NONE
        }
    }
}

/// The loop with only the Spotify part (the live tests' own process).
#[cfg(test)]
pub fn script() -> String {
    applescript::script(&[(BUNDLE_ID, spotify_part())])
}

/// The Spotify part, compiled only once Spotify is known to be running: a
/// script object whose `poll(parts)` runs the request's commands and reads
/// the state.
pub fn spotify_part() -> String {
    let act = applescript::act_handler(
        BUNDLE_ID,
        "if v is \"playpause\" then\n\
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
         end if\n",
    );
    let mut s = format!(
        "script\n\
         {act}\
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
         set pos to -1\n\
         try\n\
         set pos to ((player position of p) * 1000) as integer\n\
         end try\n\
         set out to rs & ((shuffling of p) as integer) & rs & ((repeating of p) as integer) & rs & ((sound volume of p) as integer)\n\
         set fresh to false\n\
         try\n\
         set t to current track\n\
         set tid to id of t\n\
         if tid is missing value then set tid to \"\"\n\
         considering case\n\
         set same to tid is item 2 of parts\n\
         end considering\n\
         if same then\n\
         set out to out & rs & tid\n\
         else if tid is not \"\" then\n\
         set fresh to true\n"
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
        s += &applescript::detail(var, expr);
    }
    // The position comes from the first read, unless the track's details
    // were read too (five more Apple events, up to seconds at a track
    // change): then it's read again, last, so it was taken just before
    // the reply and the worker can tell when.
    s += &format!(
        "set out to out & rs & tid & rs & dur & rs & art & rs & ar & rs & al & rs & nm\n\
          end if\n\
          end try\n\
          if fresh or pos < 0 then\n\
          set pos to 0\n\
          try\n\
          set pos to ((player position) * 1000) as integer\n\
          end try\n\
          end if\n\
          return \"{HEADER}\" & rs & ((player state of p) as text) & rs & pos & out\n\
          end timeout\n\
          end tell\n\
          end poll\n\
          end script\n"
    );
    s
}

/// One request line: the header, the known track id, the commands.
pub fn request(commands: &[Command], known_id: &str) -> String {
    applescript::request(known_id, commands.iter().map(command_word))
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

/// Turn a reply into a snapshot sampled at `now`. `known` is the track
/// whose id the request named: a reply with only that id means it.
pub fn parse(out: &str, now: Instant, known: Option<&Arc<Track>>) -> Snapshot {
    applescript::parse(out, now, known, APP)
}

/// The details a read of `track` came back without, of those a track
/// should have: a name, and for a Spotify catalog track its artist, length
/// and cover too.
fn missing(track: &Track) -> Vec<&'static str> {
    let catalog = track.uri.is_some();
    [
        ("name", track.name.trim().is_empty()),
        ("artist", catalog && track.artist.trim().is_empty()),
        ("length", catalog && track.duration.is_zero()),
        ("cover", catalog && track.artwork_url.trim().is_empty()),
    ]
    .into_iter()
    .filter_map(|(what, gone)| gone.then_some(what))
    .collect()
}

fn incomplete(track: &Track) -> bool {
    !missing(track).is_empty()
}

/// Map a failure (osascript's, or an error the script reported) to a
/// reason the UI can explain.
pub fn classify(err: &RunError) -> Unavailable {
    applescript::classify(err, APP)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    use super::super::MediaSource;
    use super::super::applescript::{REREADS, Routed, SEP};
    use super::super::runner::Osascript;
    use super::super::worker::{Polled, testing};
    use super::*;

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

    /// A fresh process running the loop with the Spotify part only.
    fn own_process() -> Routed<Osascript> {
        Routed::new(BUNDLE_ID, Osascript::new(script()))
    }

    #[test]
    fn script_guards_before_touching_spotify() {
        let s = script();
        let guard = s.find("is running").unwrap();
        let tell = s.find("my part(bid)").unwrap();
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

    /// `PLAYING` with its cover and length missing, as Spotify can answer
    /// right as it changes track.
    fn half_loaded() -> String {
        let fields: Vec<&str> = PLAYING.split(SEP).collect();
        let mut fields: Vec<String> = fields.iter().map(|f| (*f).to_owned()).collect();
        fields[7] = "0".into();
        fields[8] = String::new();
        fields.join(&SEP.to_string())
    }

    /// lava-75z.22: a track first read with details missing is read in
    /// full again (not named as known) until they're there, keeping what
    /// the first read had; then it's known as usual.
    #[test]
    fn a_track_read_with_details_missing_is_read_again() {
        let id_only = "lavatui1\u{1e}playing\u{1e}242000\u{1e}0\u{1e}1\u{1e}100\u{1e}\
                       spotify:track:0DZXVpUtPUom1VO6h5a0SU";
        let no_name = half_loaded().replace("\u{1e}Life", "\u{1e}");
        let mut spotify = Spotify::new(Canned(
            [
                Ok(half_loaded()),
                // A re-read that lost the name keeps the first read's.
                Ok(no_name),
                Ok(PLAYING.to_owned()),
                Ok(id_only.to_owned()),
            ]
            .into(),
            Vec::new(),
        ));
        let first = spotify.exchange(&[]).track.unwrap();
        assert!(first.artwork_url.is_empty() && first.duration.is_zero());
        let second = spotify.exchange(&[]).track.unwrap();
        assert_eq!(second.name, "Life");
        let third = spotify.exchange(&[]).track.unwrap();
        assert!(third.artwork_url.starts_with("https://i.scdn.co/"));
        assert_eq!(third.duration, Duration::from_millis(303_440));
        let fourth = spotify.exchange(&[]).track.unwrap();
        assert!(Arc::ptr_eq(&third, &fourth));
        let requests = &spotify.runner.1;
        // Not named while incomplete, named once complete.
        assert_eq!(requests[1], "lavatui1\u{1e}");
        assert_eq!(requests[2], "lavatui1\u{1e}");
        assert_eq!(
            requests[3],
            "lavatui1\u{1e}spotify:track:0DZXVpUtPUom1VO6h5a0SU"
        );
    }

    /// Re-reads stop after [`REREADS`]: a track that never has a cover
    /// costs a few full reads, not one per poll for good. A new track
    /// starts the count again.
    #[test]
    fn rereads_are_bounded_per_track() {
        let mut spotify = Spotify::new(Canned(VecDeque::new(), Vec::new()));
        let short = "lavatui1\u{1e}playing\u{1e}242000\u{1e}0\u{1e}1\u{1e}100\u{1e}\
                     spotify:track:0DZXVpUtPUom1VO6h5a0SU";
        let polls = usize::from(REREADS) + 3;
        spotify
            .runner
            .0
            .extend((0..=REREADS).map(|_| Ok(half_loaded())));
        spotify
            .runner
            .0
            .extend((usize::from(REREADS) + 1..polls).map(|_| Ok(short.to_owned())));
        for _ in 0..polls {
            assert!(spotify.exchange(&[]).track.is_some());
        }
        let named = |r: &String| r.ends_with("0DZXVpUtPUom1VO6h5a0SU");
        let requests = &spotify.runner.1;
        // The first read, then REREADS full re-reads, then named.
        let full = requests.iter().filter(|r| !named(r)).count();
        assert_eq!(full, usize::from(REREADS) + 1);
        assert!(requests[usize::from(REREADS) + 1..].iter().all(named));

        // Another track: read again while it's incomplete too.
        let other = half_loaded().replace("0DZX", "1ABC");
        spotify.runner.0.extend([Ok(other.clone()), Ok(other)]);
        spotify.exchange(&[]);
        spotify.exchange(&[]);
        assert_eq!(spotify.runner.1.last().unwrap(), "lavatui1\u{1e}");
    }

    /// Local files and ads have no cover or Spotify URI for good: only a
    /// missing name makes them worth reading again.
    #[test]
    fn local_files_without_a_cover_are_complete() {
        let local = Track {
            id: "spotify:local:a:b:c:200".into(),
            name: "c".into(),
            ..Track::default()
        };
        assert!(!incomplete(&local));
        assert!(incomplete(&Track {
            name: String::new(),
            ..local
        }));
        let parsed = parse(PLAYING, now(), None).track.unwrap();
        assert!(!incomplete(&parsed));
        let half = parse(&half_loaded(), now(), None).track.unwrap();
        assert_eq!(missing(&half), ["length", "cover"]);
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
        // The worker owns the backend (and its runner): once that's
        // dropped, nothing runs.
        testing::released(&runner.0);
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
        let mut spotify = Spotify::new(Routed::new(
            "com.lavatui.nonexistent",
            Patient(Osascript::new(s)),
        ));
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

        type Live = Spotify<Routed<Osascript>>;

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
                let spotify = &mut Spotify::new(own_process());
                let now = exchange(spotify, "restore check", &[]).0;
                let mut fix = Vec::new();
                if now.status != self.0.status {
                    fix.push(Command::PlayPause);
                }
                fix.push(Command::SetVolume(self.0.volume));
                settled(spotify, "restore", &fix);
            }
        }

        let spotify = &mut Spotify::new(own_process());
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
        let source = Polled::spawn(Spotify::new(own_process()), Cadence::default());
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

    /// How far the app's extrapolated position is from the truth, against
    /// the real Spotify app (read-only: no commands).
    /// `LAVATUI_TIMING_SECS=300 cargo test --release -- --ignored
    /// --nocapture live_timing_audit`
    ///
    /// The truth: a second osascript polling back to back (~20 ms apart).
    /// Spotify's position is exact (thousands of reads fit one line to a
    /// few ms), so each reading pins playback to within its round trip;
    /// the intersection over a stretch of steady playback is the truth.
    /// Meanwhile the app's own worker polls as the app does and its
    /// snapshot is read every 10 ms, as a frame would. Prints the error
    /// (snapshot minus truth: positive is early), and how long a natural
    /// pause, resume, seek or track change took to show up. No song names
    /// or ids are printed.
    #[test]
    #[ignore = "reads the real Spotify app"]
    fn live_timing_audit() {
        use super::super::worker::Cadence;
        use std::sync::mpsc;

        let secs: u64 = std::env::var("LAVATUI_TIMING_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        let run = Duration::from_secs(secs);
        let t0 = Instant::now();
        let ms = move |t: Instant| t.saturating_duration_since(t0).as_secs_f64() * 1000.0;

        // The truth poller.
        struct Read {
            send: f64,
            recv: f64,
            playing: bool,
            pos: f64,
            track: String,
        }
        let (tx, rx) = mpsc::channel();
        let truth = thread::spawn(move || {
            let mut runner = own_process();
            let mut known = String::new();
            while t0.elapsed() < run + Duration::from_secs(2) {
                let send = Instant::now();
                let Ok(out) = runner.run(&request(&[], &known), TIMEOUT) else {
                    continue;
                };
                let recv = Instant::now();
                let fields: Vec<&str> = out.split(SEP).collect();
                if let Some(id) = fields.get(6) {
                    known = (*id).to_owned();
                }
                let _ = tx.send(Read {
                    send: ms(send),
                    recv: ms(recv),
                    playing: fields.get(1) == Some(&"playing"),
                    pos: fields
                        .get(2)
                        .and_then(|p| p.trim().parse().ok())
                        .unwrap_or(0.0),
                    track: known.clone(),
                });
                thread::sleep(Duration::from_millis(5));
            }
        });

        // The app's view, every 10 ms.
        // The app's source, with change events unless
        // LAVATUI_TIMING_EVENTS=0 (polling alone, as before events).
        let events = std::env::var("LAVATUI_TIMING_EVENTS").as_deref() != Ok("0");
        let backend = Spotify::new(own_process());
        let backend = if events { backend.watching() } else { backend };
        let source = Polled::spawn(backend, Cadence::default());
        // When Spotify's notifications arrive, for the report.
        let (nudge, heard) = super::super::worker::Nudge::channel();
        let _watcher = super::super::notify::watch(&[NOTIFICATION], nudge);
        let noted = thread::spawn(move || {
            let mut at = Vec::new();
            while heard.recv().is_ok() {
                at.push(ms(Instant::now()));
            }
            at
        });
        let mut seen = Vec::new();
        while t0.elapsed() < run {
            let now = Instant::now();
            let snap = source.snapshot();
            if snap.status.is_available() {
                let id = snap.track.as_ref().map_or(String::new(), |t| t.id.clone());
                seen.push((
                    ms(now),
                    snap.position_at(now).as_secs_f64() * 1000.0,
                    snap.status == Status::Playing,
                    id,
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        drop(source);
        drop(_watcher);
        truth.join().expect("truth poller");
        let noted: Vec<f64> = noted.join().unwrap_or_default();
        println!(
            "events {}: {} notifications at {:.0?} ms",
            if events { "on" } else { "off" },
            noted.len(),
            noted
        );
        let reads: Vec<Read> = rx.try_iter().collect();

        // Steady stretches of the truth: same track, same state, and each
        // reading within 40 ms of the stretch so far. A stretch's baseline
        // (position − time while playing) is the median of its quicker
        // readings' midpoints (each reading's error: ± half its round trip).
        // (from, to, playing, track, baseline)
        let mut spans: Vec<(f64, f64, bool, String, f64)> = Vec::new();
        let mut mids: Vec<Vec<(f64, f64)>> = Vec::new();
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        for r in &reads {
            let mid = if r.playing {
                r.pos - (r.send + r.recv) / 2.0
            } else {
                r.pos
            };
            let rtt = r.recv - r.send;
            let joins = spans.last().zip(mids.last()).is_some_and(|(s, m)| {
                let mut recent: Vec<f64> = m.iter().rev().take(15).map(|x| x.0).collect();
                s.2 == r.playing && s.3 == r.track && (mid - median(&mut recent)).abs() <= 40.0
            });
            if joins {
                spans.last_mut().unwrap().1 = r.recv;
                mids.last_mut().unwrap().push((mid, rtt));
            } else {
                spans.push((r.send, r.recv, r.playing, r.track.clone(), mid));
                mids.push(vec![(mid, rtt)]);
            }
        }
        for (s, m) in spans.iter_mut().zip(&mids) {
            let mut rtts: Vec<f64> = m.iter().map(|x| x.1).collect();
            let quick = median(&mut rtts);
            let mut best: Vec<f64> = m.iter().filter(|x| x.1 <= quick).map(|x| x.0).collect();
            s.4 = median(&mut best);
        }
        // Short stretches (Spotify settling after a seek or at a track
        // start) are skipped.
        let counts: Vec<usize> = mids.iter().map(Vec::len).collect();
        let (spans, counts): (Vec<_>, Vec<_>) = spans
            .into_iter()
            .zip(counts)
            .filter(|(s, _)| s.1 - s.0 >= 1000.0)
            .unzip();
        println!(
            "truth: {} reads, {} steady stretches",
            reads.len(),
            spans.len()
        );
        for (s, n) in spans.iter().zip(&counts) {
            println!(
                "  {:>8.0}..{:>8.0} ms {} ({n} reads)",
                s.0,
                s.1,
                if s.2 { "playing" } else { "paused " },
            );
        }
        if let Ok(path) = std::env::var("LAVATUI_TIMING_CSV") {
            use std::fmt::Write as _;
            let mut csv = String::from("kind,t,send_or_pos,recv,playing,pos,track\n");
            for r in &reads {
                let _ = writeln!(
                    csv,
                    "truth,,{:.3},{:.3},{},{},{}",
                    r.send,
                    r.recv,
                    r.playing,
                    r.pos,
                    r.track.len() % 97
                );
            }
            for (t, pos, playing, track) in &seen {
                let _ = writeln!(csv, "app,{t:.3},{pos:.3},,{playing},,{}", track.len() % 97);
            }
            std::fs::write(path, csv).expect("write csv");
        }

        // The error wherever the truth is steady (a second into a stretch,
        // so detection delays are counted apart, below).
        let mut errors = Vec::new();
        let mut lagged = Vec::new();
        for (t, pos, playing, track) in &seen {
            let Some(s) = spans.iter().find(|s| s.0 <= *t && *t <= s.1) else {
                continue;
            };
            let truth = if s.2 { t + s.4 } else { s.4 };
            let err = pos - truth;
            if *t >= s.0 + 1500.0 && s.2 == *playing && &s.3 == track {
                errors.push(err);
            } else if *t < s.0 + 1500.0 {
                lagged.push((s.0, *t, err));
            }
        }
        errors.sort_by(f64::total_cmp);
        let q = |p: f64| errors[((errors.len() - 1) as f64 * p) as usize];
        if errors.is_empty() {
            println!("no steady playback seen");
            return;
        }
        let mean = errors.iter().sum::<f64>() / errors.len() as f64;
        let abs_max = q(0.0).abs().max(q(1.0).abs());
        println!(
            "steady error (snapshot - truth, ms): mean {mean:+.1}  p1 {:+.1}  p50 {:+.1}  p99 {:+.1}  max |{abs_max:.1}|  ({} frames)",
            q(0.01),
            q(0.5),
            q(0.99),
            errors.len()
        );
        // After each change in the truth: how long until the snapshot was
        // within 50 ms of it.
        for s in spans.iter().skip(1) {
            let settled =
                seen.iter()
                    .filter(|(t, ..)| *t >= s.0 && *t <= s.1)
                    .find(|(t, pos, ..)| {
                        let truth = if s.2 { t + s.4 } else { s.4 };
                        (pos - truth).abs() < 50.0
                    });
            let worst = lagged
                .iter()
                .filter(|l| l.0 == s.0)
                .map(|l| l.2)
                .fold(0.0f64, |a, b| if b.abs() > a.abs() { b } else { a });
            // A notification near the change (it may come a little before
            // the first read that shows it).
            let note = noted
                .iter()
                .find(|&&n| n > s.0 - 1500.0 && n < s.0 + 3000.0)
                .map_or("no notification".to_owned(), |n| {
                    format!("notification {:+.0} ms", n - s.0)
                });
            match settled {
                Some((t, ..)) => println!(
                    "  change at {:.0} ms ({}): shown within 50 ms after {:.0} ms; worst error before {worst:+.0} ms; {note}",
                    s.0,
                    if s.2 { "playing" } else { "paused" },
                    t - s.0
                ),
                None => println!("  change at {:.0} ms: never settled", s.0),
            }
        }
    }
}
