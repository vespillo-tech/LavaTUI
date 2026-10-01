//! The Spotify library flows against `FakeWeb` (an in-memory account) and
//! `FakeSource` (the desktop app): login / logout, like, the playlist
//! browser, add to playlist, shuffle / repeat through the Web API.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::media::{Capabilities, Command, FakeSource, Snapshot, Status as Play, Track};
use crate::spotify_web::fake::{FakeWeb, demo};
use crate::spotify_web::{Error, PlayerState, Repeat, Request, Web};
use crate::ui::keymap::{Action, PlayerKey as P};

const PLAYING: &str = "spotify:track:t0";

fn local() -> LocalTime {
    LocalTime {
        time: ClockTime::new(14, 32, 7).unwrap(),
        date: "thu 1 oct".into(),
        wall: SystemTime::UNIX_EPOCH,
    }
}

fn temp_config(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lavatui-library-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("config.toml")
}

/// A model with music beside the lamp playing [`PLAYING`] on a fake desktop
/// app, the demo account plugged in, and the player keys on.
fn rig(name: &str, account: &FakeWeb) -> (Model, Instant, FakeSource) {
    let t0 = Instant::now();
    let mut m = Model::new(
        &Session::default(),
        Store::new(Some(temp_config(name))),
        Rect::new(0, 0, 100, 30),
        None,
        local(),
        1,
        t0,
    );
    let source = FakeSource::new(
        Snapshot {
            player: Some("Spotify".into()),
            track: Some(Arc::new(Track {
                id: PLAYING.into(),
                name: "Slow Rise 0".into(),
                artist: "Wax & Wane".into(),
                album: "Lamplight".into(),
                duration: Duration::from_secs(200),
                artwork_url: String::new(),
            })),
            volume: 50,
            ..Snapshot::new(Play::Playing, t0)
        },
        Vec::new(),
    );
    let s = source.clone();
    m.music.connect_with(
        move || Box::new(s.clone()),
        || {
            crate::media::art::ArtLoader::preloaded(
                "",
                crate::media::art::Art::solid(crate::theme::Rgb(0, 0, 0)),
            )
        },
    );
    let a = account.clone();
    m.library
        .connect_with(move || Some(Box::new(a.clone()) as Box<dyn Web>));
    m.update(Action::Place("music"), t0);
    tick(&mut m, t0);
    m.update(Action::PlayerKeys, t0);
    (m, t0, source)
}

fn tick(m: &mut Model, at: Instant) {
    let area = m.layout.area;
    m.tick(at, area, local());
}

fn toast(m: &Model) -> String {
    m.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default()
}

fn key(m: &mut Model, t: Instant, k: P) {
    m.update(Action::Player(k), t);
}

#[test]
fn no_client_id_says_so() {
    let t0 = Instant::now();
    let (mut m, _, _) = rig("no-id", &demo());
    m.library.connect_with(|| None);
    tick(&mut m, t0);
    key(&mut m, t0, P::Playlists);
    assert_eq!(m.overlay, Overlay::None);
    assert!(toast(&m).contains("Client ID"), "{}", toast(&m));
    assert_eq!(m.library.account(), Account::Unavailable);
}

#[test]
fn login_then_logout_twice() {
    let account = demo();
    account.state().logged_in = false;
    let (mut m, t0, _) = rig("login", &account);
    assert_eq!(m.library.account(), Account::LoggedOut);
    key(&mut m, t0, P::Account);
    assert_eq!(m.library.account(), Account::LoggingIn);
    assert!(account.state().login_pending);
    assert_eq!(toast(&m), "log in to Spotify in your browser");

    account.finish_login();
    tick(&mut m, t0);
    assert_eq!(m.library.account(), Account::LoggedIn);
    assert_eq!(toast(&m), "logged in to Spotify");
    tick(&mut m, t0);
    assert_eq!(m.library.me.as_ref().map(|u| u.id.as_str()), Some("me"));

    key(&mut m, t0, P::Account);
    assert_eq!(toast(&m), "press i again to log out of Spotify");
    assert_eq!(m.library.account(), Account::LoggedIn);
    key(&mut m, t0 + Duration::from_millis(500), P::Account);
    assert_eq!(m.library.account(), Account::LoggedOut);
    assert_eq!(toast(&m), "logged out of Spotify");
    assert!(m.library.me.is_none());
}

#[test]
fn a_pending_login_cancels() {
    let account = demo();
    account.state().logged_in = false;
    let (mut m, t0, _) = rig("login-cancel", &account);
    key(&mut m, t0, P::Account);
    key(&mut m, t0, P::Account);
    assert_eq!(m.library.account(), Account::LoggedOut);
    assert!(!account.state().login_pending);
    assert_eq!(toast(&m), "login cancelled");
}

#[test]
fn like_and_unlike_show_in_the_heart_at_once() {
    let account = demo();
    let (mut m, t0, _) = rig("like", &account);
    tick(&mut m, t0);
    assert_eq!(m.liked(), Some(false));
    key(&mut m, t0, P::Like);
    assert_eq!(m.liked(), Some(true), "optimistic");
    assert_eq!(toast(&m), "♥ liked");
    assert!(account.state().liked.contains(PLAYING));
    key(&mut m, t0, P::Like);
    assert_eq!(m.liked(), Some(false));
    assert!(!account.state().liked.contains(PLAYING));
    // A refusal puts the heart back.
    account.state().fail = Some(Error::Offline("down".into()));
    account.state().hold = true;
    key(&mut m, t0, P::Like);
    assert_eq!(m.liked(), Some(true));
    account.release();
    tick(&mut m, t0);
    assert_eq!(m.liked(), Some(false));
    assert!(toast(&m).contains("can't reach"), "{}", toast(&m));
}

#[test]
fn the_browser_opens_owned_playlists_and_plays_the_rest() {
    let account = demo();
    let (mut m, t0, source) = rig("browse", &account);
    key(&mut m, t0, P::Playlists);
    assert_eq!(m.input_mode(), InputMode::Library { inline: false });
    assert_eq!(m.list_len(ListKind::Playlists), 4);

    // Discover Weekly isn't readable: ⏎ plays it, the browser stays.
    m.update(Action::Down, t0);
    m.update(Action::Down, t0);
    assert!(m.list_row(ListKind::Playlists, 2).unwrap().quiet);
    m.update(Action::Keep, t0);
    assert_eq!(
        source.sent().last(),
        Some(&Command::PlayUri("spotify:playlist:dw".into())),
        "{}",
        toast(&m)
    );
    assert!(matches!(m.overlay, Overlay::Library(v) if v.kind == ListKind::Playlists));

    // Lamplight Mix: ⏎ opens it, a page at a time.
    m.update(Action::Edge(false), t0);
    m.update(Action::Keep, t0);
    let Overlay::Library(view) = m.overlay else {
        panic!("{:?}", m.overlay)
    };
    assert_eq!(view.kind, ListKind::Tracks);
    assert_eq!(m.list_title(&view), "Lamplight Mix");
    assert_eq!(m.list_len(ListKind::Tracks), 50);
    m.update(Action::Edge(true), t0);
    assert_eq!(m.list_len(ListKind::Tracks), 60, "the next page loads");

    // ⏎ plays the track (no Web API player here: the track alone).
    m.update(Action::Keep, t0);
    assert_eq!(
        source.sent().last(),
        Some(&Command::PlayUri("spotify:track:t49".into()))
    );
    // `p` plays the playlist from the top.
    m.update(Action::PlayAll, t0);
    assert_eq!(
        source.sent().last(),
        Some(&Command::PlayUri("spotify:playlist:mix".into()))
    );
    // esc goes back to the playlists, where it was; esc again closes.
    m.update(Action::Back, t0);
    let Overlay::Library(view) = m.overlay else {
        panic!()
    };
    assert_eq!((view.kind, view.cursor), (ListKind::Playlists, 0));
    m.update(Action::Back, t0);
    assert_eq!(m.overlay, Overlay::None);
    assert_eq!(m.input_mode(), InputMode::Player, "back to the player keys");
}

#[test]
fn add_to_playlist_offers_only_editable_ones() {
    let account = demo();
    let (mut m, t0, _) = rig("add", &account);
    key(&mut m, t0, P::AddToPlaylist);
    let names: Vec<String> = (0..m.list_len(ListKind::AddTo))
        .map(|i| m.list_row(ListKind::AddTo, i).unwrap().name)
        .collect();
    assert_eq!(names, ["Lamplight Mix", "lavatui test", "Shared Jams"]);
    m.update(Action::Down, t0);
    m.update(Action::Keep, t0);
    assert_eq!(m.overlay, Overlay::None);
    tick(&mut m, t0);
    assert_eq!(toast(&m), "added to lavatui test");
    assert!(
        account.state().tracks["test"]
            .iter()
            .any(|t| t.uri == PLAYING)
    );
    assert_eq!(
        account.state().tracks["mix"].len(),
        60,
        "no other playlist changes"
    );
}

#[test]
fn logged_out_the_browser_offers_the_login() {
    let account = demo();
    account.state().logged_in = false;
    let (mut m, t0, _) = rig("browse-logged-out", &account);
    key(&mut m, t0, P::Playlists);
    assert_eq!(m.list_len(ListKind::Playlists), 0);
    assert_eq!(
        m.list_message(ListKind::Playlists),
        "not logged in · ⏎ to log in"
    );
    m.update(Action::Keep, t0);
    assert_eq!(m.library.account(), Account::LoggingIn);
    account.finish_login();
    tick(&mut m, t0);
    // Logged in with the browser open: reopening lists them.
    m.update(Action::Close, t0);
    key(&mut m, t0, P::Playlists);
    assert_eq!(m.list_len(ListKind::Playlists), 4);
}

fn premium(shuffle: bool) -> PlayerState {
    PlayerState {
        shuffle,
        repeat: Repeat::Off,
        is_playing: true,
        device: Some("Mac".into()),
        item_uri: Some(PLAYING.into()),
        shuffle_blocked: false,
        repeat_blocked: false,
    }
}

#[test]
fn shuffle_and_repeat_go_through_the_web_api_when_allowed() {
    let account = demo();
    account.state().player = Ok(Some(premium(false)));
    let (mut m, t0, source) = rig("web-modes", &account);
    source.set_capabilities(Capabilities::NONE);
    tick(&mut m, t0);
    assert_eq!(m.music.capabilities(), Capabilities::ALL);
    key(&mut m, t0, P::Shuffle);
    key(&mut m, t0, P::Repeat);
    assert!(source.sent().is_empty(), "nothing goes to the desktop app");
    let reqs = account.state().requests.clone();
    assert!(reqs.contains(&Request::SetShuffle(true)), "{reqs:?}");
    assert!(
        reqs.contains(&Request::SetRepeat(Repeat::Context)),
        "{reqs:?}"
    );
    tick(&mut m, t0);
    let snap = m.music.snapshot.as_ref().unwrap();
    assert!(snap.shuffle && snap.repeat, "the widget shows it");

    // Exact plays in a playlist with the Web API's player.
    key(&mut m, t0, P::Playlists);
    m.update(Action::Keep, t0);
    m.update(Action::Down, t0);
    m.update(Action::Keep, t0);
    let play = Request::Play {
        context_uri: "spotify:playlist:mix".into(),
        offset_uri: Some("spotify:track:t1".into()),
    };
    assert!(account.state().requests.contains(&play));
    m.update(Action::Close, t0);

    // Spotify refuses (no Premium): they're hidden again.
    account.state().player = Err(Error::Forbidden("Premium required".into()));
    key(&mut m, t0, P::Shuffle);
    tick(&mut m, t0);
    assert_eq!(m.music.capabilities(), Capabilities::NONE);
    assert!(toast(&m).contains("Premium"), "{}", toast(&m));
    key(&mut m, t0, P::Shuffle);
    assert!(toast(&m).contains("can't shuffle"), "{}", toast(&m));
}

#[test]
fn a_context_that_blocks_toggles_hides_them_but_keeps_the_login() {
    let account = demo();
    let blocked = PlayerState {
        shuffle_blocked: true,
        ..premium(false)
    };
    account.state().player = Ok(Some(blocked));
    let (mut m, t0, source) = rig("web-modes-blocked", &account);
    source.set_capabilities(Capabilities::NONE);
    tick(&mut m, t0);
    let caps = m.music.capabilities();
    assert!(!caps.shuffle && caps.repeat, "{caps:?}");
    key(&mut m, t0, P::Shuffle);
    assert!(toast(&m).contains("won't change"), "{}", toast(&m));
    assert!(
        !account
            .state()
            .requests
            .contains(&Request::SetShuffle(true))
    );
    // A momentary refusal ("Restriction violated") isn't the account's.
    account.state().fail = Some(Error::Forbidden(
        "Player command failed: Restriction violated".into(),
    ));
    key(&mut m, t0, P::Repeat);
    tick(&mut m, t0);
    assert!(toast(&m).contains("won't change"), "{}", toast(&m));
    assert_eq!(m.library.player.allowed, Some(true));
}

#[test]
fn without_premium_or_a_device_the_modes_stay_hidden() {
    let account = demo();
    let (mut m, t0, source) = rig("web-modes-none", &account);
    source.set_capabilities(Capabilities::NONE);
    tick(&mut m, t0);
    assert_eq!(m.music.capabilities(), Capabilities::NONE, "no device");
    account.state().player = Err(Error::Forbidden("Insufficient client scope".into()));
    let later = t0 + Duration::from_secs(31);
    tick(&mut m, later);
    tick(&mut m, later);
    assert_eq!(m.library.player.allowed, Some(false));
    key(&mut m, t0, P::Shuffle);
    assert!(
        toast(&m).contains("log in to Spotify again"),
        "{}",
        toast(&m)
    );
}

#[test]
fn a_frozen_lamp_wakes_soon_while_spotify_is_answering() {
    let account = demo();
    let (mut m, t0, _) = rig("idle", &account);
    m.frozen = true;
    m.toast = None;
    account.state().hold = true;
    key(&mut m, t0, P::Playlists);
    m.toast = None;
    let wake = m.idle_until().unwrap();
    assert!(wake <= t0 + Duration::from_millis(200), "{:?}", wake - t0);
}
