//! Opt-in tests against the real Spotify Web API (all `#[ignore]`d), for
//! any account on any OS. Everything they need comes from the environment:
//!
//! - `LAVATUI_SPOTIFY_CLIENT_ID`: your Spotify app's Client ID (required;
//!   the same variable the app reads).
//! - `LAVATUI_SPOTIFY_TOKEN_FILE`: where the login is kept (the app's own
//!   variable: a file, never the OS keyring). Default: a file in the temp
//!   directory named after the Client ID. Give each account its own file;
//!   delete it to log in afresh.
//! - `LAVATUI_LIVE_CHANGES=1`: consent to the tests that change something
//!   on the account (like then unlike, add to the test playlist, playback
//!   for a few seconds). Without it they stop before the first change.
//! - `LAVATUI_TEST_PLAYLIST`: the id of a playlist of yours they may add
//!   to and play. Else one named "lavatui test" (`live_account` creates
//!   it, private, when missing).
//! - `LAVATUI_TEST_TRACK`: the `spotify:track:…` to like, unlike and add.
//!   Else what the account is playing (Web API, any device), else (only
//!   `live_account`) a track from one of your playlists.
//!
//! The first run logs in through the browser (someone clicks Agree). Only
//! `live_play_in_context` and `live_player_in_a_playlist` drive the macOS
//! desktop app (AppleScript) and are macOS only; the rest is HTTP only.
//!
//! `LAVATUI_SPOTIFY_CLIENT_ID=… LAVATUI_LIVE_CHANGES=1 cargo test --
//! --ignored --nocapture live_account`

use std::path::PathBuf;
use std::time::Duration;

use super::client::Client;
use super::store::{FileStore, MemoryStore};
use super::*;

const TEST_PLAYLIST: &str = "lavatui test";
const CHANGES_ENV: &str = "LAVATUI_LIVE_CHANGES";
const PLAYLIST_ENV: &str = "LAVATUI_TEST_PLAYLIST";
const TRACK_ENV: &str = "LAVATUI_TEST_TRACK";

fn env(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// The token file: `LAVATUI_SPOTIFY_TOKEN_FILE`, else one per Client ID in
/// the temp directory.
fn token_file(client_id: &str) -> PathBuf {
    token_file_from(env(TOKEN_FILE_ENV), client_id)
}

fn token_file_from(set: Option<String>, client_id: &str) -> PathBuf {
    set.map(PathBuf::from).unwrap_or_else(|| {
        let id: String = client_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        std::env::temp_dir().join(format!("lavatui-live-{id}.json"))
    })
}

#[test]
fn each_client_id_gets_its_own_token_file_unless_one_is_set() {
    let a = token_file_from(None, "abc123");
    let b = token_file_from(None, "def456");
    assert_ne!(a, b);
    assert!(a.starts_with(std::env::temp_dir()), "{a:?}");
    // Nothing odd in a Client ID reaches the path.
    let odd = token_file_from(None, "../x y");
    assert_eq!(odd.file_name().unwrap(), "lavatui-live-xy.json");
    let set = token_file_from(Some("/somewhere/tokens.json".into()), "abc123");
    assert_eq!(set, PathBuf::from("/somewhere/tokens.json"));
}

/// Stops a test that would change the account unless that was asked for.
fn consent(what: &str) {
    assert!(
        env(CHANGES_ENV).as_deref() == Some("1"),
        "this test {what}: set {CHANGES_ENV}=1 to allow it"
    );
}

/// A logged-in client on the real Web API.
struct Live {
    web: SpotifyWeb,
}

impl Live {
    fn new() -> Self {
        let client_id = client_id_from_env().expect("set LAVATUI_SPOTIFY_CLIENT_ID");
        let file = token_file(&client_id);
        eprintln!("tokens: {}", file.display());
        let id = client_id.clone();
        let web = SpotifyWeb::spawn(client_id, false, move || {
            Client::new(super::http::Ureq::new(), id, Box::new(FileStore(file)))
        });
        Self { web }
    }

    /// `request`'s answer (the worker loads the saved login first; a
    /// request is the barrier).
    fn try_ask(&mut self, request: Request) -> Result<Reply, Error> {
        let want = self.web.request(request.clone());
        loop {
            match self.web.poll_timeout(Duration::from_secs(60)) {
                Some(Event::Reply { id, result }) if id == want => return result,
                Some(other) => eprintln!("event: {other:?}"),
                None => panic!("{request:?}: no reply"),
            }
        }
    }

    fn ask(&mut self, request: Request) -> Reply {
        self.try_ask(request.clone())
            .unwrap_or_else(|e| panic!("{request:?}: {e}"))
    }

    fn login(&mut self) {
        let url = self.web.login().expect("login");
        eprintln!("LOGIN: finish in the browser (opened): {url}");
        match self.web.poll_timeout(Duration::from_secs(330)) {
            Some(Event::LoggedIn { saved }) => eprintln!("logged in (saved: {saved})"),
            other => panic!("login: {other:?}"),
        }
    }

    /// Who's logged in, logging in first if no one is. A login without the
    /// playback scopes (made before they were added) logs in again.
    fn me(&mut self) -> User {
        let me = match self.try_ask(Request::Me) {
            Ok(Reply::User(me)) => me,
            _ => {
                self.login();
                let Reply::User(me) = self.ask(Request::Me) else {
                    panic!()
                };
                me
            }
        };
        if matches!(self.try_ask(Request::Player), Err(Error::Forbidden(why)) if why.contains("scope"))
        {
            eprintln!("the login lacks the playback scopes: logging in again");
            self.web.logout();
            let _ = self.web.poll_timeout(Duration::from_secs(5));
            self.login();
        }
        eprintln!("me: {} ({})", me.name(), me.id);
        me
    }

    /// The account's player (any device), if it has one and may say.
    fn player(&mut self) -> Option<PlayerState> {
        match self.try_ask(Request::Player) {
            Ok(Reply::Player(state)) => state,
            other => {
                eprintln!("player: {other:?}");
                None
            }
        }
    }

    /// The track to like and add: `LAVATUI_TEST_TRACK`, else what the
    /// account is playing.
    fn test_track(&mut self) -> Option<String> {
        if let Some(uri) = env(TRACK_ENV) {
            assert!(uri.starts_with("spotify:track:"), "{TRACK_ENV}: {uri}");
            return Some(uri);
        }
        let playing = self.player()?.item?.uri;
        eprintln!("playing: {playing}");
        playing.starts_with("spotify:track:").then_some(playing)
    }

    /// The test playlist's id: `LAVATUI_TEST_PLAYLIST`, else the user's
    /// own "lavatui test".
    fn test_playlist(&self, lists: &[Playlist], me: &User) -> Option<String> {
        env(PLAYLIST_ENV).or_else(|| {
            lists
                .iter()
                .find(|p| p.name == TEST_PLAYLIST && p.owner_id == me.id)
                .map(|p| p.id.clone())
        })
    }

    fn liked(&mut self, uri: &str) -> bool {
        let uris = vec![uri.to_owned()];
        let Reply::Contains(v) = self.ask(Request::LibraryContains { uris }) else {
            panic!()
        };
        v[0]
    }

    /// Like then unlike `uri` (or the other way round), back as it was.
    fn flip_like(&mut self, uri: &str) {
        let uris = vec![uri.to_owned()];
        let original = self.liked(uri);
        let (first, back) = if original {
            (
                Request::Unlike { uris: uris.clone() },
                Request::Like { uris },
            )
        } else {
            (
                Request::Like { uris: uris.clone() },
                Request::Unlike { uris },
            )
        };
        self.ask(first);
        let flipped = self.liked(uri);
        self.ask(back);
        let restored = self.liked(uri);
        eprintln!("liked: {original} -> {flipped} -> {restored}");
        assert_eq!((flipped, restored), (!original, original));
    }
}

/// The real transport against Spotify's token endpoint with a made-up
/// Client ID: TLS works and the refusal maps to a login error. Changes
/// nothing, needs no account.
/// `cargo test -- --ignored live_token_endpoint`
#[test]
#[ignore = "network"]
fn live_token_endpoint_refuses_a_bogus_client() {
    let mut client = Client::new(
        super::http::Ureq::new(),
        "lavatui-test-bogus-client".into(),
        Box::new(MemoryStore::default()),
    );
    let result = client.exchange_code("bogus-code", "bogus-verifier-bogus-verifier-bogus-verifier");
    assert!(matches!(result, Err(Error::Login(_))), "{result:?}");
    eprintln!("{result:?}");
}

/// The whole thing against a real account: reads the profile, playlists
/// and one owned playlist; likes/unlikes the test track (else the first
/// one read) and restores its liked state; adds it to the test playlist
/// (creating a private "lavatui test" when there's none). Never touches
/// any other playlist. Needs `LAVATUI_LIVE_CHANGES=1`.
#[test]
#[ignore = "needs a Spotify account and a browser"]
fn live_account() {
    let mut live = Live::new();
    let me = live.me();
    let Reply::Playlists(lists) = live.ask(Request::MyPlaylists) else {
        panic!()
    };
    eprintln!("playlists: {}", lists.len());

    let mut sample = None;
    if let Some(owned) = lists
        .iter()
        .find(|p| p.editable_by(&me) && p.total > 0 && p.name != TEST_PLAYLIST)
    {
        let Reply::Tracks(page) = live.ask(Request::PlaylistTracks {
            playlist_id: owned.id.clone(),
            offset: 0,
        }) else {
            panic!()
        };
        eprintln!(
            "playlist {:?}: {} of {} items read",
            owned.name,
            page.items.len(),
            page.total,
        );
        sample = page
            .items
            .into_iter()
            .find(|t| t.uri.starts_with("spotify:track:"))
            .map(|t| t.uri);
    } else {
        eprintln!("no owned, non-empty playlist to read");
    }
    let uri = live
        .test_track()
        .or(sample)
        .unwrap_or_else(|| panic!("no track to like: play one, or set {TRACK_ENV}"));
    eprintln!("test track: {uri}");

    consent("likes, unlikes and adds to a playlist");
    live.flip_like(&uri);

    let test_id = match live.test_playlist(&lists, &me) {
        Some(id) => id,
        None => {
            let Reply::Playlist(p) = live.ask(Request::CreatePlaylist {
                name: TEST_PLAYLIST.into(),
                public: false,
            }) else {
                panic!()
            };
            eprintln!("created private playlist {:?} ({})", p.name, p.id);
            p.id
        }
    };
    let Reply::Snapshot(snapshot) = live.ask(Request::AddToPlaylist {
        playlist_id: test_id.clone(),
        uris: vec![uri.clone()],
    }) else {
        panic!()
    };
    let Reply::Tracks(page) = live.ask(Request::PlaylistTracks {
        playlist_id: test_id,
        offset: 0,
    }) else {
        panic!()
    };
    eprintln!(
        "added (snapshot {snapshot}); the test playlist has {} items",
        page.total
    );
    assert!(page.items.iter().any(|t| t.uri == uri));
}

/// The library UI's calls against a real account: reads the playlists, one
/// owned playlist's first page and a followed one's; likes / unlikes the
/// test track (restoring it), adds it to the test playlist and nowhere
/// else, then reads the player and flips shuffle and repeat for a moment,
/// putting both back. Needs `LAVATUI_LIVE_CHANGES=1`, a test track (or
/// something playing) and the test playlist.
#[test]
#[ignore = "needs a Spotify account and a browser"]
fn live_library() {
    let mut live = Live::new();
    let me = live.me();
    let Reply::Playlists(lists) = live.ask(Request::MyPlaylists) else {
        panic!()
    };
    let editable = lists.iter().filter(|p| p.editable_by(&me)).count();
    eprintln!("playlists: {} ({editable} editable)", lists.len());
    let test_id = live
        .test_playlist(&lists, &me)
        .unwrap_or_else(|| panic!("no test playlist: set {PLAYLIST_ENV} or run live_account once"));
    if let Some(owned) = lists
        .iter()
        .find(|p| p.editable_by(&me) && p.total > 0 && p.id != test_id)
    {
        let Reply::Tracks(page) = live.ask(Request::PlaylistTracks {
            playlist_id: owned.id.clone(),
            offset: 0,
        }) else {
            panic!()
        };
        eprintln!(
            "{:?}: {} of {} read, more: {}",
            owned.name,
            page.items.len(),
            page.total,
            page.has_more
        );
    }
    if let Some(followed) = lists.iter().find(|p| !p.editable_by(&me)) {
        let r = live.try_ask(Request::PlaylistTracks {
            playlist_id: followed.id.clone(),
            offset: 0,
        });
        eprintln!("followed {:?} items: {:?}", followed.name, r.map(|_| "ok"));
    }
    let uri = live
        .test_track()
        .unwrap_or_else(|| panic!("play a track in Spotify first, or set {TRACK_ENV}"));

    consent("likes, unlikes, adds to the test playlist and flips shuffle / repeat");
    live.flip_like(&uri);
    let Reply::Snapshot(snap) = live.ask(Request::AddToPlaylist {
        playlist_id: test_id.clone(),
        uris: vec![uri],
    }) else {
        panic!()
    };
    eprintln!("added to the test playlist ({test_id}), snapshot {snap}");

    let Some(state) = live.player() else {
        eprintln!("no device playing (or no Premium): shuffle / repeat not tried");
        return;
    };
    eprintln!("player: {state:?}");
    if state.shuffle_blocked || state.repeat_blocked {
        eprintln!("(this context blocks some toggles: refusals expected)");
    }
    let shuffle = live.try_ask(Request::SetShuffle(!state.shuffle));
    eprintln!("shuffle -> {}: {shuffle:?}", !state.shuffle);
    let repeat = if state.repeat == Repeat::Off {
        Repeat::Context
    } else {
        Repeat::Off
    };
    let r = live.try_ask(Request::SetRepeat(repeat));
    eprintln!("repeat -> {repeat:?}: {r:?}");
    std::thread::sleep(Duration::from_millis(800));
    if let Some(now) = live.player() {
        eprintln!("read back: shuffle {} repeat {:?}", now.shuffle, now.repeat);
    }
    let a = live.try_ask(Request::SetShuffle(state.shuffle));
    let b = live.try_ask(Request::SetRepeat(state.repeat));
    std::thread::sleep(Duration::from_millis(800));
    let after = live.player().expect("the player again");
    eprintln!(
        "restored ({a:?}, {b:?}): shuffle {} repeat {:?}",
        after.shuffle, after.repeat
    );
    if shuffle.is_ok() {
        assert_eq!((after.shuffle, after.repeat), (state.shuffle, state.repeat));
    }
}

/// Read-only: the test playlist's items and whether the test track is
/// liked (to check what a UI run did). Changes nothing.
#[test]
#[ignore = "needs a Spotify account"]
fn live_peek() {
    let mut live = Live::new();
    let me = live.me();
    let Reply::Playlists(lists) = live.ask(Request::MyPlaylists) else {
        panic!()
    };
    if let Some(id) = live.test_playlist(&lists, &me)
        && let Ok(Reply::Tracks(page)) = live.try_ask(Request::PlaylistTracks {
            playlist_id: id,
            offset: 0,
        })
    {
        for t in &page.items {
            eprintln!(
                "in the test playlist: {} – {} ({})",
                t.name,
                t.artist_line(),
                t.uri
            );
        }
    }
    if let Some(uri) = live.test_track() {
        let liked = live.liked(&uri);
        eprintln!("{uri} liked: {liked}");
    }
}

/// `live_library`'s player part in a playlist context, where Spotify
/// allows the toggles: plays the test playlist for a few seconds through
/// `PUT /me/player/play`, flips shuffle and repeat and puts them back,
/// then has the desktop app play the original track from where it was
/// (paused again if it was: AppleScript, so macOS only). Needs
/// `LAVATUI_LIVE_CHANGES=1` and Premium.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs a Spotify account; changes playback for a few seconds"]
fn live_player_in_a_playlist() {
    let osa = |script: &str| -> String {
        let out = std::process::Command::new("osascript")
            .args(["-e", script])
            .output()
            .expect("osascript");
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let before = osa(
        "if application \"Spotify\" is running then tell application \"Spotify\" to return (id of current track) & \"|\" & (player position as string) & \"|\" & (player state as string)",
    );
    let mut parts = before.split('|');
    let (track, position, state) = (
        parts.next().unwrap_or("").to_owned(),
        parts.next().unwrap_or("0").replace(',', "."),
        parts.next().unwrap_or("").to_owned(),
    );
    assert!(
        track.starts_with("spotify:"),
        "Spotify must be running: {before:?}"
    );
    eprintln!("before: {track} at {position}s, {state}");

    let mut live = Live::new();
    let me = live.me();
    let Reply::Playlists(lists) = live.ask(Request::MyPlaylists) else {
        panic!()
    };
    let playlist = live
        .test_playlist(&lists, &me)
        .unwrap_or_else(|| panic!("no test playlist: set {PLAYLIST_ENV}"));
    let original = live.player().expect("a device playing (Premium)");
    consent("plays the test playlist for a few seconds");
    let played = live.try_ask(Request::Play {
        context_uri: format!("spotify:playlist:{playlist}"),
        offset_uri: None,
    });
    eprintln!("play the test playlist: {played:?}");
    std::thread::sleep(Duration::from_millis(1500));
    let p = live.player().expect("the player");
    eprintln!("in the playlist: {p:?}");
    let s = live.try_ask(Request::SetShuffle(!p.shuffle));
    let r = live.try_ask(Request::SetRepeat(if p.repeat == Repeat::Off {
        Repeat::Context
    } else {
        Repeat::Off
    }));
    std::thread::sleep(Duration::from_millis(800));
    let flipped = live.player().expect("the player");
    eprintln!(
        "flip: {s:?} {r:?} -> shuffle {} repeat {:?}",
        flipped.shuffle, flipped.repeat
    );
    let _ = live.try_ask(Request::SetShuffle(original.shuffle));
    let _ = live.try_ask(Request::SetRepeat(original.repeat));

    // Put the desktop app back where it was.
    osa(&format!(
        "tell application \"Spotify\"\nplay track \"{track}\"\ndelay 0.8\nset player position to {position}\nend tell"
    ));
    if state != "playing" {
        osa("tell application \"Spotify\" to pause");
    }
    std::thread::sleep(Duration::from_millis(800));
    let after = live.player().expect("the player");
    let now = osa(
        "tell application \"Spotify\" to return (id of current track) & \"|\" & (player position as string) & \"|\" & (player state as string)",
    );
    eprintln!(
        "after: {now}; shuffle {} repeat {:?} (were {} {:?})",
        after.shuffle, after.repeat, original.shuffle, original.repeat
    );
    assert!(s.is_ok() && r.is_ok(), "toggles refused in a playlist");
    assert_ne!((flipped.shuffle, flipped.repeat), (p.shuffle, p.repeat));
    assert_eq!(
        (after.shuffle, after.repeat),
        (original.shuffle, original.repeat)
    );
    assert!(now.starts_with(&track));
}

/// lava-75z.18 against the real desktop app: the AppleScript backend plays
/// the second track of one of your own playlists in that playlist (read
/// back through the Web API: the item and the context), then puts back
/// what was playing (its track in its context, the position, paused if it
/// was). Reads playlists only; never changes one.
/// Needs `LAVATUI_LIVE_CHANGES=1`.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "needs a Spotify account and the desktop app playing"]
fn live_play_in_context() {
    use crate::media::runner::Osascript;
    use crate::media::spotify::{Spotify, script};
    use crate::media::worker::Backend;
    use crate::media::{Command, Status};

    let mut live = Live::new();
    let me = live.me();
    let mut ask = |req: Request| live.ask(req);
    let read = |ask: &mut dyn FnMut(Request) -> Reply| match ask(Request::Player) {
        Reply::Player(Some(p)) => p,
        other => panic!("player: {other:?}"),
    };
    let mut app = Spotify::new(Osascript::new(script()));
    let before = app.exchange(&[]);
    let original = read(&mut ask);
    let was = before
        .track
        .clone()
        .expect("Spotify must have a track loaded");
    eprintln!(
        "before: {} in {:?} at {:?}, {:?}",
        was.id, original.context_uri, before.position, before.status
    );

    let Reply::Playlists(lists) = ask(Request::MyPlaylists) else {
        panic!()
    };
    let (playlist, track) = lists
        .iter()
        .filter(|p| p.editable_by(&me) && p.total >= 3)
        .filter(|p| original.context_uri.as_deref() != Some(p.uri.as_str()))
        .find_map(|p| {
            let Reply::Tracks(page) = ask(Request::PlaylistTracks {
                playlist_id: p.id.clone(),
                offset: 0,
            }) else {
                return None;
            };
            let t = page.items.get(1).filter(|t| !t.is_local)?;
            Some((p.clone(), t.clone()))
        })
        .expect("an own playlist with 3+ tracks");
    eprintln!("play {} ({}) in {}", track.name, track.uri, playlist.name);
    consent("plays one of your playlists for a few seconds");

    let command = Command::play_in_context(&track.uri, &playlist.uri).unwrap();
    let during = app.exchange(&[command]);
    std::thread::sleep(Duration::from_millis(2000));
    let p = read(&mut ask);
    eprintln!(
        "playing: {:?} in {:?} (desktop app: {:?})",
        p.item_uri(),
        p.context_uri,
        during.track.as_ref().map(|t| &t.id)
    );

    // Put it back.
    let back = match &original.context_uri {
        Some(context) => Command::play_in_context(&was.id, context),
        None => Command::play_uri(&was.id),
    };
    app.exchange(&[back.expect("the old track's uri")]);
    std::thread::sleep(Duration::from_millis(800));
    let mut restore = vec![Command::Seek(before.position)];
    if before.status != Status::Playing {
        restore.push(Command::PlayPause);
    }
    app.exchange(&restore);
    std::thread::sleep(Duration::from_millis(800));
    let after = app.exchange(&[]);
    let restored = read(&mut ask);
    eprintln!(
        "after: {:?} in {:?} at {:?}, {:?}",
        after.track.as_ref().map(|t| &t.id),
        restored.context_uri,
        after.position,
        after.status
    );

    assert_eq!(p.item_uri(), Some(track.uri.as_str()));
    assert_eq!(p.context_uri.as_deref(), Some(playlist.uri.as_str()));
    assert_eq!(after.track.map(|t| t.id.clone()), Some(was.id.clone()));
    assert_eq!(
        after.status == Status::Playing,
        before.status == Status::Playing
    );
}
