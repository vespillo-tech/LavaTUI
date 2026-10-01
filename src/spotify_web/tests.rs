//! The client and worker against a scripted HTTP layer: every request is
//! recorded, every reply comes off a queue (an unexpected request panics).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::client::Client;
use super::http::{Body, Http, Method, Request as HttpRequest, Response, TransportError};
use super::store::{MemoryStore, TokenStore, Tokens};
use super::*;

const NOW: u64 = 1_000_000;
const ID: &str = "client-123";

#[derive(Clone, Default)]
struct Mock {
    script: Arc<Mutex<VecDeque<Result<Response, TransportError>>>>,
    sent: Arc<Mutex<Vec<HttpRequest>>>,
}

impl Mock {
    fn reply(&self, status: u16, body: &str) -> &Self {
        self.push(Ok(Response {
            status,
            retry_after: None,
            body: body.into(),
        }))
    }

    fn rate_limit(&self, retry_after: &str) -> &Self {
        self.push(Ok(Response {
            status: 429,
            retry_after: Some(retry_after.into()),
            body: String::new(),
        }))
    }

    fn push(&self, r: Result<Response, TransportError>) -> &Self {
        self.script.lock().unwrap().push_back(r);
        self
    }

    fn sent(&self) -> Vec<HttpRequest> {
        self.sent.lock().unwrap().clone()
    }
}

impl Http for Mock {
    fn send(&self, req: &HttpRequest) -> Result<Response, TransportError> {
        self.sent.lock().unwrap().push(req.clone());
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected request {req:?}"))
    }
}

fn tokens(expires_at: u64) -> Tokens {
    Tokens {
        client_id: ID.into(),
        access_token: "old-access".into(),
        refresh_token: "old-refresh".into(),
        expires_at,
        scope: SCOPES.join(" "),
    }
}

struct Rig {
    client: Client<Mock>,
    mock: Mock,
    store: MemoryStore,
    sleeps: Arc<Mutex<Vec<Duration>>>,
}

fn rig(saved: Option<Tokens>) -> Rig {
    let mock = Mock::default();
    let store = MemoryStore::default();
    if let Some(t) = saved {
        store.save(&t).unwrap();
    }
    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let log = sleeps.clone();
    let client = Client::new(mock.clone(), ID.into(), Box::new(store.clone()))
        .with_time(|| NOW, move |d| log.lock().unwrap().push(d));
    Rig {
        client,
        mock,
        store,
        sleeps,
    }
}

/// Logged in with a token good for an hour.
fn logged_in() -> Rig {
    rig(Some(tokens(NOW + 3600)))
}

fn form_of(req: &HttpRequest) -> Vec<(String, String)> {
    let Body::Form(body) = &req.body else {
        panic!("not a form: {req:?}");
    };
    form::parse_query(body)
}

fn field<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

const TOKEN_REPLY: &str = r#"{"access_token":"new-access","token_type":"Bearer","expires_in":3600,"refresh_token":"new-refresh","scope":"user-library-read"}"#;
const REFRESH_NO_ROTATE: &str =
    r#"{"access_token":"new-access","token_type":"Bearer","expires_in":3600}"#;
const ME: &str =
    r#"{"id":"me","display_name":"Me Myself","uri":"spotify:user:me","product":"premium"}"#;

// ---- tokens ---------------------------------------------------------------

#[test]
fn code_exchange_posts_the_pkce_form_and_saves_tokens() {
    let mut r = rig(None);
    assert!(!r.client.is_logged_in());
    r.mock.reply(200, TOKEN_REPLY);
    assert_eq!(r.client.exchange_code("the-code", "the-verifier"), Ok(true));

    let sent = r.mock.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::Post);
    assert_eq!(sent[0].url, "https://accounts.spotify.com/api/token");
    assert_eq!(sent[0].bearer, None);
    let f = form_of(&sent[0]);
    assert_eq!(field(&f, "grant_type"), Some("authorization_code"));
    assert_eq!(field(&f, "code"), Some("the-code"));
    assert_eq!(field(&f, "redirect_uri"), Some(REDIRECT_URI));
    assert_eq!(field(&f, "client_id"), Some(ID));
    assert_eq!(field(&f, "code_verifier"), Some("the-verifier"));
    assert_eq!(field(&f, "client_secret"), None);

    assert!(r.client.is_logged_in());
    let saved = r.store.load(ID).unwrap();
    assert_eq!(saved.access_token, "new-access");
    assert_eq!(saved.refresh_token, "new-refresh");
    assert_eq!(saved.expires_at, NOW + 3600);
}

#[test]
fn failed_code_exchange_reports_and_stays_logged_out() {
    let mut r = rig(None);
    r.mock.reply(
        400,
        r#"{"error":"invalid_grant","error_description":"Invalid authorization code"}"#,
    );
    assert_eq!(
        r.client.exchange_code("bad", "v"),
        Err(Error::Login("Invalid authorization code".into()))
    );
    assert!(!r.client.is_logged_in());
    assert_eq!(r.store.load(ID), None);
}

#[test]
fn not_logged_in_sends_nothing() {
    let mut r = rig(None);
    assert_eq!(r.client.me(), Err(Error::NotLoggedIn));
    assert!(r.mock.sent().is_empty());
}

#[test]
fn tokens_saved_for_another_client_id_are_ignored() {
    let mut other = tokens(NOW + 3600);
    other.client_id = "someone-else".into();
    assert!(!rig(Some(other)).client.is_logged_in());
}

#[test]
fn stale_token_is_refreshed_before_the_call() {
    // Expires inside the margin: refresh first, no wasted 401.
    let mut r = rig(Some(tokens(NOW + 30)));
    r.mock.reply(200, REFRESH_NO_ROTATE).reply(200, ME);
    let me = r.client.me().unwrap();
    assert_eq!(me.name(), "Me Myself");

    let sent = r.mock.sent();
    let f = form_of(&sent[0]);
    assert_eq!(field(&f, "grant_type"), Some("refresh_token"));
    assert_eq!(field(&f, "refresh_token"), Some("old-refresh"));
    assert_eq!(field(&f, "client_id"), Some(ID));
    assert_eq!(sent[1].url, "https://api.spotify.com/v1/me");
    assert_eq!(sent[1].bearer.as_deref(), Some("new-access"));

    // No new refresh token in the reply: the old one is kept.
    let saved = r.store.load(ID).unwrap();
    assert_eq!(saved.refresh_token, "old-refresh");
    assert_eq!(saved.access_token, "new-access");
}

#[test]
fn unauthorized_refreshes_then_retries_once() {
    let mut r = logged_in();
    r.mock
        .reply(
            401,
            r#"{"error":{"status":401,"message":"The access token expired"}}"#,
        )
        .reply(200, TOKEN_REPLY)
        .reply(200, ME);
    assert_eq!(r.client.me().unwrap().id, "me");
    let sent = r.mock.sent();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].bearer.as_deref(), Some("old-access"));
    assert_eq!(sent[2].bearer.as_deref(), Some("new-access"));
    // A rotated refresh token is stored.
    assert_eq!(r.store.load(ID).unwrap().refresh_token, "new-refresh");
}

#[test]
fn a_second_unauthorized_is_an_error_not_a_loop() {
    let mut r = logged_in();
    let denied = r#"{"error":{"status":401,"message":"nope"}}"#;
    r.mock
        .reply(401, denied)
        .reply(200, TOKEN_REPLY)
        .reply(401, denied);
    assert_eq!(
        r.client.me(),
        Err(Error::Api {
            status: 401,
            message: "nope".into()
        })
    );
    assert_eq!(r.mock.sent().len(), 3);
}

#[test]
fn refused_refresh_logs_out() {
    let mut r = rig(Some(tokens(NOW)));
    r.mock.reply(
        400,
        r#"{"error":"invalid_grant","error_description":"Refresh token revoked"}"#,
    );
    assert_eq!(r.client.me(), Err(Error::LoginExpired));
    assert!(!r.client.is_logged_in());
    assert_eq!(r.store.load(ID), None);
    // And stays out without touching the network.
    assert_eq!(r.client.me(), Err(Error::NotLoggedIn));
    assert_eq!(r.mock.sent().len(), 1);
}

// ---- retries and errors ---------------------------------------------------

#[test]
fn short_retry_after_is_waited_out_once() {
    let mut r = logged_in();
    r.mock.rate_limit("2").reply(200, ME);
    assert!(r.client.me().is_ok());
    assert_eq!(*r.sleeps.lock().unwrap(), vec![Duration::from_secs(2)]);
}

#[test]
fn long_retry_after_goes_back_to_the_caller() {
    let mut r = logged_in();
    r.mock.rate_limit("30");
    assert_eq!(
        r.client.me(),
        Err(Error::RateLimited {
            retry_after: Duration::from_secs(30)
        })
    );
    assert!(r.sleeps.lock().unwrap().is_empty());
}

#[test]
fn rate_limited_twice_gives_up() {
    let mut r = logged_in();
    r.mock.rate_limit("1").rate_limit("junk");
    assert_eq!(
        r.client.me(),
        Err(Error::RateLimited {
            retry_after: Duration::from_secs(1)
        })
    );
    assert_eq!(r.sleeps.lock().unwrap().len(), 1);
}

#[test]
fn server_error_is_retried_once() {
    let mut r = logged_in();
    r.mock.reply(503, "").reply(200, ME);
    assert!(r.client.me().is_ok());

    let mut r = logged_in();
    r.mock.reply(502, "<html>").reply(503, "");
    assert_eq!(
        r.client.me(),
        Err(Error::Api {
            status: 503,
            message: "no details".into()
        })
    );
}

#[test]
fn transport_failure_is_offline() {
    let mut r = logged_in();
    r.mock
        .push(Err(TransportError("dns error: no route".into())));
    assert_eq!(
        r.client.me(),
        Err(Error::Offline("dns error: no route".into()))
    );
}

#[test]
fn statuses_and_bad_bodies_map_to_errors() {
    let mut r = logged_in();
    r.mock
        .reply(403, r#"{"error":{"status":403,"message":"Forbidden"}}"#)
        .reply(
            404,
            r#"{"error":{"status":404,"message":"Resource not found"}}"#,
        )
        .reply(200, "not json");
    assert_eq!(r.client.me(), Err(Error::Forbidden("Forbidden".into())));
    assert_eq!(
        r.client.me(),
        Err(Error::NotFound("Resource not found".into()))
    );
    assert!(matches!(r.client.me(), Err(Error::Decode(_))));
}

// ---- endpoints ------------------------------------------------------------

fn playlist_json(id: &str, count_field: &str) -> String {
    format!(
        r#"{{"id":"{id}","uri":"spotify:playlist:{id}","name":"List {id}","owner":{{"id":"me","display_name":"Me"}},
        "collaborative":false,"public":true,"snapshot_id":"snap-{id}",
        "{count_field}":{{"href":"x","total":7}},
        "images":[{{"url":"small","width":60}},{{"url":"big","width":640}}]}}"#
    )
}

#[test]
fn my_playlists_follows_every_page() {
    let mut r = logged_in();
    let page1 = format!(
        r#"{{"items":[{},{}],"offset":0,"limit":50,"total":3,"next":"https://api.spotify.com/v1/me/playlists?offset=2"}}"#,
        playlist_json("a", "items"),
        playlist_json("b", "tracks"),
    );
    let page2 = format!(
        r#"{{"items":[{},null],"offset":2,"limit":50,"total":3,"next":null}}"#,
        playlist_json("c", "items"),
    );
    r.mock.reply(200, &page1).reply(200, &page2);
    let lists = r.client.my_playlists().unwrap();
    assert_eq!(
        lists.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    // `items.total` (new) and `tracks.total` (deprecated) both count.
    assert!(lists.iter().all(|p| p.total == 7));
    assert_eq!(lists[0].image_url.as_deref(), Some("big"));
    assert_eq!(lists[0].snapshot_id, "snap-a");
    let me = User {
        id: "me".into(),
        display_name: None,
        uri: String::new(),
    };
    assert!(lists[0].editable_by(&me));

    let urls: Vec<_> = r.mock.sent().into_iter().map(|q| q.url).collect();
    assert_eq!(
        urls,
        [
            "https://api.spotify.com/v1/me/playlists?limit=50&offset=0",
            "https://api.spotify.com/v1/me/playlists?limit=50&offset=2",
        ]
    );
}

#[test]
fn playlist_tracks_reads_item_or_track_and_skips_holes() {
    let mut r = logged_in();
    let body = r#"{"items":[
        {"added_at":"t","is_local":false,"item":{"type":"track","id":"t1","uri":"spotify:track:t1","name":"One",
            "artists":[{"name":"A"},{"name":"B"}],"album":{"name":"Al","images":[{"url":"cover","width":300}]},"duration_ms":1000}},
        {"added_at":"t","is_local":false,"track":{"type":"track","id":"t2","uri":"spotify:track:t2","name":"Two",
            "artists":[],"album":{"name":"Al2","images":[]},"duration_ms":2000}},
        {"added_at":"t","is_local":true,"item":{"type":"track","id":null,"uri":"spotify:local:x","name":"Local",
            "artists":[{"name":"Me"}],"album":{"name":"","images":[]},"duration_ms":3000}},
        {"added_at":"t","is_local":false,"item":{"type":"episode","id":"e1","uri":"spotify:episode:e1","name":"Ep",
            "show":{"name":"Show","images":[]},"images":[{"url":"ep-art","width":64}],"duration_ms":4000}},
        {"added_at":"t","is_local":false,"item":null},
        null
    ],"offset":50,"limit":50,"total":200,"next":"https://next"}"#;
    r.mock.reply(200, body);
    let page = r.client.playlist_tracks("pl/1", 50).unwrap();
    assert_eq!(
        r.mock.sent()[0].url,
        "https://api.spotify.com/v1/playlists/pl%2F1/items?limit=50&offset=50&additional_types=track,episode"
    );
    assert_eq!(page.items.len(), 4);
    assert_eq!((page.offset, page.total, page.has_more), (50, 200, true));
    assert_eq!(page.next_offset(), 54);
    let [one, two, local, ep] = &page.items[..] else {
        unreachable!()
    };
    assert_eq!(one.artist_line(), "A, B");
    assert_eq!(one.image_url.as_deref(), Some("cover"));
    assert!(one.is_spotify());
    assert_eq!(two.album, "Al2");
    assert!(local.is_local && !local.is_spotify());
    assert_eq!(
        (ep.album.as_str(), ep.image_url.as_deref()),
        ("Show", Some("ep-art"))
    );
}

#[test]
fn create_playlist_posts_name_and_privacy() {
    let mut r = logged_in();
    r.mock.reply(201, &playlist_json("new", "items"));
    let list = r.client.create_playlist("lavatui test", false).unwrap();
    assert_eq!(list.id, "new");
    let sent = r.mock.sent();
    assert_eq!(sent[0].method, Method::Post);
    assert_eq!(sent[0].url, "https://api.spotify.com/v1/me/playlists");
    let Body::Json(body) = &sent[0].body else {
        panic!()
    };
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(
        v,
        serde_json::json!({ "name": "lavatui test", "public": false })
    );
}

fn uris(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("spotify:track:{i}")).collect()
}

#[test]
fn library_contains_chunks_by_forty_and_keeps_order() {
    let mut r = logged_in();
    let first = format!("[{}]", vec!["true"; 40].join(","));
    r.mock
        .reply(200, &first)
        .reply(200, "[false,true,false,false,false]");
    let found = r.client.library_contains(&uris(45)).unwrap();
    assert_eq!(found.len(), 45);
    assert!(found[..40].iter().all(|b| *b));
    assert_eq!(found[40..], [false, true, false, false, false]);

    let sent = r.mock.sent();
    assert_eq!(sent.len(), 2);
    let (path, query) = sent[1].url.split_once('?').unwrap();
    assert_eq!(path, "https://api.spotify.com/v1/me/library/contains");
    let q = form::parse_query(query);
    let expected = uris(45)[40..].join(",");
    assert_eq!(field(&q, "uris"), Some(expected.as_str()));
}

#[test]
fn like_and_unlike_use_the_library_endpoint() {
    let mut r = logged_in();
    r.mock.reply(200, "").reply(200, "");
    r.client.library_save(&uris(1)).unwrap();
    r.client.library_remove(&uris(1)).unwrap();
    let sent = r.mock.sent();
    assert_eq!(sent[0].method, Method::Put);
    assert_eq!(sent[1].method, Method::Delete);
    for req in sent {
        assert_eq!(
            req.url,
            "https://api.spotify.com/v1/me/library?uris=spotify%3Atrack%3A0"
        );
        assert_eq!(req.body, Body::Empty);
    }
}

#[test]
fn add_to_playlist_posts_json_in_chunks_of_a_hundred() {
    let mut r = logged_in();
    r.mock
        .reply(201, r#"{"snapshot_id":"s1"}"#)
        .reply(201, r#"{"snapshot_id":"s2"}"#);
    assert_eq!(r.client.add_to_playlist("pl", &uris(150)).unwrap(), "s2");
    let sent = r.mock.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].method, Method::Post);
    assert_eq!(sent[0].url, "https://api.spotify.com/v1/playlists/pl/items");
    let Body::Json(body) = &sent[1].body else {
        panic!("{:?}", sent[1].body)
    };
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    let sent_uris = v["uris"].as_array().unwrap();
    assert_eq!(sent_uris.len(), 50);
    assert_eq!(sent_uris[0], "spotify:track:100");
}

#[test]
fn search_clamps_the_limit_and_encodes_the_query() {
    let mut r = logged_in();
    let body = r#"{"tracks":{"items":[{"id":"t","uri":"spotify:track:t","name":"N","artists":[{"name":"Daft Punk"}],
        "album":{"name":"D","images":[]},"duration_ms":1}],"offset":0,"total":1,"next":null}}"#;
    r.mock.reply(200, body).reply(200, body).reply(200, "{}");
    let page = r.client.search_tracks("one more time", 50, 0).unwrap();
    assert_eq!(page.items[0].artists, ["Daft Punk"]);
    let tracks = r.client.artist_tracks("Daft \"Punk\"").unwrap();
    assert_eq!(tracks.len(), 1);
    // No `tracks` key at all: an empty page, not an error.
    assert!(r.client.search_tracks("x", 0, 0).unwrap().items.is_empty());

    let sent = r.mock.sent();
    let q0 = form::parse_query(sent[0].url.split_once('?').unwrap().1);
    assert_eq!(field(&q0, "q"), Some("one more time"));
    assert_eq!(field(&q0, "type"), Some("track"));
    assert_eq!(field(&q0, "limit"), Some("10"));
    let q1 = form::parse_query(sent[1].url.split_once('?').unwrap().1);
    assert_eq!(field(&q1, "q"), Some("artist:\"Daft Punk\""));
    let q2 = form::parse_query(sent[2].url.split_once('?').unwrap().1);
    assert_eq!(field(&q2, "limit"), Some("1"));
}

#[test]
fn player_state_reads_shuffle_repeat_and_nothing_playing() {
    let mut r = logged_in();
    r.mock
        .reply(
            200,
            r#"{"shuffle_state":true,"repeat_state":"context","is_playing":true,
                "device":{"name":"Mac","type":"Computer"},"item":{"uri":"spotify:track:x"},
                "actions":{"disallows":{"toggling_shuffle":true,"resuming":true}}}"#,
        )
        .reply(204, "");
    let state = r.client.player().unwrap().unwrap();
    assert!(state.shuffle && state.is_playing);
    assert_eq!(state.repeat, Repeat::Context);
    assert_eq!(state.device.as_deref(), Some("Mac"));
    assert_eq!(state.item_uri.as_deref(), Some("spotify:track:x"));
    assert!(state.shuffle_blocked && !state.repeat_blocked);
    assert_eq!(r.client.player().unwrap(), None, "204: nothing playing");
    assert_eq!(r.mock.sent()[0].url, "https://api.spotify.com/v1/me/player");
}

#[test]
fn player_setters_put_with_the_state_in_the_query() {
    let mut r = logged_in();
    r.mock.reply(204, "").reply(204, "").reply(204, "");
    r.client.set_shuffle(true).unwrap();
    r.client.set_repeat(Repeat::Track).unwrap();
    r.client
        .play("spotify:playlist:p", Some("spotify:track:t"))
        .unwrap();
    let sent = r.mock.sent();
    assert!(sent.iter().all(|s| s.method == Method::Put));
    assert_eq!(
        sent[0].url,
        "https://api.spotify.com/v1/me/player/shuffle?state=true"
    );
    assert_eq!(
        sent[1].url,
        "https://api.spotify.com/v1/me/player/repeat?state=track"
    );
    assert_eq!(sent[2].url, "https://api.spotify.com/v1/me/player/play");
    let Body::Json(body) = &sent[2].body else {
        panic!("{:?}", sent[2].body)
    };
    let v: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(v["context_uri"], "spotify:playlist:p");
    assert_eq!(v["offset"]["uri"], "spotify:track:t");
}

#[test]
fn a_refused_player_call_is_forbidden() {
    let mut r = logged_in();
    r.mock.reply(
        403,
        r#"{"error":{"status":403,"message":"Player command failed: Premium required","reason":"PREMIUM_REQUIRED"}}"#,
    );
    assert_eq!(
        r.client.set_shuffle(false),
        Err(Error::Forbidden(
            "Player command failed: Premium required".into()
        ))
    );
}

// ---- the worker -----------------------------------------------------------

fn handle(saved: Option<Tokens>) -> (SpotifyWeb, Mock, MemoryStore) {
    let mock = Mock::default();
    let store = MemoryStore::default();
    if let Some(t) = saved {
        store.save(&t).unwrap();
    }
    let (m, s) = (mock.clone(), store.clone());
    let web = SpotifyWeb::spawn(ID.into(), move || {
        Client::new(m, ID.into(), Box::new(s)).with_time(|| NOW, |_| {})
    });
    (web, mock, store)
}

fn next(web: &SpotifyWeb) -> Event {
    web.poll_timeout(Duration::from_secs(5)).expect("an event")
}

#[test]
fn worker_answers_requests_in_order_by_id() {
    let (mut web, mock, _) = handle(Some(tokens(NOW + 3600)));
    mock.reply(200, ME).reply(200, "[true]");
    let a = web.request(Request::Me);
    let b = web.request(Request::LibraryContains { uris: uris(1) });
    assert_ne!(a, b);
    let Event::Reply {
        id,
        result: Ok(Reply::User(user)),
    } = next(&web)
    else {
        panic!()
    };
    assert_eq!((id, user.id.as_str()), (a, "me"));
    assert_eq!(
        next(&web),
        Event::Reply {
            id: b,
            result: Ok(Reply::Contains(vec![true]))
        }
    );
    assert!(web.is_logged_in());
    assert_eq!(web.poll(), None);
}

#[test]
fn worker_turns_a_callback_code_into_a_login() {
    let (web, mock, store) = handle(None);
    assert!(!web.is_logged_in());
    mock.reply(200, TOKEN_REPLY);
    web.jobs
        .send(Job::Code {
            code: "c".into(),
            verifier: "v".into(),
        })
        .unwrap();
    assert_eq!(next(&web), Event::LoggedIn { saved: true });
    assert!(web.is_logged_in());
    assert!(store.load(ID).is_some());

    web.jobs
        .send(Job::LoginFailed(Error::Login("cancelled".into())))
        .unwrap();
    assert_eq!(
        next(&web),
        Event::LoginFailed(Error::Login("cancelled".into()))
    );
}

#[test]
fn worker_reports_an_expired_login_and_a_logout() {
    let (mut web, mock, store) = handle(Some(tokens(NOW)));
    mock.reply(400, r#"{"error":"invalid_grant"}"#);
    let id = web.request(Request::Me);
    assert_eq!(
        next(&web),
        Event::Reply {
            id,
            result: Err(Error::LoginExpired)
        }
    );
    assert_eq!(next(&web), Event::LoggedOut { expired: true });
    assert!(!web.is_logged_in());
    assert_eq!(store.load(ID), None);

    web.logout();
    assert_eq!(next(&web), Event::LoggedOut { expired: false });
}

// ---- live -----------------------------------------------------------------

/// The real transport against Spotify's token endpoint with a made-up
/// Client ID: TLS works and the refusal maps to a login error.
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

/// The whole thing against a real account. Needs `LAVATUI_SPOTIFY_CLIENT_ID`
/// and someone to click Agree in the browser (tokens are kept in
/// `LAVATUI_LIVE_TOKEN_FILE`, default a temp file, so a rerun skips that).
/// Reads the profile, playlists and one owned playlist; likes/unlikes the
/// track playing in the Spotify app (else the first one read) and restores
/// its liked state; adds it to a private "lavatui test" playlist it creates
/// (or reuses). Never touches any other playlist.
/// `cargo test -- --ignored --nocapture live_account`
#[test]
#[ignore = "needs a Spotify account and a browser"]
fn live_account() {
    use super::store::FileStore;
    const TEST_PLAYLIST: &str = "lavatui test";

    let client_id = client_id_from_env().expect("set LAVATUI_SPOTIFY_CLIENT_ID");
    let file = std::env::var_os("LAVATUI_LIVE_TOKEN_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("lavatui-live-spotify-tokens.json"));
    let id = client_id.clone();
    let mut web = SpotifyWeb::spawn(client_id, move || {
        Client::new(super::http::Ureq::new(), id, Box::new(FileStore(file)))
    });

    // The worker loads the saved login first; a request is the barrier.
    let ask = |web: &mut SpotifyWeb, req: Request| -> Reply {
        let want = web.request(req.clone());
        loop {
            match web.poll_timeout(Duration::from_secs(60)) {
                Some(Event::Reply { id, result }) if id == want => {
                    return result.unwrap_or_else(|e| panic!("{req:?}: {e}"));
                }
                Some(other) => eprintln!("event: {other:?}"),
                None => panic!("{req:?}: no reply"),
            }
        }
    };
    let want = web.request(Request::Me);
    let me = match web.poll_timeout(Duration::from_secs(60)) {
        Some(Event::Reply {
            id,
            result: Ok(Reply::User(me)),
        }) if id == want => me,
        _ => {
            let url = web.login().expect("login");
            eprintln!("LOGIN: finish in the browser (opened): {url}");
            match web.poll_timeout(Duration::from_secs(330)) {
                Some(Event::LoggedIn { saved }) => eprintln!("logged in (saved: {saved})"),
                other => panic!("login: {other:?}"),
            }
            let Reply::User(me) = ask(&mut web, Request::Me) else {
                panic!()
            };
            me
        }
    };
    eprintln!("me: {} ({})", me.name(), me.id);

    let Reply::Playlists(lists) = ask(&mut web, Request::MyPlaylists) else {
        panic!()
    };
    eprintln!("playlists: {}", lists.len());

    let mut sample = None;
    if let Some(owned) = lists
        .iter()
        .find(|p| p.editable_by(&me) && p.total > 0 && p.name != TEST_PLAYLIST)
    {
        let Reply::Tracks(page) = ask(
            &mut web,
            Request::PlaylistTracks {
                playlist_id: owned.id.clone(),
                offset: 0,
            },
        ) else {
            panic!()
        };
        eprintln!(
            "playlist {:?}: {} of {} items read, first: {:?}",
            owned.name,
            page.items.len(),
            page.total,
            page.items
                .first()
                .map(|t| format!("{} - {}", t.artist_line(), t.name))
        );
        sample = page
            .items
            .into_iter()
            .find(|t| t.uri.starts_with("spotify:track:"));
    } else {
        eprintln!("no owned, non-empty playlist to read");
    }

    let playing = std::process::Command::new("osascript")
        .args(["-e", "if application \"Spotify\" is running then tell application \"Spotify\" to return id of current track"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|uri| uri.starts_with("spotify:track:"));
    let uri = match (&playing, &sample) {
        (Some(uri), _) => {
            eprintln!("now playing: {uri}");
            uri.clone()
        }
        (None, Some(t)) => {
            eprintln!("nothing playing; using {} ({})", t.uri, t.name);
            t.uri.clone()
        }
        (None, None) => panic!("no track to like: play something in Spotify"),
    };
    let uris = vec![uri.clone()];
    let liked = |web: &mut SpotifyWeb, ask: &dyn Fn(&mut SpotifyWeb, Request) -> Reply| {
        let Reply::Contains(v) = ask(
            web,
            Request::LibraryContains {
                uris: vec![uri.clone()],
            },
        ) else {
            panic!()
        };
        v[0]
    };
    let original = liked(&mut web, &ask);
    let (first, back) = if original {
        (
            Request::Unlike { uris: uris.clone() },
            Request::Like { uris: uris.clone() },
        )
    } else {
        (
            Request::Like { uris: uris.clone() },
            Request::Unlike { uris: uris.clone() },
        )
    };
    ask(&mut web, first);
    let flipped = liked(&mut web, &ask);
    ask(&mut web, back);
    let restored = liked(&mut web, &ask);
    eprintln!("liked: {original} -> {flipped} -> {restored}");
    assert_eq!(flipped, !original);
    assert_eq!(restored, original);

    let test_list = match lists
        .iter()
        .find(|p| p.name == TEST_PLAYLIST && p.owner_id == me.id)
    {
        Some(p) => {
            eprintln!("reusing playlist {:?} ({})", p.name, p.id);
            p.clone()
        }
        None => {
            let Reply::Playlist(p) = ask(
                &mut web,
                Request::CreatePlaylist {
                    name: TEST_PLAYLIST.into(),
                    public: false,
                },
            ) else {
                panic!()
            };
            eprintln!("created private playlist {:?} ({})", p.name, p.id);
            p
        }
    };
    let Reply::Snapshot(snapshot) = ask(
        &mut web,
        Request::AddToPlaylist {
            playlist_id: test_list.id.clone(),
            uris: uris.clone(),
        },
    ) else {
        panic!()
    };
    let Reply::Tracks(page) = ask(
        &mut web,
        Request::PlaylistTracks {
            playlist_id: test_list.id.clone(),
            offset: 0,
        },
    ) else {
        panic!()
    };
    eprintln!(
        "added (snapshot {snapshot}); test playlist now has {} items",
        page.total
    );
    assert!(page.items.iter().any(|t| t.uri == uri));
}

/// The library UI's calls against a real account, non-destructively:
/// reads the playlists and one owned playlist's first page, likes /
/// unlikes the playing track (restoring it), adds it to the private
/// "lavatui test" playlist (`LAVATUI_TEST_PLAYLIST`, else found by name)
/// and nowhere else, then reads the player and flips shuffle and repeat
/// for a moment, putting both back. Same token file as `live_account`.
/// `LAVATUI_SPOTIFY_CLIENT_ID=… cargo test -- --ignored --nocapture live_library`
#[test]
#[ignore = "needs a Spotify account and a browser"]
fn live_library() {
    use super::store::FileStore;
    let client_id = client_id_from_env().expect("set LAVATUI_SPOTIFY_CLIENT_ID");
    let file = std::env::var_os("LAVATUI_LIVE_TOKEN_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("lavatui-live-spotify-tokens.json"));
    let id = client_id.clone();
    let mut web = SpotifyWeb::spawn(client_id, move || {
        Client::new(super::http::Ureq::new(), id, Box::new(FileStore(file)))
    });
    let try_ask = |web: &mut SpotifyWeb, req: Request| -> Result<Reply, Error> {
        let want = web.request(req.clone());
        loop {
            match web.poll_timeout(Duration::from_secs(60)) {
                Some(Event::Reply { id, result }) if id == want => return result,
                Some(other) => eprintln!("event: {other:?}"),
                None => panic!("{req:?}: no reply"),
            }
        }
    };
    let ask = |web: &mut SpotifyWeb, req: Request| -> Reply {
        try_ask(web, req.clone()).unwrap_or_else(|e| panic!("{req:?}: {e}"))
    };
    let login = |web: &mut SpotifyWeb| {
        let url = web.login().expect("login");
        eprintln!("LOGIN: finish in the browser (opened): {url}");
        match web.poll_timeout(Duration::from_secs(330)) {
            Some(Event::LoggedIn { saved }) => eprintln!("logged in (saved: {saved})"),
            other => panic!("login: {other:?}"),
        }
    };
    let me = match try_ask(&mut web, Request::Me) {
        Ok(Reply::User(me)) => me,
        _ => {
            login(&mut web);
            let Reply::User(me) = ask(&mut web, Request::Me) else {
                panic!()
            };
            me
        }
    };
    eprintln!("me: {}", me.name());

    // Player first: a login without the playback scopes logs in again.
    let mut player = try_ask(&mut web, Request::Player);
    if matches!(&player, Err(Error::Forbidden(why)) if why.contains("scope")) {
        eprintln!("token lacks the playback scopes: logging in again");
        web.logout();
        let _ = web.poll_timeout(Duration::from_secs(5));
        login(&mut web);
        player = try_ask(&mut web, Request::Player);
    }

    let Reply::Playlists(lists) = ask(&mut web, Request::MyPlaylists) else {
        panic!()
    };
    let editable = lists.iter().filter(|p| p.editable_by(&me)).count();
    eprintln!("playlists: {} ({editable} editable)", lists.len());
    let test_id = std::env::var("LAVATUI_TEST_PLAYLIST")
        .ok()
        .unwrap_or_else(|| {
            lists
                .iter()
                .find(|p| p.name == "lavatui test" && p.owner_id == me.id)
                .expect("a 'lavatui test' playlist")
                .id
                .clone()
        });
    if let Some(owned) = lists
        .iter()
        .find(|p| p.editable_by(&me) && p.total > 0 && p.id != test_id)
    {
        let Reply::Tracks(page) = ask(
            &mut web,
            Request::PlaylistTracks {
                playlist_id: owned.id.clone(),
                offset: 0,
            },
        ) else {
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
        let r = try_ask(
            &mut web,
            Request::PlaylistTracks {
                playlist_id: followed.id.clone(),
                offset: 0,
            },
        );
        eprintln!("followed {:?} items: {:?}", followed.name, r.map(|_| "ok"));
    }

    let playing = std::process::Command::new("osascript")
        .args(["-e", "if application \"Spotify\" is running then tell application \"Spotify\" to return id of current track"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|uri| uri.starts_with("spotify:track:"))
        .expect("play a track in Spotify first");
    eprintln!("playing: {playing}");
    let uris = vec![playing.clone()];
    let liked = |web: &mut SpotifyWeb| {
        let Reply::Contains(v) = ask(web, Request::LibraryContains { uris: uris.clone() }) else {
            panic!()
        };
        v[0]
    };
    let original = liked(&mut web);
    let (flip, back) = if original {
        (
            Request::Unlike { uris: uris.clone() },
            Request::Like { uris: uris.clone() },
        )
    } else {
        (
            Request::Like { uris: uris.clone() },
            Request::Unlike { uris: uris.clone() },
        )
    };
    ask(&mut web, flip);
    let flipped = liked(&mut web);
    ask(&mut web, back);
    let restored = liked(&mut web);
    eprintln!("liked: {original} -> {flipped} -> {restored}");
    assert_eq!((flipped, restored), (!original, original));

    let Reply::Snapshot(snap) = ask(
        &mut web,
        Request::AddToPlaylist {
            playlist_id: test_id.clone(),
            uris: uris.clone(),
        },
    ) else {
        panic!()
    };
    eprintln!("added to lavatui test ({test_id}), snapshot {snap}");

    match player {
        Ok(Reply::Player(Some(state))) => {
            eprintln!("player: {state:?}");
            if state.shuffle_blocked || state.repeat_blocked {
                eprintln!("(this context blocks some toggles: refusals expected)");
            }
            let shuffle = try_ask(&mut web, Request::SetShuffle(!state.shuffle));
            eprintln!("shuffle -> {}: {shuffle:?}", !state.shuffle);
            std::thread::sleep(Duration::from_millis(800));
            if let Ok(Reply::Player(Some(now))) = try_ask(&mut web, Request::Player) {
                eprintln!("read back: shuffle {} repeat {:?}", now.shuffle, now.repeat);
            }
            let repeat = if state.repeat == Repeat::Off {
                Repeat::Context
            } else {
                Repeat::Off
            };
            let r = try_ask(&mut web, Request::SetRepeat(repeat));
            eprintln!("repeat -> {repeat:?}: {r:?}");
            std::thread::sleep(Duration::from_millis(800));
            if let Ok(Reply::Player(Some(now))) = try_ask(&mut web, Request::Player) {
                eprintln!("read back: shuffle {} repeat {:?}", now.shuffle, now.repeat);
            }
            let a = try_ask(&mut web, Request::SetShuffle(state.shuffle));
            let b = try_ask(&mut web, Request::SetRepeat(state.repeat));
            std::thread::sleep(Duration::from_millis(800));
            let Reply::Player(Some(after)) = ask(&mut web, Request::Player) else {
                panic!()
            };
            eprintln!(
                "restored ({a:?}, {b:?}): shuffle {} repeat {:?}",
                after.shuffle, after.repeat
            );
            if shuffle.is_ok() {
                assert_eq!((after.shuffle, after.repeat), (state.shuffle, state.repeat));
            }
        }
        other => eprintln!("player: {other:?}"),
    }
}

/// `live_library`'s player part in a playlist context, where Spotify
/// allows the toggles: plays "lavatui test" (`LAVATUI_TEST_PLAYLIST`) for
/// a few seconds through `PUT /me/player/play`, flips shuffle and repeat
/// and puts them back, then has the desktop app play the original track
/// from where it was (paused again if it was). Opt-in:
/// `LAVATUI_SPOTIFY_CLIENT_ID=… LAVATUI_TEST_PLAYLIST=… cargo test -- --ignored --nocapture live_player_in_a_playlist`
#[test]
#[ignore = "needs a Spotify account; changes playback for a few seconds"]
fn live_player_in_a_playlist() {
    use super::store::FileStore;
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

    let client_id = client_id_from_env().expect("set LAVATUI_SPOTIFY_CLIENT_ID");
    let playlist = std::env::var("LAVATUI_TEST_PLAYLIST").expect("set LAVATUI_TEST_PLAYLIST");
    let file = std::env::temp_dir().join("lavatui-live-spotify-tokens.json");
    let id = client_id.clone();
    let mut web = SpotifyWeb::spawn(client_id, move || {
        Client::new(super::http::Ureq::new(), id, Box::new(FileStore(file)))
    });
    let ask = |web: &mut SpotifyWeb, req: Request| -> Result<Reply, Error> {
        let want = web.request(req);
        loop {
            match web.poll_timeout(Duration::from_secs(60)) {
                Some(Event::Reply { id, result }) if id == want => return result,
                Some(_) => {}
                None => panic!("no reply"),
            }
        }
    };
    let read = |web: &mut SpotifyWeb| match ask(web, Request::Player) {
        Ok(Reply::Player(Some(p))) => p,
        other => panic!("player: {other:?}"),
    };
    let original = read(&mut web);
    let played = ask(
        &mut web,
        Request::Play {
            context_uri: format!("spotify:playlist:{playlist}"),
            offset_uri: None,
        },
    );
    eprintln!("play the test playlist: {played:?}");
    std::thread::sleep(Duration::from_millis(1500));
    let p = read(&mut web);
    eprintln!("in the playlist: {p:?}");
    let s = ask(&mut web, Request::SetShuffle(!p.shuffle));
    let r = ask(
        &mut web,
        Request::SetRepeat(if p.repeat == Repeat::Off {
            Repeat::Context
        } else {
            Repeat::Off
        }),
    );
    std::thread::sleep(Duration::from_millis(800));
    let flipped = read(&mut web);
    eprintln!(
        "flip: {s:?} {r:?} -> shuffle {} repeat {:?}",
        flipped.shuffle, flipped.repeat
    );
    let _ = ask(&mut web, Request::SetShuffle(original.shuffle));
    let _ = ask(&mut web, Request::SetRepeat(original.repeat));

    // Put the desktop app back where it was.
    osa(&format!(
        "tell application \"Spotify\"\nplay track \"{track}\"\ndelay 0.8\nset player position to {position}\nend tell"
    ));
    if state != "playing" {
        osa("tell application \"Spotify\" to pause");
    }
    std::thread::sleep(Duration::from_millis(800));
    let after = read(&mut web);
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

/// Read-only: the "lavatui test" playlist's items and whether the track
/// playing in Spotify is liked (to check what a UI run did).
/// `LAVATUI_SPOTIFY_CLIENT_ID=… LAVATUI_TEST_PLAYLIST=… cargo test -- --ignored --nocapture live_peek`
#[test]
#[ignore = "needs a Spotify account"]
fn live_peek() {
    use super::store::FileStore;
    let client_id = client_id_from_env().expect("set LAVATUI_SPOTIFY_CLIENT_ID");
    let playlist = std::env::var("LAVATUI_TEST_PLAYLIST").expect("set LAVATUI_TEST_PLAYLIST");
    let file = std::env::temp_dir().join("lavatui-live-spotify-tokens.json");
    let id = client_id.clone();
    let mut web = SpotifyWeb::spawn(client_id, move || {
        Client::new(super::http::Ureq::new(), id, Box::new(FileStore(file)))
    });
    let mut ask = |req: Request| {
        let want = web.request(req);
        loop {
            match web.poll_timeout(Duration::from_secs(60)) {
                Some(Event::Reply { id, result }) if id == want => return result,
                Some(_) => {}
                None => panic!("no reply"),
            }
        }
    };
    if let Ok(Reply::Tracks(page)) = ask(Request::PlaylistTracks {
        playlist_id: playlist,
        offset: 0,
    }) {
        for t in &page.items {
            eprintln!(
                "in lavatui test: {} – {} ({})",
                t.name,
                t.artist_line(),
                t.uri
            );
        }
    }
    let playing = std::process::Command::new("osascript")
        .args([
            "-e",
            "tell application \"Spotify\" to return id of current track",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    let liked = ask(Request::LibraryContains {
        uris: vec![playing.clone()],
    });
    eprintln!("{playing} liked: {liked:?}");
}
