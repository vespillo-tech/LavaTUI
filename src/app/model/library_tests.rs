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
    let track = Track {
        id: PLAYING.into(),
        uri: Some(PLAYING.into()),
        name: "Slow Rise 0".into(),
        artist: "Wax & Wane".into(),
        album: "Lamplight".into(),
        duration: Duration::from_secs(200),
        artwork_url: String::new(),
    };
    rig_on(
        name,
        account,
        desktop("Spotify", track, t0),
        Capabilities::ALL,
    )
}

/// The desktop player `player` playing `track` (as its backend reports it).
fn desktop(player: &str, track: Track, t0: Instant) -> Snapshot {
    Snapshot {
        player: Some(player.into()),
        track: Some(Arc::new(track)),
        volume: 50,
        ..Snapshot::new(Play::Playing, t0)
    }
}

/// [`rig`] on any desktop player `snapshot`, its controls `caps`.
fn rig_on(
    name: &str,
    account: &FakeWeb,
    snapshot: Snapshot,
    caps: Capabilities,
) -> (Model, Instant, FakeSource) {
    let t0 = snapshot.sampled_at;
    let mut m = Model::new(
        &Session::default(),
        Store::new(Some(temp_config(name))),
        Rect::new(0, 0, 100, 30),
        None,
        local(),
        1,
        t0,
    );
    m.welcome = false;
    let source = FakeSource::new(snapshot, Vec::new());
    source.set_capabilities(caps);
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
    // The guided setup opens instead (lava-1xk.5).
    let view = m.settings_view().expect("the settings screen");
    assert_eq!(view.page, crate::app::Page::Spotify);
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
    assert_eq!(
        m.input_mode(),
        InputMode::Library {
            inline: false,
            typing: false
        }
    );
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

    // ⏎ plays the track in its playlist (no Web API player here: the
    // desktop app does it).
    m.update(Action::Keep, t0);
    assert_eq!(
        source.sent().last(),
        Some(&Command::PlayInContext {
            track: "spotify:track:t49".into(),
            context: "spotify:playlist:mix".into(),
        })
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
        "not logged in · Enter to log in"
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
        item_name: Some("Slow Rise 0".into()),
        context_uri: None,
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
    let caps = m.music.capabilities();
    assert!(caps.shuffle && caps.repeat, "{caps:?}");
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
fn a_refused_web_play_falls_back_to_the_desktop_app_in_context() {
    let account = demo();
    account.state().player = Ok(Some(premium(false)));
    let (mut m, t0, source) = rig("web-play-refused", &account);
    tick(&mut m, t0);
    key(&mut m, t0, P::Playlists);
    m.update(Action::Keep, t0);
    m.update(Action::Down, t0);
    account.state().fail = Some(Error::Forbidden("Premium required".into()));
    m.update(Action::Keep, t0);
    tick(&mut m, t0);
    assert_eq!(
        source.sent().last(),
        Some(&Command::PlayInContext {
            track: "spotify:track:t1".into(),
            context: "spotify:playlist:mix".into(),
        })
    );
    assert_eq!(m.library.player.allowed, Some(false));
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

fn view(m: &Model) -> ListView {
    let Overlay::Library(view) = m.overlay else {
        panic!("{:?}", m.overlay)
    };
    view
}

fn type_in(m: &mut Model, t: Instant, text: &str) {
    for c in text.chars() {
        m.update(Action::Type(c), t);
    }
}

fn names(m: &Model, kind: ListKind) -> Vec<String> {
    (0..m.list_len(kind))
        .map(|i| m.list_row(kind, i).unwrap().name)
        .collect()
}

#[test]
fn slash_filters_the_playlists_and_back_keeps_the_filter() {
    let account = demo();
    let (mut m, t0, _) = rig("find-playlists", &account);
    key(&mut m, t0, P::Playlists);
    // Letters are keys until `/`.
    m.update(Action::Type('j'), t0);
    assert_eq!(m.list_len(ListKind::Playlists), 4);
    m.update(Action::Find, t0);
    assert!(view(&m).typing);
    assert_eq!(
        m.input_mode(),
        InputMode::Library {
            inline: false,
            typing: true
        }
    );
    // Case and word order don't matter; every word must be there.
    type_in(&mut m, t0, "MIX lamp");
    assert_eq!(names(&m, ListKind::Playlists), ["Lamplight Mix"]);
    m.update(Action::Erase, t0);
    m.update(Action::Erase, t0);
    m.update(Action::Erase, t0);
    m.update(Action::Erase, t0);
    m.update(Action::Erase, t0);
    assert_eq!(m.library.find.text, "MIX");
    m.update(Action::ClearFind, t0);
    type_in(&mut m, t0, "/");
    assert!(!view(&m).typing, "esc closed the filter");
    m.update(Action::Find, t0);
    type_in(&mut m, t0, "jam");
    assert_eq!(names(&m, ListKind::Playlists), ["Shared Jams"]);

    // ⏎ opens the match (not the first playlist); its tracks start
    // unfiltered.
    m.update(Action::Keep, t0);
    assert_eq!(view(&m).kind, ListKind::Tracks);
    assert!(!view(&m).typing);
    assert_eq!(m.list_title(&view(&m)), "Shared Jams");
    assert_eq!(names(&m, ListKind::Tracks), ["Convection"]);
    // esc: back to the filtered playlists, on the one we came from.
    m.update(Action::Back, t0);
    let back = view(&m);
    assert_eq!(back.kind, ListKind::Playlists);
    assert!(back.typing);
    assert_eq!(m.library.find.text, "jam");
    assert_eq!(
        m.list_row(ListKind::Playlists, back.cursor).unwrap().name,
        "Shared Jams"
    );

    // Nothing matches: it says so. Backspace on nothing closes the filter.
    type_in(&mut m, t0, "zz");
    assert_eq!(m.list_len(ListKind::Playlists), 0);
    assert_eq!(
        m.list_message(ListKind::Playlists),
        "nothing matches “jamzz”"
    );
    for _ in 0..6 {
        m.update(Action::Erase, t0);
    }
    assert!(!view(&m).typing);
    assert_eq!(m.list_len(ListKind::Playlists), 4);
    // Opening the browser again starts with no filter.
    m.update(Action::Find, t0);
    type_in(&mut m, t0, "jam");
    m.update(Action::Close, t0);
    key(&mut m, t0, P::Playlists);
    assert_eq!(m.list_len(ListKind::Playlists), 4);
    assert!(!view(&m).typing);
}

#[test]
fn a_filter_looks_through_every_page_and_esc_stays_on_the_row() {
    let account = demo();
    let (mut m, t0, source) = rig("find-tracks", &account);
    key(&mut m, t0, P::Playlists);
    m.update(Action::Keep, t0);
    assert_eq!(m.list_total(ListKind::Tracks), 50, "one page so far");
    m.update(Action::Find, t0);
    type_in(&mut m, t0, "rise 5");
    tick(&mut m, t0);
    assert_eq!(
        m.list_total(ListKind::Tracks),
        60,
        "the rest loaded to look in"
    );
    let hits = names(&m, ListKind::Tracks);
    assert_eq!(hits.len(), 15, "{hits:?}"); // 5, 15, 25, 35, 45, 50..=59
    // Artists match too.
    m.update(Action::ClearFind, t0);
    m.update(Action::Find, t0);
    type_in(&mut m, t0, "wane 57");
    assert_eq!(names(&m, ListKind::Tracks), ["Slow Rise 57"]);
    // ⏎ plays the match, in its playlist.
    m.update(Action::Keep, t0);
    assert_eq!(
        source.sent().last(),
        Some(&Command::PlayInContext {
            track: "spotify:track:t57".into(),
            context: "spotify:playlist:mix".into(),
        })
    );
    // esc clears the filter and leaves the cursor on that track.
    m.update(Action::ClearFind, t0);
    let v = view(&m);
    assert_eq!(m.list_len(ListKind::Tracks), 60);
    assert_eq!(
        m.list_row(ListKind::Tracks, v.cursor).unwrap().name,
        "Slow Rise 57"
    );
}

#[test]
fn clicks_pick_filtered_rows() {
    let account = demo();
    let (mut m, t0, source) = rig("find-click", &account);
    key(&mut m, t0, P::Playlists);
    m.update(Action::Find, t0);
    type_in(&mut m, t0, "discover");
    let Some(crate::ui::picker::Placement::Sheet { list, .. }) = m.list_placement(&view(&m)) else {
        panic!("a sheet at 100x30")
    };
    let click = Action::Click {
        col: list.x + 4,
        row: list.y,
    };
    m.update(click, t0);
    m.update(click, t0);
    assert_eq!(
        source.sent().last(),
        Some(&Command::PlayUri("spotify:playlist:dw".into())),
        "a double click plays the one match"
    );
}

/// Made up, Spotify-shaped (22 base-62 characters).
const FAKE_ID: &str = "0LavaTuiFakeTrack00001";

fn settle(m: &mut Model, at: Instant) {
    for _ in 0..4 {
        tick(m, at);
    }
}

/// What Windows' media controls report: no volume, nothing to play URIs
/// with, and a track known only by its words.
const SMTC: Capabilities = Capabilities {
    volume: false,
    uris: false,
    contexts: false,
    ..Capabilities::ALL
};

fn smtc_track(name: &str) -> Track {
    crate::media::smtc::track(name, "Wax & Wane", "Lamplight", Duration::from_secs(200)).unwrap()
}

/// The Web API's player on some device, playing `uri` called `name`.
fn web_playing(uri: &str, name: &str) -> PlayerState {
    PlayerState {
        item_uri: Some(uri.into()),
        item_name: Some(name.into()),
        ..premium(false)
    }
}

#[test]
fn linux_spotify_tracks_can_be_liked_and_added_and_still_seek() {
    use crate::media::mpris::{self, Meta};
    let text = |s: &str| Meta::Text(s.into());
    // As Spotify on Linux reports it: an object path, and the link.
    let metadata = [
        (
            "mpris:trackid",
            text(&format!("/com/spotify/track/{FAKE_ID}")),
        ),
        ("mpris:length", Meta::Int(200_000_000)),
        ("xesam:title", text("Slow Rise")),
        ("xesam:artist", Meta::List(vec![text("The Paraffins")])),
        ("xesam:album", text("Heat Rises")),
        (
            "xesam:url",
            text(&format!("https://open.spotify.com/track/{FAKE_ID}")),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    let track = mpris::track(&metadata).unwrap();
    let uri = format!("spotify:track:{FAKE_ID}");
    let account = demo();
    let t0 = Instant::now();
    let caps = Capabilities {
        contexts: false,
        ..Capabilities::ALL
    };
    let (mut m, t0, source) = rig_on("mpris-like", &account, desktop("Spotify", track, t0), caps);
    settle(&mut m, t0);
    assert_eq!(m.liked(), Some(false), "asked about {uri}");
    key(&mut m, t0, P::Like);
    assert!(account.state().liked.contains(&uri));

    key(&mut m, t0, P::AddToPlaylist);
    m.update(Action::Keep, t0);
    tick(&mut m, t0);
    assert_eq!(toast(&m), "added to Lamplight Mix");
    assert!(account.state().tracks["mix"].iter().any(|t| t.uri == uri));

    // Seeking still names the track by its object path.
    let snap = m.music.snapshot.clone().unwrap();
    assert_eq!(
        mpris::plan(&Command::Seek(Duration::from_secs(9)), Some(&snap), t0),
        mpris::Call::SetPosition(format!("/com/spotify/track/{FAKE_ID}"), 9_000_000)
    );

    // A track in its playlist plays alone on Linux, and the toast says so.
    key(&mut m, t0, P::Playlists);
    m.update(Action::Keep, t0);
    m.update(Action::Keep, t0);
    assert_eq!(
        source.sent().last(),
        Some(&Command::PlayInContext {
            track: "spotify:track:t0".into(),
            context: "spotify:playlist:mix".into(),
        })
    );
    assert_eq!(toast(&m), "playing Slow Rise 0 · just this song");
}

#[test]
fn windows_spotify_is_matched_through_the_web_player() {
    let account = demo();
    account.state().player = Ok(Some(web_playing(PLAYING, "Slow Rise 0")));
    let t0 = Instant::now();
    let snap = desktop("Spotify", smtc_track("Slow Rise 0"), t0);
    let (mut m, t0, _) = rig_on("smtc-like", &account, snap, SMTC);
    settle(&mut m, t0);
    assert_eq!(m.liked(), Some(false));
    key(&mut m, t0, P::Like);
    assert!(account.state().liked.contains(PLAYING));
    key(&mut m, t0, P::AddToPlaylist);
    assert!(matches!(m.overlay, Overlay::Library(_)), "{}", toast(&m));
    m.update(Action::Keep, t0);
    tick(&mut m, t0);
    assert_eq!(toast(&m), "added to Lamplight Mix");

    // Spotify plays something else elsewhere: this track is not it.
    account.state().player = Ok(Some(web_playing("spotify:track:other", "Another Song")));
    let later = t0 + Duration::from_secs(31);
    settle(&mut m, later);
    assert_eq!(m.liked(), None);
    key(&mut m, later, P::Like);
    assert_eq!(toast(&m), "nothing to like");
    key(&mut m, later, P::AddToPlaylist);
    assert_eq!(toast(&m), "nothing playing to add");
}

#[test]
fn other_players_local_files_and_ads_are_never_given_a_spotify_uri() {
    let local = "spotify:local:Wax+%26+Wane:Lamplight:Slow+Rise+0:200";
    for (player, id, web) in [
        // Another app playing a song of the same name as the account's.
        ("Vlc", "Slow Rise 0\u{1f}Wax & Wane\u{1f}Lamplight", PLAYING),
        // Spotify's local file and ad (macOS ids).
        ("Spotify", local, local),
        ("Spotify", "spotify:ad:000000012c4a1bd4", local),
    ] {
        let account = demo();
        account.state().player = Ok(Some(web_playing(web, "Slow Rise 0")));
        let track = Track {
            id: id.into(),
            uri: None,
            ..smtc_track("Slow Rise 0")
        };
        let t0 = Instant::now();
        let (mut m, t0, _) = rig_on("no-uri", &account, desktop(player, track, t0), SMTC);
        settle(&mut m, t0);
        assert_eq!(m.liked(), None, "{player} {id}");
        key(&mut m, t0, P::Like);
        assert_eq!(toast(&m), "nothing to like", "{player} {id}");
        assert!(account.state().liked.is_empty());
        assert!(
            !account
                .state()
                .requests
                .iter()
                .any(|r| matches!(r, Request::LibraryContains { .. })),
            "{player} {id}"
        );
    }
}

#[test]
fn windows_library_playback_says_truthfully_when_it_cant() {
    let account = demo();
    let t0 = Instant::now();
    let mut snap = desktop("Spotify", smtc_track("Slow Rise 0"), t0);
    snap.status = Play::Paused;
    let (mut m, t0, source) = rig_on("smtc-play", &account, snap, SMTC);
    settle(&mut m, t0);
    let open_dw = |m: &mut Model| {
        key(m, t0, P::Playlists);
        m.update(Action::Edge(false), t0);
        m.update(Action::Down, t0);
        m.update(Action::Down, t0);
        m.update(Action::Keep, t0);
    };
    // No device playing: nothing to play it with yet.
    open_dw(&mut m);
    assert_eq!(toast(&m), "press play in Spotify first, then pick it again");
    assert!(source.sent().is_empty(), "{:?}", source.sent());
    let status = |m: &Model| m.music.snapshot.as_ref().unwrap().status.clone();
    assert_eq!(status(&m), Play::Paused, "not shown as playing");
    m.update(Action::Close, t0);

    // A device plays: the Web API plays it.
    account.state().player = Ok(Some(premium(false)));
    let later = t0 + Duration::from_secs(31);
    settle(&mut m, later);
    open_dw(&mut m);
    let play = Request::Play {
        context_uri: "spotify:playlist:dw".into(),
        offset_uri: None,
    };
    assert!(account.state().requests.contains(&play));
    assert_eq!(toast(&m), "playing Discover Weekly");

    // Spotify refuses (no Premium): no fallback that does nothing.
    account.state().fail = Some(Error::Forbidden("Premium required".into()));
    m.update(Action::Keep, t0);
    tick(&mut m, t0);
    assert_eq!(toast(&m), "playing from here needs Spotify Premium");
    assert!(source.sent().is_empty(), "{:?}", source.sent());
    assert_eq!(status(&m), Play::Paused);
}

#[test]
fn web_modes_belong_only_to_the_player_they_describe() {
    // VLC on the desktop while the account plays (shuffled) elsewhere.
    let account = demo();
    account.state().player = Ok(Some(premium(true)));
    let vlc = Track {
        id: "/org/videolan/vlc/playlist/7".into(),
        uri: None,
        ..smtc_track("Slow Rise 0")
    };
    let t0 = Instant::now();
    let (mut m, t0, source) = rig_on(
        "modes-vlc",
        &account,
        desktop("VLC media player", vlc.clone(), t0),
        Capabilities::ALL,
    );
    settle(&mut m, t0);
    assert_eq!(m.music.web_caps, Capabilities::NONE);
    assert!(!m.music.snapshot.as_ref().unwrap().shuffle, "VLC's own");
    key(&mut m, t0, P::Shuffle);
    assert_eq!(source.sent(), [Command::SetShuffle(true)]);
    let web_shuffle = |a: &FakeWeb| {
        a.state()
            .requests
            .iter()
            .any(|r| matches!(r, Request::SetShuffle(_)))
    };
    assert!(!web_shuffle(&account), "nothing went to the account");
    // A player without its own shuffle says so; the account is untouched.
    source.set_capabilities(Capabilities::NONE);
    key(&mut m, t0, P::Shuffle);
    assert!(toast(&m).contains("can't shuffle"), "{}", toast(&m));
    assert!(!web_shuffle(&account));

    // Switching to Spotify playing that track: the Web modes apply...
    let spotify = Track {
        id: PLAYING.into(),
        uri: Some(PLAYING.into()),
        ..vlc.clone()
    };
    source.set(desktop("Spotify", spotify, t0));
    settle(&mut m, t0);
    assert!(m.music.web_caps.shuffle);
    assert!(m.music.snapshot.as_ref().unwrap().shuffle);
    key(&mut m, t0, P::Shuffle);
    assert!(web_shuffle(&account));
    // ...and back to VLC, they're gone at once.
    source.set(desktop("VLC media player", vlc, t0));
    tick(&mut m, t0);
    assert_eq!(m.music.web_caps, Capabilities::NONE);
    assert!(!m.music.snapshot.as_ref().unwrap().shuffle);
}

#[test]
fn a_refused_account_is_told_how_to_fix_it_once() {
    // Logged in, but not on the app's allowlist: every request is a 403.
    let account = demo();
    account.state().fail = Some(Error::Forbidden(
        "Check settings on developer.spotify.com/dashboard, the user may not be registered.".into(),
    ));
    let (mut m, t0, _) = rig("refused", &account);
    settle(&mut m, t0);
    assert!(m.library.refused.is_some());
    assert!(
        m.list_message(ListKind::Playlists)
            .starts_with("Spotify refused this account"),
        "no endless loading…"
    );
    let asked = |a: &FakeWeb| {
        a.state()
            .requests
            .iter()
            .filter(|r| **r == Request::Me)
            .count()
    };
    let before = asked(&account);
    settle(&mut m, t0 + Duration::from_secs(1));
    assert_eq!(asked(&account), before, "not asked again and again");
    // A library key opens the setup, whose first row says why.
    key(&mut m, t0, P::Playlists);
    let view = m.settings_view().expect("the setup");
    assert_eq!(view.page, crate::app::Page::Spotify);
    let rows = m.settings_rows(crate::app::Page::Spotify);
    assert_eq!(rows[0].value, "refused");
    assert!(
        rows[0].about.contains("User Management"),
        "{}",
        rows[0].about
    );
    // Logging in again starts afresh.
    account.state().fail = None;
    m.library.logout();
    settle(&mut m, t0);
    assert!(m.library.refused.is_none());
}
