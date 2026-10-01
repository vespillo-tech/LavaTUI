//! Spotify Web API client, for the library features the desktop app can't
//! do: playlists, add to playlist, like/unlike, search.
//!
//! Self-contained and UI-free. [`SpotifyWeb`] is a handle to a worker
//! thread that does all the (blocking) HTTP; the UI sends [`Request`]s and
//! drains [`Event`]s with [`SpotifyWeb::poll`] once per frame, so nothing
//! here ever blocks a frame.
//!
//! Login is Authorization Code + PKCE with the user's own Client ID (no
//! secret): [`SpotifyWeb::login`] opens the browser on Spotify's consent
//! page, a one-shot server on [`REDIRECT_URI`] catches the redirect, and
//! the worker trades the code for tokens, kept in the OS keyring (else a
//! 0600 file). Setup and Spotify's current rules: `docs/spotify.md`.
//!
//! ```ignore
//! let mut spotify = SpotifyWeb::new(client_id);
//! if !spotify.is_logged_in() { let url = spotify.login()?; /* show url */ }
//! let id = spotify.request(Request::MyPlaylists);
//! // each frame:
//! while let Some(event) = spotify.poll() { /* match event */ }
//! ```

mod callback;
mod client;
mod error;
mod form;
mod http;
mod pkce;
mod store;
mod types;

#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

pub use error::Error;
pub use types::{Page, Playlist, Track, User};

use client::Client;
use http::Http;
use pkce::Pkce;

/// The redirect URI to register in the Spotify dashboard, exactly.
pub const REDIRECT_URI: &str = "http://127.0.0.1:8731/callback";
/// Where a Client ID can come from when the config doesn't set one.
pub const CLIENT_ID_ENV: &str = "LAVATUI_SPOTIFY_CLIENT_ID";
/// What we ask the user to grant.
pub const SCOPES: &[&str] = &[
    "playlist-read-private",
    "playlist-read-collaborative",
    "playlist-modify-public",
    "playlist-modify-private",
    "user-library-read",
    "user-library-modify",
];

const ACCOUNTS_BASE: &str = "https://accounts.spotify.com";
const API_BASE: &str = "https://api.spotify.com/v1";

/// The Client ID from [`CLIENT_ID_ENV`], if set and non-blank.
pub fn client_id_from_env() -> Option<String> {
    let id = std::env::var(CLIENT_ID_ENV).ok()?;
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

/// Ties a [`Event::Reply`] to its [`Request`].
pub type RequestId = u64;

/// A Web API call. URIs are `spotify:track:…` (episodes work too where the
/// API allows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// → [`Reply::User`].
    Me,
    /// All the user's playlists, every page → [`Reply::Playlists`].
    MyPlaylists,
    /// One page (50) of a playlist from `offset` → [`Reply::Tracks`]. Only
    /// playlists the user owns or collaborates on (else `Forbidden`).
    PlaylistTracks { playlist_id: String, offset: u32 },
    /// A new playlist owned by the user → [`Reply::Playlist`].
    CreatePlaylist { name: String, public: bool },
    /// Append to a playlist → [`Reply::Snapshot`].
    AddToPlaylist {
        playlist_id: String,
        uris: Vec<String>,
    },
    /// Liked or not, per URI, in order → [`Reply::Contains`].
    LibraryContains { uris: Vec<String> },
    /// Like (save to library) → [`Reply::Done`].
    Like { uris: Vec<String> },
    /// Unlike (remove from library) → [`Reply::Done`].
    Unlike { uris: Vec<String> },
    /// Track search, `limit` clamped to 1..=10 → [`Reply::Tracks`].
    SearchTracks {
        query: String,
        limit: u32,
        offset: u32,
    },
    /// Up to 10 tracks by the artist, for "more like this" →
    /// [`Reply::TrackList`].
    ArtistTracks { artist: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    User(User),
    Playlists(Vec<Playlist>),
    Playlist(Playlist),
    Tracks(Page<Track>),
    TrackList(Vec<Track>),
    Contains(Vec<bool>),
    /// The playlist's new snapshot id.
    Snapshot(String),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Login finished. `saved: false`: the tokens couldn't be stored, so
    /// this login lasts for this session only.
    LoggedIn {
        saved: bool,
    },
    LoginFailed(Error),
    /// After [`SpotifyWeb::logout`] (`expired: false`), or when Spotify
    /// refused the refresh token (`expired: true`; log in again).
    LoggedOut {
        expired: bool,
    },
    Reply {
        id: RequestId,
        result: Result<Reply, Error>,
    },
}

enum Job {
    Request(RequestId, Request),
    Code { code: String, verifier: String },
    LoginFailed(Error),
    Logout,
}

/// The handle the UI owns. Dropping it cancels a pending login and lets the
/// worker finish its current request and exit.
pub struct SpotifyWeb {
    client_id: String,
    jobs: Sender<Job>,
    events: Receiver<Event>,
    logged_in: Arc<AtomicBool>,
    next_id: RequestId,
    login_cancel: Option<Arc<AtomicBool>>,
}

impl SpotifyWeb {
    /// Starts the worker for `client_id`. A saved login is picked up on the
    /// worker (a keyring read can be slow or prompt), so
    /// [`is_logged_in`](Self::is_logged_in) may turn true a moment later.
    pub fn new(client_id: impl Into<String>) -> Self {
        let client_id = client_id.into();
        let id = client_id.clone();
        Self::spawn(client_id, move || {
            Client::new(http::Ureq::new(), id, Box::new(store::SystemStore::new()))
        })
    }

    fn spawn<H: Http + 'static>(
        client_id: String,
        make: impl FnOnce() -> Client<H> + Send + 'static,
    ) -> Self {
        let (jobs, job_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let logged_in = Arc::new(AtomicBool::new(false));
        let flag = logged_in.clone();
        std::thread::Builder::new()
            .name("spotify-web".into())
            .spawn(move || {
                crate::thread_qos::worker();
                work(make(), &job_rx, &event_tx, &flag)
            })
            .expect("spawn spotify-web worker");
        Self {
            client_id,
            jobs,
            events,
            logged_in,
            next_id: 0,
            login_cancel: None,
        }
    }

    /// The client for the configured Client ID
    /// ([`Settings::spotify_client_id`](crate::config::Settings::spotify_client_id)),
    /// or `None` when there isn't one (the library features stay off).
    pub fn from_settings(settings: &crate::config::Settings) -> Option<Self> {
        settings.spotify_client_id().map(Self::new)
    }

    pub fn is_logged_in(&self) -> bool {
        self.logged_in.load(Ordering::Relaxed)
    }

    /// Starts a login: opens the browser on Spotify's consent page and
    /// returns its URL (show it, in case no browser opened). The outcome
    /// arrives as [`Event::LoggedIn`] or [`Event::LoginFailed`]; the user
    /// has five minutes. A login already in progress is cancelled.
    pub fn login(&mut self) -> Result<String, Error> {
        self.cancel_login();
        let pkce = Pkce::new().map_err(|e| Error::Login(format!("no randomness: {e}")))?;
        let url = pkce.authorize_url(&self.client_id);
        let cancel = Arc::new(AtomicBool::new(false));
        self.login_cancel = Some(cancel.clone());
        let jobs = self.jobs.clone();
        let browser_url = url.clone();
        std::thread::Builder::new()
            .name("spotify-login".into())
            .spawn(move || {
                crate::thread_qos::worker();
                let job = match wait_for_code(&browser_url, &pkce, &cancel) {
                    Ok(code) => Job::Code {
                        code,
                        verifier: pkce.verifier,
                    },
                    Err(err) => Job::LoginFailed(err),
                };
                // A cancelled attempt reports nothing: a newer one owns the UI.
                if !cancel.load(Ordering::Relaxed) {
                    let _ = jobs.send(job);
                }
            })
            .map_err(|e| Error::Login(e.to_string()))?;
        Ok(url)
    }

    /// Stops waiting for the browser. Silent: no event follows.
    pub fn cancel_login(&mut self) {
        if let Some(cancel) = self.login_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Forgets the login (here and in the keyring/file); then
    /// [`Event::LoggedOut`].
    pub fn logout(&mut self) {
        self.cancel_login();
        self.logged_in.store(false, Ordering::Relaxed);
        let _ = self.jobs.send(Job::Logout);
    }

    /// Queues `request`; its [`Event::Reply`] carries the returned id.
    /// Requests run one at a time, in order.
    pub fn request(&mut self, request: Request) -> RequestId {
        self.next_id += 1;
        let _ = self.jobs.send(Job::Request(self.next_id, request));
        self.next_id
    }

    /// The next event, if one is ready. Never blocks.
    pub fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }

    #[cfg(test)]
    fn poll_timeout(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for SpotifyWeb {
    fn drop(&mut self) {
        self.cancel_login();
    }
}

/// The login thread: listen (retrying briefly while a just-cancelled
/// attempt lets go of the port), open the browser, wait for the redirect.
fn wait_for_code(url: &str, pkce: &Pkce, cancel: &AtomicBool) -> Result<String, Error> {
    let mut listener = callback::bind();
    for _ in 0..20 {
        if listener.is_ok() || cancel.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        listener = callback::bind();
    }
    let listener = listener?;
    // A failed open is fine: the caller shows the URL.
    let _ = webbrowser::open(url);
    callback::wait(&listener, &pkce.state, cancel, callback::TIMEOUT)
}

/// The worker loop: one job at a time until the handle (and any login
/// thread) is gone.
fn work<H: Http>(
    mut client: Client<H>,
    jobs: &Receiver<Job>,
    events: &Sender<Event>,
    logged_in: &AtomicBool,
) {
    logged_in.store(client.is_logged_in(), Ordering::Relaxed);
    while let Ok(job) = jobs.recv() {
        let mut out = Vec::with_capacity(2);
        match job {
            Job::Request(id, request) => {
                let was_logged_in = client.is_logged_in();
                let result = handle(&mut client, request);
                let expired = was_logged_in && !client.is_logged_in();
                out.push(Event::Reply { id, result });
                if expired {
                    out.push(Event::LoggedOut { expired: true });
                }
            }
            Job::Code { code, verifier } => {
                out.push(match client.exchange_code(&code, &verifier) {
                    Ok(saved) => Event::LoggedIn { saved },
                    Err(err) => Event::LoginFailed(err),
                })
            }
            Job::LoginFailed(err) => out.push(Event::LoginFailed(err)),
            Job::Logout => {
                client.logout();
                out.push(Event::LoggedOut { expired: false });
            }
        }
        logged_in.store(client.is_logged_in(), Ordering::Relaxed);
        for event in out {
            if events.send(event).is_err() {
                return;
            }
        }
    }
}

fn handle<H: Http>(client: &mut Client<H>, request: Request) -> Result<Reply, Error> {
    Ok(match request {
        Request::Me => Reply::User(client.me()?),
        Request::MyPlaylists => Reply::Playlists(client.my_playlists()?),
        Request::PlaylistTracks {
            playlist_id,
            offset,
        } => Reply::Tracks(client.playlist_tracks(&playlist_id, offset)?),
        Request::CreatePlaylist { name, public } => {
            Reply::Playlist(client.create_playlist(&name, public)?)
        }
        Request::AddToPlaylist { playlist_id, uris } => {
            Reply::Snapshot(client.add_to_playlist(&playlist_id, &uris)?)
        }
        Request::LibraryContains { uris } => Reply::Contains(client.library_contains(&uris)?),
        Request::Like { uris } => {
            client.library_save(&uris)?;
            Reply::Done
        }
        Request::Unlike { uris } => {
            client.library_remove(&uris)?;
            Reply::Done
        }
        Request::SearchTracks {
            query,
            limit,
            offset,
        } => Reply::Tracks(client.search_tracks(&query, limit, offset)?),
        Request::ArtistTracks { artist } => Reply::TrackList(client.artist_tracks(&artist)?),
    })
}
