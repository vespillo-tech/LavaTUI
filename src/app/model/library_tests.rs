//! The Spotify library flows against `FakeWeb` (an in-memory account) and
//! `FakeSource` (the desktop app): login / logout, like, the playlist
//! browser, add to playlist, shuffle / repeat through the Web API.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::library::{CHECK_STALL, KEYCHAIN_HEADS_UP, PENDING_FOR};
use super::*;
use crate::media::{Capabilities, Command, FakeSource, Snapshot, Status as Play, Track};
use crate::spotify_web::fake::{FakeWeb, demo, track as web_track};
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
        item: Some(web_track("t0", "Slow Rise 0", "Wax & Wane")),
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
fn songs_removed_from_a_playlist_do_not_repeat_on_the_next_page() {
    let account = demo();
    // 60 songs with four empty slots (two together, one at the end of the
    // first page, one at the very end): 64 slots over two pages.
    account
        .state()
        .holes
        .insert("mix".into(), vec![3, 4, 49, 63]);
    let (mut m, t0, _source) = rig("holes", &account);
    key(&mut m, t0, P::Playlists);
    m.update(Action::Keep, t0);
    assert_eq!(m.list_total(ListKind::Tracks), 47, "the first page's songs");
    // A filter looks through everything: the rest loads.
    m.update(Action::Find, t0);
    type_in(&mut m, t0, "slow");
    tick(&mut m, t0);
    tick(&mut m, t0);
    let ids: Vec<String> = m
        .library
        .open
        .as_ref()
        .unwrap()
        .tracks
        .items
        .iter()
        .map(|t| t.id.clone().unwrap())
        .collect();
    assert_eq!(ids.len(), 60, "every song once");
    assert_eq!(ids, (0..60).map(|i| format!("t{i}")).collect::<Vec<_>>());
    let offsets: Vec<u32> = account
        .state()
        .requests
        .iter()
        .filter_map(|r| match r {
            Request::PlaylistTracks {
                playlist_id,
                offset,
            } if playlist_id == "mix" => Some(*offset),
            _ => None,
        })
        .collect();
    assert_eq!(
        offsets,
        [0, 50],
        "the second page starts after the empty slots"
    );
    assert!(
        !m.library.open.as_ref().unwrap().has_more,
        "the last page ends it"
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

/// The Web API's player on some device, playing `uri` called `name` (by
/// the demo's artist, on its album, as long as [`smtc_track`]).
fn web_playing(uri: &str, name: &str) -> PlayerState {
    let item = crate::spotify_web::Track {
        uri: uri.into(),
        name: name.into(),
        ..web_track("t0", name, "Wax & Wane")
    };
    PlayerState {
        item: Some(item),
        ..premium(false)
    }
}

/// [`web_playing`] the [`PLAYING`] track, as `change` makes it.
fn web_item(change: impl FnOnce(&mut crate::spotify_web::Track)) -> PlayerState {
    let mut state = web_playing(PLAYING, "Slow Rise 0");
    change(state.item.as_mut().unwrap());
    state
}

/// Whether anything was asked of the account about a track or its modes.
fn touched(account: &FakeWeb) -> Vec<Request> {
    account
        .state()
        .requests
        .iter()
        .filter(|r| {
            matches!(
                r,
                Request::LibraryContains { .. }
                    | Request::Like { .. }
                    | Request::Unlike { .. }
                    | Request::AddToPlaylist { .. }
                    | Request::SetShuffle(_)
                    | Request::SetRepeat(_)
                    | Request::Play { .. }
            )
        })
        .cloned()
        .collect()
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
    settle(&mut m, t0);
    // It's in there already: added again only when asked to.
    assert_eq!(stage(&m), Some(super::Stage::Confirm));
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
fn windows_spotify_needs_more_than_a_title_to_match() {
    // The desktop app (media controls: no URI) plays "Slow Rise 0" by Wax
    // & Wane, 200 s, on Lamplight; the account's player says...
    let cases: [(&str, PlayerState); 7] = [
        (
            "the same title by another artist",
            web_item(|t| {
                t.artists = vec!["The Paraffins".into()];
            }),
        ),
        (
            "the same title and artist, another length",
            web_item(|t| {
                t.duration_ms = 245_000;
            }),
        ),
        (
            "the same title and artist on another album",
            web_item(|t| {
                t.album = "Live at the Lamp".into();
            }),
        ),
        (
            "a local file of it",
            web_item(|t| {
                t.uri = "spotify:local:Wax+%26+Wane:Lamplight:Slow+Rise+0:200".into();
                t.id = None;
                t.is_local = true;
            }),
        ),
        (
            "an episode of that name",
            web_item(|t| {
                t.uri = "spotify:episode:0LavaTuiFakeEpisode001".into();
            }),
        ),
        ("no artist", web_item(|t| t.artists.clear())),
        (
            "only the title, nothing else to go on",
            web_item(|t| {
                t.duration_ms = 0;
                t.album.clear();
            }),
        ),
    ];
    for (what, state) in cases {
        let account = demo();
        account.state().player = Ok(Some(state));
        let t0 = Instant::now();
        let snap = desktop("Spotify", smtc_track("Slow Rise 0"), t0);
        let (mut m, t0, source) = rig_on("smtc-same-title", &account, snap, SMTC);
        settle(&mut m, t0);
        assert_eq!(m.liked(), None, "{what}");
        assert_eq!(m.music.web_caps, Capabilities::NONE, "{what}");
        key(&mut m, t0, P::Like);
        assert_eq!(toast(&m), "nothing to like", "{what}");
        key(&mut m, t0, P::AddToPlaylist);
        assert_eq!(toast(&m), "nothing playing to add", "{what}");
        // Shuffle stays the desktop app's own: the account is untouched.
        key(&mut m, t0, P::Shuffle);
        assert_eq!(source.sent(), [Command::SetShuffle(true)], "{what}");
        settle(&mut m, t0);
        assert_eq!(touched(&account), [], "{what}");
    }
}

#[test]
fn windows_spotify_matches_the_same_track_however_its_artists_are_listed() {
    let both = |t: &mut crate::spotify_web::Track| {
        t.artists = vec!["Wax & Wane".into(), "Mara Vell".into()];
    };
    let cases: [(&str, &str, Duration, PlayerState); 4] = [
        (
            "every artist, a rounded length",
            "Wax & Wane, Mara Vell",
            Duration::from_millis(200_400),
            web_item(both),
        ),
        (
            "the first artist only",
            "Wax & Wane",
            Duration::from_secs(200),
            web_item(both),
        ),
        (
            "no length: the album says",
            "Wax & Wane",
            Duration::ZERO,
            web_playing(PLAYING, "Slow Rise 0"),
        ),
        (
            "case and punctuation aside",
            "wax and wane",
            Duration::from_secs(200),
            web_item(|t| {
                t.artists = vec!["Wax And Wane".into()];
                t.name = "slow rise 0".into();
            }),
        ),
    ];
    for (what, artist, length, state) in cases {
        let account = demo();
        account.state().player = Ok(Some(state));
        let track = crate::media::smtc::track("Slow Rise 0", artist, "Lamplight", length).unwrap();
        let t0 = Instant::now();
        let (mut m, t0, _) = rig_on("smtc-match", &account, desktop("Spotify", track, t0), SMTC);
        settle(&mut m, t0);
        assert_eq!(m.liked(), Some(false), "{what}");
        key(&mut m, t0, P::Like);
        assert!(account.state().liked.contains(PLAYING), "{what}");
    }
}

#[test]
fn windows_spotify_never_matches_a_state_from_before_the_track_changed() {
    // The account's player (another device) plays "Slow Rise 0"; the
    // desktop app moves on to a track of that very name.
    let account = demo();
    account.state().player = Ok(Some(web_playing(PLAYING, "Slow Rise 0")));
    let t0 = Instant::now();
    let other =
        crate::media::smtc::track("Ember", "Wax & Wane", "Lamplight", Duration::from_secs(180));
    let (mut m, t0, source) = rig_on(
        "smtc-stale",
        &account,
        desktop("Spotify", other.unwrap(), t0),
        SMTC,
    );
    settle(&mut m, t0);
    assert_eq!(m.liked(), None, "Ember is not it");
    source.set(desktop("Spotify", smtc_track("Slow Rise 0"), t0));
    // The state in hand was asked for while Ember played: not about this.
    tick(&mut m, t0);
    assert_eq!(m.playing_uri(), None);
    assert_eq!(m.music.web_caps, Capabilities::NONE);
    // Once Spotify answers for this track, it is.
    settle(&mut m, t0);
    assert_eq!(m.playing_uri().as_deref(), Some(PLAYING));
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
fn a_player_that_ignores_shuffle_says_so_and_spotify_then_uses_the_account() {
    // Spotify over MPRIS, logged out: shuffle goes to the player, which
    // (as Spotify on Linux does) takes it and changes nothing.
    let account = demo();
    account.state().logged_in = false;
    let caps = Capabilities {
        contexts: false,
        ..Capabilities::ALL
    };
    let t0 = Instant::now();
    let track = Track {
        id: "/com/spotify/track/t0".into(),
        uri: Some(PLAYING.into()),
        ..smtc_track("Slow Rise 0")
    };
    let snap = desktop("Spotify", track, t0);
    let (mut m, t0, source) = rig_on("modes-ignored", &account, snap.clone(), caps);
    settle(&mut m, t0);
    key(&mut m, t0, P::Shuffle);
    assert_eq!(source.sent(), [Command::SetShuffle(true)]);
    // The backend sees the read didn't change ([`ModesCheck`]): the
    // optimistic "on" is undone and shuffle / repeat are withdrawn.
    source.set(snap.clone());
    source.set_capabilities(Capabilities {
        shuffle: false,
        repeat: false,
        ..caps
    });
    tick(&mut m, t0);
    assert_eq!(
        toast(&m),
        "Spotify ignored that · log in (i) to shuffle and repeat"
    );
    assert!(!m.music.snapshot.as_ref().unwrap().shuffle);
    assert!(!m.music.capabilities().shuffle, "no longer offered");
    // Pressed again: nothing sent, and it says what would work.
    key(&mut m, t0, P::Repeat);
    assert_eq!(source.sent().len(), 1);
    assert_eq!(
        toast(&m),
        "Spotify can't shuffle or repeat from here · log in (i) to"
    );

    // Logged in (Premium, this track): the account's player does it.
    account.state().player = Ok(Some(premium(false)));
    account.finish_login();
    settle(&mut m, t0);
    assert!(m.music.capabilities().shuffle);
    key(&mut m, t0, P::Shuffle);
    tick(&mut m, t0);
    assert!(
        account
            .state()
            .requests
            .contains(&Request::SetShuffle(true))
    );
    assert_eq!(source.sent().len(), 1, "not the player that ignores it");
}

#[test]
fn spotify_modes_go_to_the_account_even_when_its_player_offers_them() {
    // A player with its own shuffle (MPRIS, before any check) playing the
    // track the logged-in account's player shows: the modes shown are the
    // account's, so that's where a change goes.
    let account = demo();
    account.state().player = Ok(Some(premium(false)));
    let (mut m, t0, source) = rig("modes-web-first", &account);
    assert!(m.music.source_capabilities().repeat);
    settle(&mut m, t0);
    key(&mut m, t0, P::Repeat);
    tick(&mut m, t0);
    assert!(source.sent().is_empty(), "{:?}", source.sent());
    assert!(
        account
            .state()
            .requests
            .contains(&Request::SetRepeat(Repeat::Context))
    );
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

// ---- the saved login, read only when needed (lava-1xk.38) -----------------

/// [`rig`] with the demo account's login saved but unread (macOS), and
/// the config saying a login is saved.
fn locked_rig(name: &str, account: &FakeWeb, saved_flag: bool) -> (Model, Instant, FakeSource) {
    let (mut m, t0, source) = rig(name, account);
    m.settings.spotify.logged_in = saved_flag;
    m.library.saved = saved_flag;
    for _ in 0..3 {
        tick(&mut m, t0);
    }
    (m, t0, source)
}

#[test]
fn starting_never_reads_the_saved_login() {
    let account = demo().locked();
    let (m, _, _) = locked_rig("lazy-start", &account, true);
    {
        let s = account.state();
        assert_eq!(s.unlocks, 0, "read at start");
        assert!(s.requests.is_empty(), "{:?}", s.requests);
    }
    // The config's flag says logged in, without the secret.
    assert_eq!(m.library.account(), Account::LoggedIn);
    assert!(!m.library.logged_in());
    assert_eq!(m.liked(), None);
}

#[test]
fn the_first_library_key_warns_reads_the_login_then_runs() {
    let account = demo().locked();
    let (mut m, t0, _) = locked_rig("lazy-like", &account, true);
    // macOS's question is still open: the answer is held back.
    account.state().hold = true;
    key(&mut m, t0, P::Like);
    assert_eq!(toast(&m), KEYCHAIN_HEADS_UP);
    assert_eq!(account.state().unlocks, 1);
    tick(&mut m, t0);
    assert!(!account.state().liked.contains(PLAYING), "not yet");
    // A second key while macOS asks doesn't read it again.
    key(&mut m, t0, P::Like);
    assert_eq!(account.state().unlocks, 1);
    account.release();
    tick(&mut m, t0);
    account.release();
    assert!(account.state().liked.contains(PLAYING), "the like ran");
    assert_eq!(toast(&m), "♥ liked");
    assert!(m.settings.spotify.logged_in);
    // Read once a session: later keys just work.
    key(&mut m, t0, P::Like);
    assert_eq!(account.state().unlocks, 1);
    assert!(!account.state().liked.contains(PLAYING));
}

#[test]
fn a_login_from_before_the_flag_is_found_by_i_not_replaced() {
    let account = demo().locked();
    let (mut m, t0, _) = locked_rig("lazy-upgrade", &account, false);
    assert_eq!(m.library.account(), Account::LoggedOut);
    key(&mut m, t0, P::Account);
    tick(&mut m, t0);
    assert_eq!(account.state().unlocks, 1);
    assert_eq!(toast(&m), "logged in to Spotify");
    assert!(!account.state().login_pending, "no new browser login");
    assert_eq!(m.library.account(), Account::LoggedIn);
    assert!(m.settings.spotify.logged_in, "remembered for next time");
}

#[test]
fn a_stale_flag_turns_into_the_login_offer() {
    let account = demo().locked();
    account.state().saved = false;
    let (mut m, t0, _) = locked_rig("lazy-stale", &account, true);
    key(&mut m, t0, P::Playlists);
    tick(&mut m, t0);
    assert!(!m.settings.spotify.logged_in);
    assert_eq!(m.library.account(), Account::LoggedOut);
    assert!(matches!(m.overlay, Overlay::Library(_)), "the key ran");
}

#[test]
fn shuffle_reads_the_login_then_goes_through_the_account() {
    let account = demo().locked();
    account.state().player = Ok(Some(premium(false)));
    let (mut m, t0, source) = locked_rig("lazy-shuffle", &account, true);
    source.set_capabilities(Capabilities::NONE);
    tick(&mut m, t0);
    key(&mut m, t0, P::Shuffle);
    assert_eq!(toast(&m), KEYCHAIN_HEADS_UP);
    for _ in 0..3 {
        tick(&mut m, t0);
    }
    let reqs = account.state().requests.clone();
    assert!(reqs.contains(&Request::SetShuffle(true)), "{reqs:?}");
    assert!(source.sent().is_empty());
}

#[test]
fn a_key_waiting_for_the_login_gives_up_after_a_while() {
    let account = demo().locked();
    let (mut m, t0, _) = locked_rig("lazy-timeout", &account, true);
    // macOS's question sits unanswered.
    account.state().hold = true;
    key(&mut m, t0, P::Like);
    tick(&mut m, t0 + Duration::from_secs(1));
    assert!(m.library.pending.is_some());
    tick(&mut m, t0 + PENDING_FOR + Duration::from_secs(1));
    assert!(m.library.pending.is_none());
    // Answered after all: nothing runs by surprise.
    account.release();
    tick(&mut m, t0 + PENDING_FOR + Duration::from_secs(2));
    assert!(!account.state().liked.contains(PLAYING));
}

/// The player keys the music card's controls stand for, as clicked.
fn buttons(m: &Model) -> Vec<P> {
    let area = m.layout.area;
    let mut found = Vec::new();
    for row in area.top()..area.bottom() {
        for col in area.left()..area.right() {
            if let Some(k) = m.music_hit(col, row)
                && !found.contains(&k)
            {
                found.push(k);
            }
        }
    }
    found
}

/// lava-75z.22: a failed "is it liked?" (offline, a rate limit) used to be
/// asked again on the very next frame, over and over, which kept a rate
/// limit going; and the `+` waited on its answer. Now it waits (Spotify's
/// `Retry-After` when longer), and `+` shows for any Spotify song.
#[test]
fn a_failed_liked_lookup_waits_and_add_stays() {
    let account = demo();
    let (mut m, t0, source) = rig("liked-retry", &account);
    m.settings.input.mouse = true;
    m.update(Action::Back, t0);
    tick(&mut m, t0);
    assert_eq!(m.liked(), Some(false));
    let asked = |a: &FakeWeb| {
        a.state()
            .requests
            .iter()
            .filter(|r| matches!(r, Request::LibraryContains { .. }))
            .count()
    };
    let before = asked(&account);

    // The next song's lookup is rate limited.
    account.state().fail = Some(Error::RateLimited {
        retry_after: Duration::from_secs(20),
    });
    let next = Track {
        id: "spotify:track:t1".into(),
        uri: Some("spotify:track:t1".into()),
        name: "Slow Rise 1".into(),
        ..Track::default()
    };
    source.set(desktop("Spotify", next, t0));
    for frame in 0..30 {
        tick(&mut m, t0 + Duration::from_millis(16 * frame));
    }
    assert_eq!(asked(&account), before + 1, "not asked every frame");
    assert_eq!(m.liked(), None, "no heart while unknown");
    let shown = buttons(&m);
    assert!(shown.contains(&P::AddToPlaylist), "{shown:?}");
    assert!(!shown.contains(&P::Like), "{shown:?}");

    // Not before Spotify said to wait; then asked again, and the heart is
    // back.
    tick(&mut m, t0 + Duration::from_secs(10));
    assert_eq!(asked(&account), before + 1);
    tick(&mut m, t0 + Duration::from_secs(21));
    assert_eq!(asked(&account), before + 2);
    tick(&mut m, t0 + Duration::from_secs(21));
    assert_eq!(m.liked(), Some(false));
    assert!(buttons(&m).contains(&P::Like));
}

// ---- a song the playlist has already (lava-75z.24) -------------------------

/// The pages of playlist songs read so far.
fn reads(account: &FakeWeb) -> usize {
    let s = account.state();
    let reads = s.requests.iter();
    reads
        .filter(|r| matches!(r, Request::PlaylistUris { .. }))
        .count()
}

fn adds(account: &FakeWeb) -> usize {
    let s = account.state();
    let adds = s.requests.iter();
    adds.filter(|r| matches!(r, Request::AddToPlaylist { .. }))
        .count()
}

fn stage(m: &Model) -> Option<super::Stage> {
    m.library.adding.as_ref().map(|a| a.stage)
}

/// The add picker open on the demo account (what's playing, `t0`, is in
/// "Lamplight Mix", the first row) and read ahead.
fn add_picker(name: &str, account: &FakeWeb) -> (Model, Instant) {
    let (mut m, t0, _) = rig(name, account);
    key(&mut m, t0, P::AddToPlaylist);
    settle(&mut m, t0);
    (m, t0)
}

#[test]
fn a_song_the_playlist_has_asks_before_adding_it_again() {
    let account = demo();
    let (mut m, t0) = add_picker("again", &account);
    // Read ahead: the playlist that has it is marked, quietly.
    let mix = m.list_row(ListKind::AddTo, 0).unwrap();
    assert_eq!(mix.detail, "✓ 60");
    assert_eq!(m.list_row(ListKind::AddTo, 1).unwrap().detail, "0");

    m.update(Action::Keep, t0);
    assert_eq!(stage(&m), Some(super::Stage::Confirm));
    assert!(matches!(m.overlay, Overlay::Library(_)), "still asking");
    assert_eq!(adds(&account), 0, "nothing added yet");
    // Everything but ⏎ / esc / q waits.
    m.update(Action::Down, t0);
    m.update(Action::Type('x'), t0);
    assert_eq!(stage(&m), Some(super::Stage::Confirm));

    // ⏎: in it goes, again.
    m.update(Action::Keep, t0);
    assert_eq!(m.overlay, Overlay::None);
    tick(&mut m, t0);
    assert_eq!(toast(&m), "added to Lamplight Mix");
    let twice = account.state().tracks["mix"]
        .iter()
        .filter(|t| t.uri == PLAYING)
        .count();
    assert_eq!(twice, 2);
}

#[test]
fn esc_at_the_question_adds_nothing_and_goes_back_to_the_list() {
    let account = demo();
    let (mut m, t0) = add_picker("again-esc", &account);
    m.update(Action::Keep, t0);
    assert_eq!(stage(&m), Some(super::Stage::Confirm));
    m.update(Action::Back, t0);
    assert_eq!(m.library.adding, None);
    let Overlay::Library(view) = m.overlay else {
        panic!("back to the list")
    };
    assert_eq!((view.kind, view.cursor), (ListKind::AddTo, 0));
    // q from the question closes it all.
    m.update(Action::Keep, t0);
    m.update(Action::Close, t0);
    assert_eq!((m.overlay, m.library.adding.clone()), (Overlay::None, None));
    settle(&mut m, t0);
    assert_eq!(adds(&account), 0);
    assert_eq!(account.state().tracks["mix"].len(), 60);
}

#[test]
fn a_song_the_playlist_lacks_goes_straight_in() {
    let account = demo();
    let (mut m, t0) = add_picker("not-there", &account);
    m.update(Action::Down, t0);
    m.update(Action::Keep, t0);
    assert_eq!(m.overlay, Overlay::None, "no question");
    tick(&mut m, t0);
    assert_eq!(toast(&m), "added to lavatui test");
    // Known now: the mark follows the add, and no page is read again.
    let before = reads(&account);
    key(&mut m, t0, P::AddToPlaylist);
    settle(&mut m, t0);
    assert_eq!(m.list_row(ListKind::AddTo, 1).unwrap().detail, "✓ 1");
    assert_eq!(reads(&account), before, "still known");
}

/// The add picker opened again after the playlist changed elsewhere (a
/// new snapshot id: what was read of it is stale), Spotify holding its
/// answers from here on.
fn reopen_changed(m: &mut Model, account: &FakeWeb, t: Instant, snapshot: &str) {
    if m.overlay != Overlay::None {
        m.update(Action::Close, t);
    }
    account.state().hold = true;
    account.state().playlists[0].snapshot_id = snapshot.into();
    key(m, t, P::AddToPlaylist);
    // The playlists come back; their songs don't, yet.
    account.release();
    account.state().hold = true;
    tick(m, t);
}

#[test]
fn chosen_before_the_check_is_done_it_waits_then_decides() {
    let account = demo();
    let (mut m, t0) = add_picker("checking", &account);
    reopen_changed(&mut m, &account, t0, "changed");
    m.update(Action::Keep, t0);
    assert!(matches!(stage(&m), Some(super::Stage::Checking { .. })));
    assert!(m.library.busy(), "frames keep coming while it checks");
    account.release();
    settle(&mut m, t0);
    assert_eq!(stage(&m), Some(super::Stage::Confirm));
}

#[test]
fn a_failed_check_still_adds_and_says_so() {
    let account = demo();
    account.state().fail_uris = Some(Error::Offline("no network".into()));
    let (mut m, t0) = add_picker("check-failed", &account);
    assert_eq!(m.list_row(ListKind::AddTo, 0).unwrap().detail, "60");
    m.update(Action::Keep, t0);
    assert_eq!(m.overlay, Overlay::None);
    tick(&mut m, t0);
    assert_eq!(
        toast(&m),
        "added to Lamplight Mix · couldn't check it first"
    );
    assert_eq!(account.state().tracks["mix"].len(), 61);
}

#[test]
fn a_stalled_check_adds_anyway_and_enter_need_not_wait() {
    let account = demo();
    let (mut m, t0) = add_picker("check-stalled", &account);
    reopen_changed(&mut m, &account, t0, "elsewhere");
    m.update(Action::Keep, t0);
    assert!(matches!(stage(&m), Some(super::Stage::Checking { .. })));
    tick(&mut m, t0 + CHECK_STALL - Duration::from_millis(1));
    assert!(m.library.adding.is_some(), "still waiting");
    tick(&mut m, t0 + CHECK_STALL);
    assert_eq!(m.overlay, Overlay::None);
    account.release();
    settle(&mut m, t0 + CHECK_STALL);
    assert_eq!(
        toast(&m),
        "added to Lamplight Mix · couldn't check it first"
    );

    // ⏎ while checking adds at once.
    reopen_changed(&mut m, &account, t0, "again");
    m.update(Action::Keep, t0);
    assert!(matches!(stage(&m), Some(super::Stage::Checking { .. })));
    m.update(Action::Keep, t0);
    assert_eq!(m.overlay, Overlay::None);
    account.release();
    settle(&mut m, t0);
    assert_eq!(toast(&m), "added to Lamplight Mix");
}

#[test]
fn a_long_playlist_is_read_once_page_by_page() {
    let account = demo();
    {
        let mut s = account.state();
        let mut big: Vec<_> = (0..260)
            .map(|i| web_track(&format!("b{i}"), &format!("Deep Wax {i}"), "Wax & Wane"))
            .collect();
        big[255] = web_track("t0", "Slow Rise 0", "Wax & Wane");
        let mut p = crate::spotify_web::fake::playlist("big", "Long Night", "me", false, 260);
        p.snapshot_id = "big1".into();
        s.playlists.insert(0, p);
        s.tracks.insert("big".into(), big);
    }
    let (mut m, t0) = add_picker("long", &account);
    assert_eq!(m.list_row(ListKind::AddTo, 0).unwrap().name, "Long Night");
    assert_eq!(m.list_row(ListKind::AddTo, 0).unwrap().detail, "✓ 260");
    let pages: Vec<u32> = account
        .state()
        .requests
        .iter()
        .filter_map(|r| match r {
            Request::PlaylistUris {
                playlist_id,
                offset,
            } if playlist_id == "big" => Some(*offset),
            _ => None,
        })
        .collect();
    assert_eq!(pages, [0, 50, 100, 150, 200, 250], "each page once");
    m.update(Action::Keep, t0);
    assert_eq!(stage(&m), Some(super::Stage::Confirm));

    // Opened again with nothing changed: nothing is read again.
    m.update(Action::Close, t0);
    let before = reads(&account);
    key(&mut m, t0, P::AddToPlaylist);
    settle(&mut m, t0);
    assert_eq!(reads(&account), before);
}

#[test]
fn reading_ahead_stops_at_its_budget_and_when_spotify_says_wait() {
    let account = demo();
    {
        let mut s = account.state();
        for n in 0..3 {
            let id = format!("long{n}");
            let tracks = (0..1000)
                .map(|i| web_track(&format!("{id}x{i}"), "Drip", "Wax & Wane"))
                .collect();
            s.playlists.push(crate::spotify_web::fake::playlist(
                &id, &id, "me", false, 1000,
            ));
            s.tracks.insert(id, tracks);
        }
    }
    let (mut m, t0) = add_picker("budget", &account);
    for _ in 0..20 {
        tick(&mut m, t0);
    }
    assert_eq!(reads(&account), 40, "the read-ahead budget");
    m.update(Action::Close, t0);

    // Too many requests: nothing more until the wait is over.
    let account = demo();
    account.state().fail_uris = Some(Error::RateLimited {
        retry_after: Duration::from_secs(30),
    });
    let (mut m, t0) = add_picker("rate-limited", &account);
    assert_eq!(reads(&account), 1);
    settle(&mut m, t0 + Duration::from_secs(10));
    assert_eq!(reads(&account), 1);
    account.state().fail_uris = None;
    settle(&mut m, t0 + Duration::from_secs(31));
    assert!(reads(&account) > 1);
}

/// `--demo`'s model with music beside the lamp and the player keys on.
fn demo_model(name: &str) -> (Model, Instant) {
    let t0 = Instant::now();
    let session = Session {
        demo: true,
        ..Session::default()
    };
    let mut m = Model::new(
        &session,
        Store::new(Some(temp_config(name))),
        Rect::new(0, 0, 100, 30),
        None,
        local(),
        1,
        t0,
    );
    m.welcome = false;
    m.update(Action::Place("music"), t0);
    settle(&mut m, t0);
    m.update(Action::PlayerKeys, t0);
    (m, t0)
}

#[test]
fn the_demo_is_logged_in_to_a_made_up_account() {
    let (mut m, t0) = demo_model("demo-account");
    assert!(m.library.demo);
    assert_eq!(m.library.account(), Account::LoggedIn);
    assert_eq!(m.liked(), Some(true), "the first song is liked");
    key(&mut m, t0, P::Playlists);
    settle(&mut m, t0);
    assert_eq!(
        names(&m, ListKind::Playlists),
        [
            "Late Night Lava",
            "Slow Sunday",
            "Pomodoro Focus",
            "Rising Heat"
        ]
    );
    m.update(Action::Keep, t0);
    settle(&mut m, t0);
    assert_eq!(
        names(&m, ListKind::Tracks)[..2],
        ["Slow Rise", "Warm Light Falling"]
    );
    // Set up, as far as the settings screen goes, with no Client ID.
    assert!(m.spotify_set_up());

    // Logging out and in needs no browser, and the config never hears.
    m.update(Action::Close, t0);
    let saved = m.settings.spotify.logged_in;
    key(&mut m, t0, P::Account);
    key(&mut m, t0, P::Account);
    settle(&mut m, t0);
    assert_eq!(m.library.account(), Account::LoggedOut);
    key(&mut m, t0, P::Account);
    settle(&mut m, t0);
    assert_eq!(m.library.account(), Account::LoggedIn);
    assert_eq!(toast(&m), "logged in to Spotify");
    assert_eq!(m.settings.spotify.logged_in, saved);
}

/// [`demo_model`] with the demo's player and account kept to look at.
fn demo_rig(name: &str) -> (Model, Instant, FakeSource, FakeWeb) {
    let (mut m, t0) = demo_model(name);
    let (source, account) = (crate::demo::source(t0), crate::demo::account());
    let (s, a) = (source.clone(), account.clone());
    m.music.connect_with(
        move || Box::new(s.clone()),
        crate::media::art::ArtLoader::start,
    );
    m.library
        .connect_with(move || Some(Box::new(a.clone()) as Box<dyn Web>));
    settle(&mut m, t0);
    (m, t0, source, account)
}

#[test]
fn the_demo_asks_before_adding_its_first_song_again() {
    let (mut m, t0, _, account) = demo_rig("demo-again");
    assert_eq!(m.library.account(), Account::LoggedIn);
    key(&mut m, t0, P::AddToPlaylist);
    settle(&mut m, t0);
    assert_eq!(
        names(&m, ListKind::AddTo),
        [
            "Late Night Lava",
            "Slow Sunday",
            "Pomodoro Focus",
            "Rising Heat"
        ]
    );
    assert_eq!(m.list_row(ListKind::AddTo, 0).unwrap().detail, "✓ 5");
    m.update(Action::Keep, t0);
    assert_eq!(stage(&m), Some(super::Stage::Confirm));
    m.update(Action::Back, t0);

    // Another playlist takes it at once, as the song it is.
    m.update(Action::Down, t0);
    m.update(Action::Keep, t0);
    tick(&mut m, t0);
    assert_eq!(toast(&m), "added to Slow Sunday");
    let s = account.state();
    let added = s
        .tracks
        .values()
        .flatten()
        .filter(|t| t.name == "Slow Rise");
    assert_eq!(added.count(), 2, "Late Night Lava's and the new one");
    // Everything asked went to the fake.
    assert!(
        s.requests
            .iter()
            .any(|r| matches!(r, Request::AddToPlaylist { .. }))
    );
}

#[test]
fn the_demo_plays_from_the_browser_and_likes() {
    let (mut m, t0, source, account) = demo_rig("demo-play");
    // Slow Sunday's second song, then on through Slow Sunday.
    key(&mut m, t0, P::Playlists);
    settle(&mut m, t0);
    m.update(Action::Down, t0);
    m.update(Action::Keep, t0);
    settle(&mut m, t0);
    m.update(Action::Down, t0);
    m.update(Action::Keep, t0);
    let name = |s: &FakeSource| s.snapshot_at(t0).track.unwrap().name.clone();
    assert_eq!(name(&source), "Convection");
    source.send_at(Command::Next, t0);
    assert_eq!(name(&source), "Blob Merge");
    // `p`: Pomodoro Focus from the top.
    m.update(Action::Close, t0);
    key(&mut m, t0, P::Playlists);
    m.update(Action::Down, t0);
    m.update(Action::Down, t0);
    m.update(Action::PlayAll, t0);
    assert_eq!(name(&source), "Convection");
    source.send_at(Command::Next, t0);
    assert_eq!(name(&source), "Ninety Minutes to Warm");
    m.update(Action::Close, t0);

    // Like it, then unlike it.
    settle(&mut m, t0);
    assert_eq!(m.liked(), Some(false));
    key(&mut m, t0, P::Like);
    settle(&mut m, t0);
    assert_eq!(m.liked(), Some(true));
    let uri = m.playing_uri().unwrap();
    assert!(account.state().liked.contains(&uri));
    key(&mut m, t0, P::Like);
    settle(&mut m, t0);
    assert_eq!(m.liked(), Some(false));
    assert!(!account.state().liked.contains(&uri));
}
