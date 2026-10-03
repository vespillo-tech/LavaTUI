//! Apple Music (the Music app) on macOS, driven through AppleScript (the
//! module only exists on macOS).
//!
//! [`music_part`] is the Music part of the shared AppleScript loop
//! ([`applescript`](super::applescript): one long-lived `osascript`, the
//! never-launch guard, the record format). A request is `lavatui1 ␞ known
//! track id ␞ cover file ␞ command…`.
//!
//! Music has no artwork URL: it hands over the picture itself (`raw data
//! of artwork 1`, JPEG or PNG). When a track is read in full the script
//! writes it to the cover file named in the request (a private temporary
//! file) and says `1` in the artwork field; [`AppleMusic`] reads the file
//! on the worker, gives the bytes to the art loader
//! ([`art::stash`](super::art::stash), as on Windows) and puts the
//! `lavatui-thumb:` URL it gets in [`Track::artwork_url`].
//!
//! The track id is Music's `persistent ID`; a radio stream adds its
//! current stream title, so each song on a station is a new track (named
//! after the stream title, the station as its album).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use super::applescript::{self, App, EVENT_TIMEOUT_SECS, HEADER, Known, TIMEOUT};
use super::modes::ModesCheck;
use super::runner::Runner;
use super::worker::Backend;
use super::{Capabilities, Command, Snapshot, Status, Track};

pub const BUNDLE_ID: &str = "com.apple.Music";
pub const APP: App = App {
    bundle: BUNDLE_ID,
    name: "Music",
};

/// What Music posts when it plays, pauses, stops or changes track.
pub const NOTIFICATION: &str = "com.apple.Music.playerInfo";

/// What the script says in the artwork field when it wrote the cover.
const COVER_WRITTEN: &str = "1";

/// The Apple Music backend, over a [`Runner`] (the shared process,
/// through a [`Routed`](super::applescript::Routed) runner).
pub struct AppleMusic<R> {
    runner: R,
    name: Arc<str>,
    known: Known,
    /// Where the script writes a track's cover (`None`: covers off).
    cover_file: Option<PathBuf>,
    /// Whether Music does what shuffle / repeat are set to.
    modes: ModesCheck,
}

impl<R: Runner> AppleMusic<R> {
    pub fn new(runner: R) -> Self {
        let file = format!("lavatui-{}-music-cover", std::process::id());
        Self {
            runner,
            name: Arc::from(APP.name),
            known: Known::default(),
            cover_file: Some(std::env::temp_dir().join(file)),
            modes: ModesCheck::default(),
        }
    }

    /// Covers written to `file` instead (tests).
    #[cfg(test)]
    fn with_cover_file(mut self, file: Option<PathBuf>) -> Self {
        self.cover_file = file;
        self
    }

    /// The cover file's path as the request carries it ("" when there's
    /// none or it can't be framed).
    fn cover_field(&self) -> String {
        self.cover_file
            .as_deref()
            .and_then(|p| p.to_str())
            .map(applescript::field)
            .unwrap_or_default()
            .to_owned()
    }

    /// `track` with its artwork field turned into a cover URL: the bytes
    /// the script just wrote, stashed for the art loader.
    fn cover(&self, track: &Arc<Track>) -> Arc<Track> {
        let url = match (&self.cover_file, track.artwork_url.as_str()) {
            (Some(file), COVER_WRITTEN) => {
                let bytes = std::fs::read(file).ok();
                let _ = std::fs::remove_file(file);
                bytes.and_then(super::art::stash).unwrap_or_default()
            }
            _ => String::new(),
        };
        Arc::new(Track {
            artwork_url: url,
            ..Track::clone(track)
        })
    }
}

impl<R: Runner> Backend for AppleMusic<R> {
    fn exchange(&mut self, commands: &[Command]) -> Snapshot {
        let named = self.known.named(incomplete).to_owned();
        let rest = std::iter::once(self.cover_field()).chain(commands.iter().map(command_word));
        let result = self
            .runner
            .run(&applescript::request(&named, rest), TIMEOUT);
        let now = Instant::now();
        let mut snapshot = match result {
            Ok(out) => applescript::parse(&out, now, self.known.track.as_ref(), APP),
            Err(err) => Snapshot::new(Status::Unavailable(applescript::classify(&err, APP)), now),
        };
        // A track read in full: its cover is in the file, if it has one.
        if let Some(track) = &mut snapshot.track
            && !self
                .known
                .track
                .as_ref()
                .is_some_and(|k| Arc::ptr_eq(k, track))
        {
            *track = self.cover(track);
        }
        self.known.update(&mut snapshot, incomplete);
        if snapshot.status.is_available() {
            for command in commands {
                self.modes.sent(command, now);
            }
            self.modes.read(&snapshot, now);
        }
        snapshot.player = Some(Arc::clone(&self.name));
        snapshot
    }

    /// Music can't be told what to play (no URIs); shuffle and repeat
    /// until it's seen to ignore them.
    fn capabilities(&self) -> Capabilities {
        let modes = !self.modes.ignored();
        Capabilities {
            shuffle: modes,
            repeat: modes,
            volume: true,
            ..Capabilities::NONE
        }
    }
}

/// A track read without a name or a cover is read again (Music can still
/// be loading a streamed song's artwork as it starts).
fn incomplete(track: &Track) -> bool {
    track.name.trim().is_empty() || track.artwork_url.is_empty()
}

fn command_word(command: &Command) -> String {
    match command {
        Command::PlayPause => "playpause".into(),
        Command::Next => "next".into(),
        Command::Previous => "previous".into(),
        // Whole milliseconds (see Spotify's).
        Command::Seek(to) => format!("seek {}", to.as_millis()),
        Command::SetShuffle(on) => format!("shuffle {on}"),
        Command::SetRepeat(on) => format!("repeat {on}"),
        // Exact, except that Music reads 1 back as 0 (macOS 26.6).
        Command::SetVolume(volume) => format!("volume {}", (*volume).min(100)),
        // Never offered (`capabilities`): nothing to do.
        Command::PlayUri(_) | Command::PlayInContext { .. } => String::new(),
    }
}

/// The Music part, compiled only once Music is known to be running: a
/// script object whose `poll(parts)` runs the request's commands and reads
/// the state.
pub fn music_part() -> String {
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
         set shuffle enabled to (a is \"true\")\n\
         else if v is \"repeat\" then\n\
         if a is \"true\" then\n\
         set song repeat to all\n\
         else\n\
         set song repeat to off\n\
         end if\n\
         else if v is \"volume\" then\n\
         set sound volume to (a as integer)\n\
         end if\n",
    );
    let mut s = format!(
        "script\n\
         {act}\
         on keepcover(t, p)\n\
         if p is \"\" then return \"\"\n\
         tell application id \"{BUNDLE_ID}\"\n\
         if (count of artworks of t) is 0 then return \"\"\n\
         set raw to raw data of artwork 1 of t\n\
         end tell\n\
         set fh to open for access (POSIX file p) with write permission\n\
         try\n\
         set eof fh to 0\n\
         write raw to fh\n\
         on error\n\
         close access fh\n\
         return \"\"\n\
         end try\n\
         close access fh\n\
         return \"{COVER_WRITTEN}\"\n\
         end keepcover\n\
         on poll(parts)\n\
         set rs to character id 30\n\
         tell application id \"{BUNDLE_ID}\"\n\
         with timeout of {EVENT_TIMEOUT_SECS} seconds\n\
         repeat with i from 4 to count of parts\n\
         try\n\
         my act(item i of parts)\n\
         end try\n\
         end repeat\n\
         set plst to (player state) as text\n\
         set sh to 0\n\
         try\n\
         if shuffle enabled then set sh to 1\n\
         end try\n\
         set rp to 0\n\
         try\n\
         if song repeat is not off then set rp to 1\n\
         end try\n\
         set vol to 0\n\
         try\n\
         set vol to sound volume\n\
         end try\n\
         set strm to \"\"\n\
         try\n\
         set strm to current stream title\n\
         if strm is missing value then set strm to \"\"\n\
         end try\n\
         set pos to -1\n\
         try\n\
         set pos to ((player position) * 1000) as integer\n\
         end try\n\
         set out to rs & sh & rs & rp & rs & vol\n\
         set fresh to false\n\
         try\n\
         set t to current track\n\
         set tid to persistent ID of t\n\
         if tid is missing value then set tid to \"\"\n\
         if strm is not \"\" then set tid to tid & \"/\" & strm\n\
         considering case\n\
         set same to tid is item 2 of parts\n\
         end considering\n\
         if same then\n\
         set out to out & rs & tid\n\
         else if tid is not \"\" then\n\
         set fresh to true\n"
    );
    // One read per detail, each allowed to fail (streams lack some), in
    // record order.
    for (var, expr) in [
        ("dur", "((duration of t) * 1000) as integer"),
        ("art", "my keepcover(t, item 3 of parts)"),
        ("ar", "artist of t"),
        ("al", "album of t"),
        ("nm", "name of t"),
    ] {
        s += &applescript::detail(var, expr);
    }
    // A stream: the song is its stream title, the station its album.
    // The position is read last after a full read (see Spotify's).
    s += &format!(
        "if strm is not \"\" then\n\
         set al to nm\n\
         set nm to strm\n\
         end if\n\
         set out to out & rs & tid & rs & dur & rs & art & rs & ar & rs & al & rs & nm\n\
         end if\n\
         end try\n\
         if fresh or pos < 0 then\n\
         set pos to 0\n\
         try\n\
         set pos to ((player position) * 1000) as integer\n\
         end try\n\
         end if\n\
         return \"{HEADER}\" & rs & plst & rs & pos & out\n\
         end timeout\n\
         end tell\n\
         end poll\n\
         end script\n"
    );
    s
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use super::super::Unavailable;
    use super::super::applescript::{REREADS, SEP};
    use super::super::runner::RunError;
    use super::*;

    /// The shape Music's part answers in: playing a library song whose
    /// cover was written (`1`).
    const PLAYING: &str = "lavatui1\u{1e}playing\u{1e}61250\u{1e}1\u{1e}0\u{1e}35\u{1e}\
        0123456789ABCDEF\u{1e}215000\u{1e}1\u{1e}The Made-Up Band\u{1e}Invented Album\u{1e}\
        First Song";
    const SAME: &str =
        "lavatui1\u{1e}paused\u{1e}62000\u{1e}1\u{1e}1\u{1e}35\u{1e}0123456789ABCDEF";

    /// A runner that answers from a list and records the requests; each
    /// reply that says the cover was written writes `cover` to the file
    /// the request names, as the script would.
    struct Canned {
        replies: VecDeque<Result<String, RunError>>,
        requests: Vec<String>,
        cover: Vec<u8>,
    }

    impl Canned {
        fn new(replies: impl IntoIterator<Item = Result<String, RunError>>) -> Self {
            Self {
                replies: replies.into_iter().collect(),
                requests: Vec::new(),
                cover: png(),
            }
        }
    }

    impl Runner for Canned {
        fn run(&mut self, request: &str, _: Duration) -> Result<String, RunError> {
            self.requests.push(request.to_owned());
            let reply = self.replies.pop_front().unwrap_or(Err(RunError::Timeout));
            if let Ok(out) = &reply
                && out.split(SEP).nth(8) == Some(COVER_WRITTEN)
            {
                let file = request.split(SEP).nth(2).unwrap();
                std::fs::write(file, &self.cover).unwrap();
            }
            reply
        }
    }

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        let image = image::RgbImage::from_pixel(4, 4, image::Rgb([200, 40, 90]));
        image::DynamicImage::ImageRgb8(image)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        bytes
    }

    /// A cover file of the test's own.
    fn cover_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("lavatui-test-{}-{name}", std::process::id()))
    }

    fn music(canned: Canned, name: &str) -> AppleMusic<Canned> {
        AppleMusic::new(canned).with_cover_file(Some(cover_file(name)))
    }

    #[test]
    fn a_playing_song_is_read_with_its_cover_stashed() {
        let mut music = music(Canned::new([Ok(PLAYING.to_owned())]), "read");
        let snap = music.exchange(&[]);
        assert_eq!(snap.status, Status::Playing);
        assert_eq!(snap.player.as_deref(), Some("Music"));
        assert!(!snap.is_spotify());
        assert_eq!(snap.position, Duration::from_millis(61_250));
        assert!(snap.shuffle && !snap.repeat);
        assert_eq!(snap.volume, 35);
        let track = snap.track.unwrap();
        assert_eq!(track.id, "0123456789ABCDEF");
        assert_eq!(track.uri, None, "not a Spotify track");
        assert_eq!(track.name, "First Song");
        assert_eq!(track.artist, "The Made-Up Band");
        assert_eq!(track.album, "Invented Album");
        assert_eq!(track.duration, Duration::from_millis(215_000));
        assert!(
            track
                .artwork_url
                .starts_with(crate::media::art::THUMB_SCHEME),
            "{}",
            track.artwork_url
        );
        assert!(!cover_file("read").exists(), "the file is cleaned up");
        // The request named nothing yet and carried the cover file.
        let fields: Vec<&str> = music.runner.requests[0].split(SEP).collect();
        assert_eq!(fields[..2], ["lavatui1", ""]);
        assert_eq!(fields[2], cover_file("read").to_str().unwrap());
    }

    #[test]
    fn the_known_song_is_named_and_reused() {
        let mut music = music(
            Canned::new([Ok(PLAYING.to_owned()), Ok(SAME.to_owned())]),
            "known",
        );
        let first = music.exchange(&[]).track.unwrap();
        let snap = music.exchange(&[Command::PlayPause]);
        assert_eq!(snap.status, Status::Paused);
        assert!(snap.repeat);
        assert!(Arc::ptr_eq(snap.track.as_ref().unwrap(), &first));
        let fields: Vec<&str> = music.runner.requests[1].split(SEP).collect();
        assert_eq!(fields[1], "0123456789ABCDEF");
        assert_eq!(fields[3], "playpause");
    }

    #[test]
    fn a_song_without_a_cover_is_read_again_a_few_times() {
        let bare = PLAYING.replace("\u{1e}1\u{1e}The", "\u{1e}\u{1e}The");
        let mut music = music(Canned::new([]), "bare");
        music
            .runner
            .replies
            .extend((0..=REREADS + 2).map(|_| Ok(bare.clone())));
        for _ in 0..=REREADS + 2 {
            let track = music.exchange(&[]).track.unwrap();
            assert_eq!(track.artwork_url, "");
        }
        let named = |r: &String| r.split(SEP).nth(1) == Some("0123456789ABCDEF");
        let full = music.runner.requests.iter().filter(|r| !named(r)).count();
        assert_eq!(full, usize::from(REREADS) + 1);
        assert_eq!(music.known.rereads(), 0);
    }

    #[test]
    fn a_cover_that_comes_later_is_kept() {
        let bare = PLAYING.replace("\u{1e}1\u{1e}The", "\u{1e}\u{1e}The");
        let mut music = music(
            Canned::new([Ok(bare), Ok(PLAYING.to_owned()), Ok(SAME.to_owned())]),
            "later",
        );
        assert_eq!(music.exchange(&[]).track.unwrap().artwork_url, "");
        let second = music.exchange(&[]).track.unwrap();
        assert!(
            second
                .artwork_url
                .starts_with(crate::media::art::THUMB_SCHEME)
        );
        let third = music.exchange(&[]).track.unwrap();
        assert!(Arc::ptr_eq(&second, &third));
    }

    #[test]
    fn a_cover_that_wont_load_is_no_cover() {
        let mut canned = Canned::new([Ok(PLAYING.to_owned())]);
        canned.cover = Vec::new();
        let mut music = music(canned, "empty");
        assert_eq!(music.exchange(&[]).track.unwrap().artwork_url, "");
        // No file to write to: the field is empty and nothing is read.
        let mut music = AppleMusic::new(Canned::new([Ok(SAME.to_owned())])).with_cover_file(None);
        music.exchange(&[]);
        assert_eq!(music.runner.requests[0], "lavatui1\u{1e}\u{1e}");
    }

    #[test]
    fn a_radio_stream_is_named_by_its_stream_title() {
        // As the part builds it: the station's name moves to the album.
        let out = "lavatui1\u{1e}playing\u{1e}5000\u{1e}0\u{1e}0\u{1e}50\u{1e}\
                   FEDCBA9876543210/Someone - Something\u{1e}0\u{1e}\u{1e}\u{1e}\
                   Made-Up Radio\u{1e}Someone - Something";
        let mut music = music(Canned::new([Ok(out.to_owned())]), "radio");
        let track = music.exchange(&[]).track.unwrap();
        assert_eq!(track.name, "Someone - Something");
        assert_eq!(track.album, "Made-Up Radio");
        assert_eq!(track.duration, Duration::ZERO);
    }

    #[test]
    fn commands_become_words() {
        let mut music = music(Canned::new([Ok(SAME.to_owned())]), "words");
        music.exchange(&[
            Command::Next,
            Command::Previous,
            Command::Seek(Duration::from_millis(61_250)),
            Command::SetShuffle(true),
            Command::SetRepeat(false),
            Command::SetVolume(42),
            Command::SetVolume(200),
        ]);
        let fields: Vec<&str> = music.runner.requests[0].split(SEP).skip(3).collect();
        assert_eq!(
            fields,
            [
                "next",
                "previous",
                "seek 61250",
                "shuffle true",
                "repeat false",
                "volume 42",
                "volume 100"
            ]
        );
    }

    #[test]
    fn shuffle_and_repeat_stop_being_offered_once_ignored() {
        let mut music = music(Canned::new([]), "modes");
        assert!(music.capabilities().shuffle && music.capabilities().repeat);
        assert!(
            !music.capabilities().uris,
            "Music can't be told what to play"
        );
        // Asked for shuffle, read without it for longer than the grace.
        let off = SAME.replace(
            "\u{1e}paused\u{1e}62000\u{1e}1",
            "\u{1e}paused\u{1e}62000\u{1e}0",
        );
        music.runner.replies.push_back(Ok(PLAYING.to_owned()));
        music.exchange(&[]);
        music.runner.replies.push_back(Ok(off.clone()));
        music.exchange(&[Command::SetShuffle(true)]);
        std::thread::sleep(crate::media::modes::MODES_GRACE);
        music.runner.replies.push_back(Ok(off));
        music.exchange(&[]);
        assert!(!music.capabilities().shuffle);
    }

    #[test]
    fn failures_are_explained_for_music() {
        let mut music = music(
            Canned::new([
                Ok("lavatui1\u{1e}not running".to_owned()),
                Ok("lavatui1\u{1e}error\u{1e}-1743\u{1e}Not authorized to send Apple events to Music."
                    .to_owned()),
                Err(RunError::Timeout),
            ]),
            "errors",
        );
        let want = [
            Unavailable::NotRunning,
            Unavailable::PermissionDenied,
            Unavailable::NotResponding,
        ];
        for want in want {
            let snap = music.exchange(&[]);
            assert_eq!(snap.status, Status::Unavailable(want.clone()));
            assert_eq!(snap.player.as_deref(), Some("Music"));
        }
        assert!(
            Unavailable::NotRunning
                .message("Music")
                .contains("Open Music")
        );
    }

    #[test]
    fn the_part_only_talks_to_music() {
        let part = music_part();
        assert_eq!(
            part.matches("tell application id").count(),
            part.matches("tell application id \"com.apple.Music\"")
                .count()
        );
        // The cover is written outside any tell (a scripting addition).
        let save = &part[part.find("on keepcover").unwrap()..part.find("end keepcover").unwrap()];
        let open = save.find("open for access").unwrap();
        assert!(save[..open].rfind("end tell") > save[..open].rfind("tell application"));
    }

    /// The Music part compiles against Music's own dictionary, without
    /// Music: a stand-in app (a few lines of Cocoa, built here with clang)
    /// carries a copy of Music's scripting definition under a made-up
    /// name, and `osacompile` resolves the part's terms against it. Music
    /// itself is never asked, so it can't launch.
    /// `cargo test -- --ignored --nocapture music_part_compiles`
    #[test]
    #[ignore = "builds a stand-in app with clang"]
    fn music_part_compiles_against_musics_dictionary() {
        use std::process::Command as Process;

        let stamp = Instant::now().elapsed().as_nanos() ^ u128::from(std::process::id());
        let dir = std::env::temp_dir().join(format!("lavatui-music-standin-{stamp:x}"));
        let app = dir.join("Stand-in.app/Contents");
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Resources")).unwrap();
        std::fs::copy(
            "/System/Applications/Music.app/Contents/Resources/com.apple.Music.sdef",
            app.join("Resources/Standin.sdef"),
        )
        .expect("Music's scripting definition");
        std::fs::write(
            app.join("Info.plist"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <plist version=\"1.0\"><dict>\n\
             <key>CFBundleIdentifier</key><string>dev.lavatui.test.standin</string>\n\
             <key>CFBundleExecutable</key><string>Standin</string>\n\
             <key>CFBundlePackageType</key><string>APPL</string>\n\
             <key>OSAScriptingDefinition</key><string>Standin.sdef</string>\n\
             <key>NSAppleScriptEnabled</key><true/>\n\
             <key>LSBackgroundOnly</key><true/>\n\
             </dict></plist>\n",
        )
        .unwrap();
        // Serves its dictionary, then quits by itself.
        std::fs::write(
            dir.join("main.m"),
            "#import <Cocoa/Cocoa.h>\n\
             int main(void) { @autoreleasepool { [NSApplication sharedApplication];\n\
             [NSTimer scheduledTimerWithTimeInterval:20 repeats:NO block:^(NSTimer *t) { exit(0); }];\n\
             [NSApp run]; } return 0; }\n",
        )
        .unwrap();
        let built = Process::new("clang")
            .args(["-fobjc-arc", "-framework", "Cocoa", "-o"])
            .arg(app.join("MacOS/Standin"))
            .arg(dir.join("main.m"))
            .status()
            .is_ok_and(|s| s.success());
        assert!(
            built,
            "clang (Xcode command line tools) builds the stand-in"
        );

        let path = dir.join("Stand-in.app");
        let part = music_part().replace(
            "application id \"com.apple.Music\"",
            &format!("application \"{}\"", path.display()),
        );
        assert!(!part.contains("com.apple.Music"));
        std::fs::write(dir.join("part.applescript"), &part).unwrap();
        let out = Process::new("/usr/bin/osacompile")
            .arg("-o")
            .arg(dir.join("part.scpt"))
            .arg(dir.join("part.applescript"))
            .output()
            .unwrap();
        let _ = Process::new("/usr/bin/pkill")
            .arg("-f")
            .arg(path.join("Contents/MacOS/Standin"))
            .status();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{stderr}");
        println!("the Music part compiles against Music's dictionary");
    }

    /// Against the real Music app, which must be open (it's never opened
    /// for you). Read-only unless `LAVATUI_LIVE_CHANGES=1`: then volume,
    /// shuffle and repeat are each changed and put back (and play/pause,
    /// only with a song loaded and playing or paused: from stopped it would
    /// start something), whether Music honoured them is printed, and
    /// everything must be as it was at the end. No song names are printed.
    /// `cargo test --release -- --ignored --nocapture live_music`
    #[test]
    #[ignore = "reads (and with LAVATUI_LIVE_CHANGES=1 drives) the real Music app"]
    fn live_music() {
        use std::sync::Mutex;
        use std::thread::sleep;

        use super::super::applescript::Routed;
        use super::super::runner::Osascript;

        const SETTLE: Duration = Duration::from_millis(500);
        type Live = AppleMusic<Routed<Arc<Mutex<Osascript>>>>;

        fn read(music: &mut Live, label: &str, commands: &[Command]) -> Snapshot {
            let start = Instant::now();
            let snap = music.exchange(commands);
            let track = snap.track.as_deref();
            println!(
                "{label:<14} {:>5.0} ms  {:?} pos {:.1}s shuffle {} repeat {} vol {}  track {} {:.0}s cover {}",
                start.elapsed().as_secs_f64() * 1000.0,
                snap.status,
                snap.position.as_secs_f64(),
                snap.shuffle,
                snap.repeat,
                snap.volume,
                track.map_or("none", |t| if t.name.is_empty() {
                    "unnamed"
                } else {
                    "named"
                }),
                track.map_or(0.0, |t| t.duration.as_secs_f64()),
                track.map_or("-", |t| t.artwork_url.split(':').next().unwrap_or("")),
            );
            snap
        }

        let script = applescript::script(&[(BUNDLE_ID, music_part())]);
        let shared = Arc::new(Mutex::new(Osascript::new(script)));
        let music = &mut AppleMusic::new(Routed::new(BUNDLE_ID, shared));
        let first = read(music, "first", &[]);
        if first.status == Status::Unavailable(Unavailable::NotRunning) {
            println!("Music isn't open: nothing to check (it was not opened)");
            return;
        }
        for _ in 0..3 {
            read(music, "poll", &[]);
        }
        if first.status != Status::Playing {
            println!("nothing is playing in Music");
        }
        if std::env::var("LAVATUI_LIVE_CHANGES").as_deref() != Ok("1") {
            return;
        }
        let mut changed =
            |label: &str, command: Command, back: Command, check: &dyn Fn(&Snapshot) -> bool| {
                read(music, label, &[command]);
                sleep(SETTLE);
                let after = read(music, "  settled", &[]);
                println!(
                    "  {label}: {}",
                    if check(&after) { "honoured" } else { "IGNORED" }
                );
                read(music, "  back", &[back]);
                sleep(SETTLE);
                read(music, "  settled", &[]);
            };
        // Music reads 1 back as 0 (every other level is exact): stay clear.
        let quieter = if first.volume > 12 {
            first.volume - 10
        } else {
            first.volume + 10
        };
        changed(
            "volume",
            Command::SetVolume(quieter),
            Command::SetVolume(first.volume),
            &|s| s.volume == quieter,
        );
        let other = match first.status {
            Status::Playing => Some(Status::Paused),
            Status::Paused => Some(Status::Playing),
            _ => None,
        };
        match other.filter(|_| first.track.is_some()) {
            Some(other) => changed("play/pause", Command::PlayPause, Command::PlayPause, &|s| {
                s.status == other
            }),
            None => println!("  play/pause: skipped (no song loaded, or stopped)"),
        }
        let shuffle = !first.shuffle;
        changed(
            "shuffle",
            Command::SetShuffle(shuffle),
            Command::SetShuffle(first.shuffle),
            &|s| s.shuffle == shuffle,
        );
        let repeat = !first.repeat;
        changed(
            "repeat",
            Command::SetRepeat(repeat),
            Command::SetRepeat(first.repeat),
            &|s| s.repeat == repeat,
        );
        let last = read(music, "end", &[]);
        assert_eq!(
            (&last.status, last.volume, last.shuffle, last.repeat),
            (&first.status, first.volume, first.shuffle, first.repeat),
            "left as it was found"
        );
    }
}
