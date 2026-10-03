//! The blocking client: tokens (exchange, refresh, persist), one `call` that
//! handles 401 → refresh → retry, 429 `Retry-After` and a flaky 5xx, and
//! the endpoints. Runs on the worker thread; generic over [`Http`] so the
//! tests script the server.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::error::{self, Error};
use super::form;
use super::http::{Body, Http, Method, Request, Response};
use super::store::{TokenStore, Tokens};
use super::types::{
    Page, PlayerState, Playlist, RawPage, RawPlayer, RawPlaylist, RawPlaylistItem, RawSearch,
    RawSnapshot, RawUriItem, Repeat, Track, Uris, User,
};
use super::{ACCOUNTS_BASE, API_BASE, REDIRECT_URI};

/// Refresh this long before the access token runs out (clock skew, slow
/// requests).
const EXPIRY_MARGIN: u64 = 60;
/// Longest `Retry-After` we sit out inline (once); longer ones go back to
/// the caller as [`Error::RateLimited`].
const MAX_INLINE_WAIT: Duration = Duration::from_secs(3);
/// Pause before the one retry of a 5xx.
const SERVER_ERROR_PAUSE: Duration = Duration::from_millis(500);
/// Page size for paged reads (the API's maximum).
pub const PAGE: u32 = 50;
/// Stop following `next` after this many pages (2500 playlists).
const MAX_PAGES: u32 = 50;
/// `/me/library` takes at most 40 URIs per call.
const LIBRARY_CHUNK: usize = 40;
/// Add-to-playlist takes at most 100 URIs per call.
const PLAYLIST_CHUNK: usize = 100;
/// What [`Client::playlist_uris`] asks for: each item's URI (under `item`
/// since Feb 2026, `track` before) and the paging.
const URI_FIELDS: &str = "items(item(uri),track(uri)),next,total";
/// Search's `limit` maximum for development-mode apps (Feb 2026).
pub const SEARCH_MAX: u32 = 10;

pub struct Client<H> {
    http: H,
    client_id: String,
    tokens: Option<Tokens>,
    store: Box<dyn TokenStore>,
    /// The store has been read (or overwritten): [`Self::load`] is a no-op.
    loaded: bool,
    /// Spotify said to wait until then (unix seconds): every API call
    /// fails at once with [`Error::RateLimited`] until it's over, whoever
    /// asks.
    quiet_until: u64,
    /// Unix seconds.
    clock: Box<dyn Fn() -> u64 + Send>,
    sleep: Box<dyn Fn(Duration) + Send>,
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

impl<H: Http> Client<H> {
    /// A client for `client_id`, logged in if `store` holds its tokens.
    pub fn new(http: H, client_id: String, store: Box<dyn TokenStore>) -> Self {
        let mut client = Self::unloaded(http, client_id, store);
        client.load();
        client
    }

    /// A client that hasn't read `store` yet: [`Self::load`] does, when
    /// the saved login is first needed (on macOS, reading the Keychain can
    /// make the system ask the user, lava-1xk.38).
    pub fn unloaded(http: H, client_id: String, store: Box<dyn TokenStore>) -> Self {
        Self {
            http,
            client_id,
            tokens: None,
            store,
            loaded: false,
            quiet_until: 0,
            clock: Box::new(unix_now),
            sleep: Box::new(std::thread::sleep),
        }
    }

    /// Reads the saved login, the first time only.
    pub fn load(&mut self) {
        if !std::mem::replace(&mut self.loaded, true) {
            self.tokens = self.store.load(&self.client_id);
        }
    }

    /// Keeps the login in `store` from now on: read from the old store (if
    /// it wasn't yet), forgotten there, saved in the new one. `false`:
    /// logged in, but `store` couldn't keep it (this session only).
    pub fn move_to(&mut self, store: Box<dyn TokenStore>) -> bool {
        self.load();
        self.store.clear();
        self.store = store;
        self.tokens.is_none() || self.persist()
    }

    #[cfg(test)]
    pub fn with_time(
        mut self,
        clock: impl Fn() -> u64 + Send + 'static,
        sleep: impl Fn(Duration) + Send + 'static,
    ) -> Self {
        self.clock = Box::new(clock);
        self.sleep = Box::new(sleep);
        self
    }

    #[cfg(test)]
    pub fn http(&self) -> &H {
        &self.http
    }

    pub fn is_logged_in(&self) -> bool {
        self.tokens.is_some()
    }

    /// Forgets the tokens, here and in the store.
    pub fn logout(&mut self) {
        self.loaded = true;
        self.tokens = None;
        self.store.clear();
    }

    /// Trades the callback's code for tokens. `Ok(false)`: logged in, but
    /// the tokens couldn't be saved (this session only).
    pub fn exchange_code(&mut self, code: &str, verifier: &str) -> Result<bool, Error> {
        let body = form::pairs(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", &self.client_id),
            ("code_verifier", verifier),
        ]);
        let resp = self.token_request(body)?;
        if resp.status != 200 {
            return Err(match resp.status {
                429 => rate_limited(&resp),
                _ => Error::Login(error::message(&resp.body)),
            });
        }
        let reply: TokenReply = decode(&resp.body)?;
        let Some(refresh_token) = reply.refresh_token.clone() else {
            return Err(Error::Login("no refresh token in the reply".into()));
        };
        self.loaded = true;
        self.set_tokens(reply, refresh_token);
        Ok(self.persist())
    }

    /// Gets a new access token with the refresh token. A refused refresh
    /// (expired after six months, revoked) logs out.
    fn refresh(&mut self) -> Result<(), Error> {
        let Some(old) = &self.tokens else {
            return Err(Error::NotLoggedIn);
        };
        let old_refresh = old.refresh_token.clone();
        let body = form::pairs(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &old_refresh),
            ("client_id", &self.client_id),
        ]);
        let resp = self.token_request(body)?;
        match resp.status {
            200 => {
                let reply: TokenReply = decode(&resp.body)?;
                // Spotify may or may not rotate it; keep the old one if not.
                let refresh_token = reply.refresh_token.clone().unwrap_or(old_refresh);
                self.set_tokens(reply, refresh_token);
                self.persist();
                Ok(())
            }
            400 | 401 => {
                let code = error::oauth_code(&resp.body);
                self.logout();
                Err(if code.as_deref() == Some("invalid_grant") {
                    Error::LoginExpired
                } else {
                    Error::Login(error::message(&resp.body))
                })
            }
            429 => Err(rate_limited(&resp)),
            status => Err(error::from_status(status, &resp.body)),
        }
    }

    fn token_request(&self, body: String) -> Result<Response, Error> {
        self.http
            .send(&Request {
                method: Method::Post,
                url: format!("{ACCOUNTS_BASE}/api/token"),
                bearer: None,
                body: Body::Form(body),
            })
            .map_err(|e| Error::Offline(e.0))
    }

    fn set_tokens(&mut self, reply: TokenReply, refresh_token: String) {
        self.tokens = Some(Tokens {
            client_id: self.client_id.clone(),
            access_token: reply.access_token,
            refresh_token,
            expires_at: (self.clock)() + reply.expires_in,
            scope: reply.scope,
        });
    }

    fn persist(&self) -> bool {
        self.tokens
            .as_ref()
            .is_some_and(|t| self.store.save(t).is_ok())
    }

    /// One authorized API call: refreshes a stale token first; on 401
    /// refreshes and retries once; on 429 waits out a short `Retry-After`
    /// once, and a longer one keeps every call off the network until it's
    /// over; retries a 5xx once. Returns the 2xx body.
    fn call(&mut self, method: Method, path: &str, body: Body) -> Result<String, Error> {
        let url = format!("{API_BASE}{path}");
        let (mut refreshed, mut waited, mut retried) = (false, false, false);
        loop {
            let now = (self.clock)();
            if now < self.quiet_until {
                let retry_after = Duration::from_secs(self.quiet_until - now);
                return Err(Error::RateLimited { retry_after });
            }
            let tokens = self.tokens.as_ref().ok_or(Error::NotLoggedIn)?;
            if (self.clock)() + EXPIRY_MARGIN >= tokens.expires_at {
                self.refresh()?;
                refreshed = true;
            }
            let bearer = self.tokens.as_ref().map(|t| t.access_token.clone());
            let resp = self
                .http
                .send(&Request {
                    method,
                    url: url.clone(),
                    bearer,
                    body: body.clone(),
                })
                .map_err(|e| Error::Offline(e.0))?;
            match resp.status {
                200..=299 => return Ok(resp.body),
                401 if !refreshed => {
                    self.refresh()?;
                    refreshed = true;
                }
                429 => match retry_after(&resp) {
                    wait if !waited && wait <= MAX_INLINE_WAIT => {
                        (self.sleep)(wait);
                        waited = true;
                    }
                    wait => {
                        self.quiet_until = now + wait.as_secs().max(1);
                        return Err(Error::RateLimited { retry_after: wait });
                    }
                },
                500 | 502 | 503 | 504 if !retried => {
                    (self.sleep)(SERVER_ERROR_PAUSE);
                    retried = true;
                }
                status => return Err(error::from_status(status, &resp.body)),
            }
        }
    }

    fn get<T: DeserializeOwned>(&mut self, path: &str) -> Result<T, Error> {
        decode(&self.call(Method::Get, path, Body::Empty)?)
    }

    // ---- endpoints --------------------------------------------------------

    /// `GET /me`.
    pub fn me(&mut self) -> Result<User, Error> {
        self.get("/me")
    }

    /// All of the user's playlists (owned and followed), every page.
    pub fn my_playlists(&mut self) -> Result<Vec<Playlist>, Error> {
        let mut all = Vec::new();
        let mut offset = 0;
        for _ in 0..MAX_PAGES {
            let page: RawPage<RawPlaylist> =
                self.get(&format!("/me/playlists?limit={PAGE}&offset={offset}"))?;
            let page = page.map(|p| Some(Playlist::from(p)));
            let done = !page.has_more;
            offset = page.next_offset;
            all.extend(page.items);
            if done {
                break;
            }
        }
        Ok(all)
    }

    /// Creates a playlist owned by the user (`POST /me/playlists`).
    pub fn create_playlist(&mut self, name: &str, public: bool) -> Result<Playlist, Error> {
        let body = serde_json::json!({ "name": name, "public": public }).to_string();
        let raw: RawPlaylist =
            decode(&self.call(Method::Post, "/me/playlists", Body::Json(body))?)?;
        Ok(raw.into())
    }

    /// One page of a playlist's items from `offset` (only for playlists the
    /// user owns or collaborates on, for development-mode apps).
    pub fn playlist_tracks(
        &mut self,
        playlist_id: &str,
        offset: u32,
    ) -> Result<Page<Track>, Error> {
        let page: RawPage<RawPlaylistItem> = self.get(&format!(
            "/playlists/{}/items?limit={PAGE}&offset={offset}&additional_types=track,episode",
            form::encode(playlist_id)
        ))?;
        Ok(page.map(RawPlaylistItem::into_track))
    }

    /// One page of a playlist's item URIs from `offset` and nothing else
    /// (`fields`), to tell whether a song is in it already.
    pub fn playlist_uris(&mut self, playlist_id: &str, offset: u32) -> Result<Uris, Error> {
        let page: RawPage<RawUriItem> = self.get(&format!(
            "/playlists/{}/items?limit={PAGE}&offset={offset}&additional_types=track,episode&fields={}",
            form::encode(playlist_id),
            form::encode(URI_FIELDS),
        ))?;
        Ok(page.into_uris(offset))
    }

    /// Appends `uris` to the playlist; returns the new snapshot id.
    pub fn add_to_playlist(&mut self, playlist_id: &str, uris: &[String]) -> Result<String, Error> {
        let path = format!("/playlists/{}/items", form::encode(playlist_id));
        let mut snapshot = String::new();
        for chunk in uris.chunks(PLAYLIST_CHUNK) {
            let body = serde_json::json!({ "uris": chunk }).to_string();
            let reply: RawSnapshot = decode(&self.call(Method::Post, &path, Body::Json(body))?)?;
            snapshot = reply.snapshot_id;
        }
        Ok(snapshot)
    }

    /// Whether each of `uris` is in the user's library (Liked Songs for
    /// tracks), in order.
    pub fn library_contains(&mut self, uris: &[String]) -> Result<Vec<bool>, Error> {
        let mut out = Vec::with_capacity(uris.len());
        for chunk in uris.chunks(LIBRARY_CHUNK) {
            let found: Vec<bool> = self.get(&format!(
                "/me/library/contains?uris={}",
                form::encode(&chunk.join(","))
            ))?;
            out.extend(found);
        }
        Ok(out)
    }

    /// Saves `uris` to the library (likes tracks).
    pub fn library_save(&mut self, uris: &[String]) -> Result<(), Error> {
        self.library_write(Method::Put, uris)
    }

    /// Removes `uris` from the library (unlikes tracks).
    pub fn library_remove(&mut self, uris: &[String]) -> Result<(), Error> {
        self.library_write(Method::Delete, uris)
    }

    fn library_write(&mut self, method: Method, uris: &[String]) -> Result<(), Error> {
        for chunk in uris.chunks(LIBRARY_CHUNK) {
            let path = format!("/me/library?uris={}", form::encode(&chunk.join(",")));
            self.call(method, &path, Body::Empty)?;
        }
        Ok(())
    }

    /// Track search; `limit` is clamped to 1..=10 (the development-mode cap).
    pub fn search_tracks(
        &mut self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Page<Track>, Error> {
        let limit = limit.clamp(1, SEARCH_MAX);
        let reply: RawSearch = self.get(&format!(
            "/search?q={}&type=track&limit={limit}&offset={offset}",
            form::encode(query)
        ))?;
        Ok(match reply.tracks {
            Some(page) => page.map(|t| t.into_track()),
            None => Page {
                items: Vec::new(),
                offset,
                next_offset: offset,
                total: 0,
                has_more: false,
            },
        })
    }

    /// Tracks by an artist, for "more like this". `GET /artists/{id}/
    /// top-tracks` is gone for development-mode apps (Feb 2026), so this is
    /// a field-filtered search.
    pub fn artist_tracks(&mut self, artist: &str) -> Result<Vec<Track>, Error> {
        let query = format!("artist:\"{}\"", artist.replace('"', ""));
        Ok(self.search_tracks(&query, SEARCH_MAX, 0)?.items)
    }

    // ---- the player (Premium only; needs the playback scopes) ------------

    /// The active device's state (`GET /me/player`); `None` when nothing
    /// is playing anywhere (204).
    pub fn player(&mut self) -> Result<Option<PlayerState>, Error> {
        let body = self.call(Method::Get, "/me/player", Body::Empty)?;
        if body.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(decode::<RawPlayer>(&body)?.into()))
    }

    /// `PUT /me/player/shuffle`.
    pub fn set_shuffle(&mut self, on: bool) -> Result<(), Error> {
        let path = format!("/me/player/shuffle?state={on}");
        self.call(Method::Put, &path, Body::Empty).map(drop)
    }

    /// `PUT /me/player/repeat`.
    pub fn set_repeat(&mut self, repeat: Repeat) -> Result<(), Error> {
        let path = format!("/me/player/repeat?state={}", repeat.as_str());
        self.call(Method::Put, &path, Body::Empty).map(drop)
    }

    /// Plays `context_uri` (a playlist, album, …) on the active device,
    /// from `offset_uri` (a track in it) when given (`PUT /me/player/play`).
    pub fn play(&mut self, context_uri: &str, offset_uri: Option<&str>) -> Result<(), Error> {
        let mut body = serde_json::json!({ "context_uri": context_uri });
        if let Some(uri) = offset_uri {
            body["offset"] = serde_json::json!({ "uri": uri });
        }
        let body = Body::Json(body.to_string());
        self.call(Method::Put, "/me/player/play", body).map(drop)
    }
}

fn decode<T: DeserializeOwned>(body: &str) -> Result<T, Error> {
    serde_json::from_str(body).map_err(|e| Error::Decode(e.to_string()))
}

/// `Retry-After` in seconds; 1 s when missing or not a number (an
/// HTTP-date, which Spotify doesn't send).
fn retry_after(resp: &Response) -> Duration {
    let secs = resp
        .retry_after
        .as_deref()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(1);
    Duration::from_secs(secs)
}

fn rate_limited(resp: &Response) -> Error {
    Error::RateLimited {
        retry_after: retry_after(resp),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
