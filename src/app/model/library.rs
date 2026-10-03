//! The Spotify library (Web API): login, the playlist browser and the
//! add-to-playlist picker, like / unlike, and shuffle / repeat through the
//! player endpoints where the account allows them.
//!
//! All network work happens on the [`SpotifyWeb`] worker: the model only
//! queues [`Request`]s and drains [`Event`]s once a frame ([`Library::sync`],
//! bounded), so nothing here blocks a frame. The client exists only while
//! the music widget is placed (or the Spotify setup is open) and a Client
//! ID is configured (`spotify.client_id` or `LAVATUI_SPOTIFY_CLIENT_ID`);
//! tests and `--demo` plug in a `FakeWeb` (the demo's needs no Client ID).
//!
//! The saved login is read only when a library feature is first used (or
//! the Spotify setup opens): on macOS that read can make the Keychain ask
//! for permission, so it never happens just for starting the app
//! (lava-1xk.38). Until then `spotify.logged_in` (not a secret) says
//! whether there is one; the key that needed it runs again once it's read.
//!
//! Adding a song to a playlist that has it already asks first
//! (lava-75z.24). Spotify can't say which playlists hold a song, so the
//! add picker reads its playlists' songs (URIs only, a page at a time, one
//! request in flight, a budget per opening, the chosen playlist first) and
//! keeps them per playlist while its snapshot id stays the same. A check
//! that fails or stalls never loses the add: it goes ahead, and says so.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::{Model, Overlay};
use crate::media::Capabilities;
use crate::spotify_web::{
    Error, Event, LoginStore, PlayerState, Playlist, Repeat, Reply, Request, RequestId, Track,
    Uris, User, Web,
};
use crate::ui::keymap::{Action, PlayerKey};
use crate::ui::picker::{self, Hit, Placement};

/// How often the player state is read again while nothing changes.
const PLAYER_EVERY: Duration = Duration::from_secs(30);
/// A read that failed (no network, a server error, a rate limit) is
/// asked again after this, or Spotify's `Retry-After` if longer: not
/// every frame, which kept a rate limit going and the heart and `+` away
/// (lava-75z.22).
pub(super) const RETRY: Duration = Duration::from_secs(5);
/// The second `i` within this logs out.
const LOGOUT_WINDOW: Duration = Duration::from_secs(2);
/// Events handled per frame at most (the rest wait for the next one).
const EVENTS_PER_FRAME: usize = 32;
/// The next page of a playlist is asked for when the cursor comes this
/// close to the end of what has loaded.
const PREFETCH: usize = 10;
/// Two clicks on one row this close together choose it.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// A key waiting for the saved login is dropped after this (the user may
/// still be looking at macOS's question, or have said no).
pub(super) const PENDING_FOR: Duration = Duration::from_secs(30);
/// Pages of playlist songs the add picker reads ahead per opening, so the
/// check is usually done before `⏎` (50 songs a page).
const READ_AHEAD_PAGES: u32 = 40;
/// A check that hasn't moved on for this long is given up: the song is
/// added anyway.
pub(super) const CHECK_STALL: Duration = Duration::from_secs(5);
/// Said just before the first Keychain read of a session.
pub const KEYCHAIN_HEADS_UP: &str =
    "macOS may ask to let LavaTUI use your saved Spotify login · choose Always Allow";

type Connect = Box<dyn Fn() -> Option<Box<dyn Web>>>;

/// Connects to the Web API with `client_id` (none: never), keeping the
/// login in `store`.
#[cfg(not(test))]
fn connector(client_id: Option<String>, store: LoginStore) -> Connect {
    Box::new(move || {
        client_id
            .clone()
            .map(|id| Box::new(crate::spotify_web::SpotifyWeb::new(id, store)) as Box<dyn Web>)
    })
}

/// Tests never reach the real thing: they plug in a fake.
#[cfg(test)]
fn connector(_: Option<String>, _: LoginStore) -> Connect {
    Box::new(|| None)
}

/// A list that loads from the Web API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing<T> {
    pub items: Vec<T>,
    /// Answered at least once.
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
}

impl<T> Default for Listing<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            loaded: false,
            loading: false,
            error: None,
        }
    }
}

/// The playlist open in the browser and as much of it as has loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPlaylist {
    pub playlist: Playlist,
    pub tracks: Listing<Track>,
    /// Where the next page starts: songs removed from the playlist leave
    /// empty slots, so it can be past `tracks.items.len()`.
    pub next_offset: u32,
    pub has_more: bool,
    /// Which opening this is: a page asked for by an earlier one is
    /// left alone.
    opening: u64,
    /// The last page failed: not asked again before this.
    retry: Option<Instant>,
}

/// The Web API's view of the player: shuffle / repeat live here when the
/// desktop app's own controls can't change them (lava-75z.12).
#[derive(Debug, Default)]
pub struct WebPlayer {
    /// `Some(false)`: Spotify refused (not Premium, or a login without the
    /// playback scopes); shuffle / repeat stay hidden until the next login.
    pub allowed: Option<bool>,
    pub state: Option<PlayerState>,
    /// The refusal was about scopes: logging in again fixes it.
    pub needs_login: bool,
    asked_at: Option<Instant>,
    /// The desktop player's track (its own id) when the state was last
    /// asked for.
    asked_for: Option<String>,
    /// The desktop player's track when the request `state` answers was
    /// sent: a state asked for before a track change says nothing about
    /// the new one.
    state_for: Option<String>,
    in_flight: bool,
}

/// Where the login stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Account {
    /// No Client ID (or music is off): no library features.
    Unavailable,
    LoggedOut,
    /// Waiting for the browser.
    LoggingIn,
    LoggedIn,
}

/// What a request was for, so its reply lands in the right place.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Want {
    Me,
    Playlists,
    /// A page of the open playlist, from `offset`, for its `opening`.
    Tracks {
        playlist_id: String,
        opening: u64,
        offset: u32,
    },
    Liked(String),
    Like {
        uri: String,
        on: bool,
    },
    Add {
        playlist_id: String,
        uri: String,
        name: String,
        /// Added without knowing whether it was there already.
        unchecked: bool,
    },
    /// A page of a playlist's songs, read from `offset` at `snapshot`.
    Uris {
        playlist_id: String,
        snapshot: String,
        offset: u32,
    },
    Player,
    Mode,
    /// Playing in a context; the desktop app plays it instead if Spotify
    /// refuses (no Premium).
    Play(Playing),
}

pub struct Library {
    web: Option<Box<dyn Web>>,
    connect: Connect,
    /// `connect` was plugged in (tests, `--demo`): a new Client ID or
    /// login store keeps it.
    plugged: bool,
    /// `--demo`'s made-up account: set up without a Client ID, and its
    /// login never saved to the config.
    pub demo: bool,
    client_id: Option<String>,
    store: LoginStore,
    /// A login is saved, as far as we know (`spotify.logged_in`): shown as
    /// logged in until the saved login has been read.
    pub saved: bool,
    /// The saved login has been read since connecting (or replaced).
    unlocked: bool,
    /// A library key waiting for the saved login to be read.
    pub(super) pending: Option<(PlayerKey, Instant)>,
    wants: Vec<(RequestId, Want)>,
    /// The consent page, while a login waits for the browser.
    pub login_url: Option<String>,
    /// Why the last login didn't work (until the next one starts).
    pub login_error: Option<String>,
    pub me: Option<User>,
    me_asked: bool,
    /// The account lookup failed: asked again then.
    me_retry: Option<Instant>,
    /// Spotify refused the logged-in account (not on the app's allowlist,
    /// or the app's owner has no Premium): its message. Not asked again
    /// until the next login.
    pub refused: Option<String>,
    pub playlists: Listing<Playlist>,
    pub open: Option<OpenPlaylist>,
    /// The playing track's URI and whether it's liked.
    pub liked: Option<(String, bool)>,
    liked_asked: Option<String>,
    /// When to ask about `liked_asked` again, after its lookup failed.
    liked_retry: Option<Instant>,
    pub player: WebPlayer,
    logout_armed: Option<Instant>,
    /// A refused context play: the desktop app should play it instead.
    fallback: Option<Playing>,
    /// What the open list is filtered by (lava-75z.17).
    pub find: Find,
    /// What's known of the songs in the playlists, by playlist id.
    contents: HashMap<String, Contents>,
    /// A page of songs is being read (one at a time).
    reading: bool,
    /// Spotify asked us to slow down: no reading ahead until then.
    read_after: Option<Instant>,
    /// Pages the add picker may still read ahead.
    read_budget: u32,
    /// Playlists opened so far (each opening's number).
    openings: u64,
    /// An add waiting for the check or for the user.
    pub adding: Option<Adding>,
}

/// The songs of one playlist read so far, as of its `snapshot` id.
#[derive(Debug, Default)]
struct Contents {
    snapshot: String,
    uris: HashSet<String>,
    /// Where the next page starts; `None` once every page is read.
    next: Option<u32>,
    /// Spotify wouldn't say: not asked again until the snapshot changes,
    /// or after `retry` when that may help (no network, a server error).
    failed: bool,
    retry: Option<Instant>,
}

impl Contents {
    fn new(snapshot: &str) -> Self {
        Self {
            snapshot: snapshot.to_owned(),
            next: Some(0),
            ..Self::default()
        }
    }
}

/// `⏎` in the add picker, before the song is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adding {
    pub playlist_id: String,
    pub name: String,
    pub uri: String,
    pub stage: Stage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Reading the playlist's songs; the last page came `since`. Shows
    /// how far it got when the playlist is long.
    Checking {
        since: Instant,
        read: u32,
        total: u32,
    },
    /// It's there already: add it again?
    Confirm,
}

/// Type-to-filter in the library overlay: `/`, then what's typed keeps
/// the rows whose name (and, for tracks, artists) has every word in it.
#[derive(Debug, Default)]
pub struct Find {
    /// What's typed.
    pub text: String,
    /// The playlists' filter, kept while one playlist's tracks are open.
    back: String,
    /// The items matching `text` (indices into the unfiltered list) and
    /// which list they're for; `None` while nothing is typed.
    hits: Option<(ListKind, Vec<usize>)>,
    /// The lists changed since `hits` was worked out.
    stale: bool,
}

/// What to play: a playlist, from its top or from one of its tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Playing {
    context: String,
    track: Option<String>,
    name: String,
}

impl Library {
    /// The library for `client_id` (none: off, unless a `FakeWeb` is
    /// plugged in with [`Self::connect_with`]), its login kept in
    /// `store`; `saved`: one is saved. Tests never reach the real thing (a
    /// keyring read could prompt): they plug in a fake.
    pub fn new(client_id: Option<String>, store: LoginStore, saved: bool) -> Self {
        Self {
            web: None,
            connect: connector(client_id.clone(), store),
            plugged: false,
            demo: false,
            client_id,
            store,
            saved,
            unlocked: false,
            pending: None,
            wants: Vec::new(),
            login_url: None,
            login_error: None,
            me: None,
            me_asked: false,
            me_retry: None,
            refused: None,
            playlists: Listing::default(),
            open: None,
            liked: None,
            liked_asked: None,
            liked_retry: None,
            player: WebPlayer::default(),
            logout_armed: None,
            fallback: None,
            find: Find::default(),
            contents: HashMap::new(),
            reading: false,
            read_after: None,
            read_budget: 0,
            openings: 0,
            adding: None,
        }
    }

    /// Use `connect` for the client, from the next time music is placed
    /// (tests and `--demo`: a `FakeWeb`).
    pub fn connect_with(&mut self, connect: impl Fn() -> Option<Box<dyn Web>> + 'static) {
        self.connect = Box::new(connect);
        self.plugged = true;
        self.disconnect();
    }

    /// A new Client ID (the settings screen): the old client goes, and the
    /// next sync connects with this one. A plugged-in fake stays.
    pub fn set_client_id(&mut self, client_id: Option<String>) {
        if !self.plugged {
            self.connect = connector(client_id.clone(), self.store);
        }
        self.client_id = client_id;
        self.disconnect();
    }

    /// Keep the login in `store` from now on, moving a saved one there
    /// (`Moved` says when it's done).
    pub fn set_store(&mut self, store: LoginStore) {
        if store == self.store {
            return;
        }
        self.store = store;
        if !self.plugged {
            self.connect = connector(self.client_id.clone(), store);
        }
        // Nothing connected (no Client ID): nothing to move.
        if let Some(web) = &mut self.web {
            web.set_store(store);
        }
    }

    /// The saved login hasn't been read yet, and reading it may make the
    /// system ask the user (the macOS Keychain).
    pub fn locked(&self) -> bool {
        self.web.as_ref().is_some_and(|w| w.locked())
    }

    /// Read the saved login now (`Unlocked` says when it's done). Whether
    /// it was still locked, i.e. the system may now ask.
    pub fn unlock(&mut self) -> bool {
        match &mut self.web {
            Some(web) if web.locked() => {
                web.unlock();
                true
            }
            _ => false,
        }
    }

    /// Open the browser on Spotify's consent page (logged out only).
    pub fn start_login(&mut self) -> Option<String> {
        let web = self.web.as_mut()?;
        self.login_error = None;
        match web.login() {
            Ok(url) => {
                self.login_url = Some(url);
                None
            }
            Err(e) => {
                let e = e.to_string();
                self.login_error = Some(e.clone());
                Some(e)
            }
        }
    }

    /// Stop waiting for the browser.
    pub fn cancel_login(&mut self) {
        if let Some(web) = &mut self.web {
            web.cancel_login();
        }
        self.login_url = None;
    }

    /// Forget the login (`LoggedOut` says when it's done).
    pub fn logout(&mut self) {
        if let Some(web) = &mut self.web {
            web.logout();
        }
    }

    fn disconnect(&mut self) {
        if let Some(web) = &mut self.web {
            web.cancel_login();
        }
        self.web = None;
        self.forget();
        self.login_url = None;
        self.unlocked = false;
        self.pending = None;
    }

    /// Drop everything that belonged to a login.
    fn forget(&mut self) {
        self.wants.clear();
        self.me = None;
        self.me_asked = false;
        self.me_retry = None;
        self.refused = None;
        self.playlists = Listing::default();
        self.open = None;
        self.liked = None;
        self.liked_asked = None;
        self.liked_retry = None;
        self.player = WebPlayer::default();
        self.logout_armed = None;
        self.contents.clear();
        self.reading = false;
        self.read_after = None;
        self.adding = None;
    }

    /// Where the login stands, as shown: a saved login not read yet counts
    /// as logged in.
    pub fn account(&self) -> Account {
        match &self.web {
            None => Account::Unavailable,
            Some(_) if self.login_url.is_some() => Account::LoggingIn,
            Some(web) if web.is_logged_in() => Account::LoggedIn,
            Some(_) if self.saved && !self.unlocked => Account::LoggedIn,
            Some(_) => Account::LoggedOut,
        }
    }

    /// Logged in, with the login read: requests can go.
    pub fn logged_in(&self) -> bool {
        self.login_url.is_none() && self.web.as_ref().is_some_and(|w| w.is_logged_in())
    }

    /// Requests in flight or a login pending: worth looking again soon.
    pub fn busy(&self) -> bool {
        let checking = self
            .adding
            .as_ref()
            .is_some_and(|a| matches!(a.stage, Stage::Checking { .. }));
        !self.wants.is_empty() || self.login_url.is_some() || checking
    }

    fn request(&mut self, request: Request, want: Want) {
        if let Some(web) = &mut self.web {
            let id = web.request(request);
            self.wants.push((id, want));
        }
    }

    /// Shuffle / repeat go through the Web API: logged in, Spotify said
    /// yes, and a device is playing.
    pub fn modes(&self) -> Option<&PlayerState> {
        (self.logged_in() && self.player.allowed == Some(true))
            .then_some(self.player.state.as_ref())
            .flatten()
    }

    /// Whether `uri` (the playing track) is liked, once known.
    pub fn liked(&self, uri: &str) -> Option<bool> {
        if !self.logged_in() {
            return None;
        }
        self.liked
            .as_ref()
            .filter(|(u, _)| u == uri)
            .map(|&(_, on)| on)
    }

    /// Whether `playlist` has `uri`, once known: `None` while its songs
    /// are still being read, or couldn't be.
    pub fn has(&self, playlist: &Playlist, uri: &str) -> Option<bool> {
        let c = self
            .contents
            .get(&playlist.id)
            .filter(|c| c.snapshot == playlist.snapshot_id)?;
        if c.uris.contains(uri) {
            Some(true)
        } else {
            (c.next.is_none() && !c.failed).then_some(false)
        }
    }

    /// Read the next page of the first of `order` (playlist indices,
    /// most wanted first) not yet read through. `chosen`, the playlist an
    /// add waits on, goes first and needs no budget. One page at a time,
    /// none while the playlists are being read again (their snapshot ids
    /// may change) or Spotify asked us to wait.
    fn read_ahead(&mut self, chosen: Option<&str>, order: &[usize], now: Instant) {
        if self.reading || self.playlists.loading || self.read_after.is_some_and(|at| now < at) {
            return;
        }
        let chosen = chosen.and_then(|id| self.playlists.items.iter().position(|p| p.id == id));
        let budget = self.read_budget > 0;
        let next = chosen
            .into_iter()
            .chain(order.iter().copied().filter(|_| budget));
        for i in next {
            let Some(p) = self.playlists.items.get(i) else {
                continue;
            };
            let c = self
                .contents
                .entry(p.id.clone())
                .or_insert_with(|| Contents::new(&p.snapshot_id));
            if c.snapshot != p.snapshot_id {
                *c = Contents::new(&p.snapshot_id);
            }
            if c.failed && c.retry.is_some_and(|at| now >= at) {
                c.failed = false;
                c.retry = None;
            }
            let Some(offset) = c.next.filter(|_| !c.failed) else {
                continue;
            };
            if Some(i) != chosen {
                self.read_budget -= 1;
            }
            let (playlist_id, snapshot) = (p.id.clone(), p.snapshot_id.clone());
            let request = Request::PlaylistUris {
                playlist_id: playlist_id.clone(),
                offset,
            };
            self.reading = true;
            let want = Want::Uris {
                playlist_id,
                snapshot,
                offset,
            };
            self.request(request, want);
            return;
        }
    }

    /// A page of songs (or why not).
    fn read(
        &mut self,
        playlist_id: &str,
        snapshot: &str,
        offset: u32,
        result: Result<Reply, Error>,
        now: Instant,
    ) {
        self.reading = false;
        let Some(c) = self
            .contents
            .get_mut(playlist_id)
            .filter(|c| c.snapshot == snapshot && c.next == Some(offset))
        else {
            return;
        };
        let read = match result {
            Ok(Reply::Uris(Uris { uris, next, total })) => {
                c.uris.extend(uris);
                // A page that doesn't move on would never end.
                c.next = next.filter(|&n| n > offset);
                Some((c.next.unwrap_or(total), total))
            }
            // Asked again once the wait is over (an add waiting on it
            // goes ahead when the check stalls).
            Err(Error::RateLimited { retry_after }) => {
                self.read_after = Some(now + retry_after);
                None
            }
            Err(e) => {
                c.failed = true;
                c.retry = e.retry_after(RETRY).map(|wait| now + wait);
                None
            }
            Ok(_) => {
                c.failed = true;
                None
            }
        };
        let next = c.next.filter(|_| !c.failed);
        let chosen = match &mut self.adding {
            Some(adding) if adding.playlist_id == playlist_id => {
                if let (Some((read, total)), Stage::Checking { .. }) = (read, adding.stage) {
                    adding.stage = Stage::Checking {
                        since: now,
                        read,
                        total,
                    };
                }
                true
            }
            _ => false,
        };
        // On to the next page at once, while it's still wanted.
        if let Some(offset) = next
            && (chosen || self.read_budget > 0)
            && self.read_after.is_none_or(|at| now >= at)
        {
            if !chosen {
                self.read_budget -= 1;
            }
            self.reading = true;
            let request = Request::PlaylistUris {
                playlist_id: playlist_id.to_owned(),
                offset,
            };
            let want = Want::Uris {
                playlist_id: playlist_id.to_owned(),
                snapshot: snapshot.to_owned(),
                offset,
            };
            self.request(request, want);
        }
    }

    /// The song is in the playlist now: the playlist's new `snapshot` id
    /// keeps what's known of it.
    fn added(&mut self, playlist_id: &str, uri: String, snapshot: String) {
        let Some(p) = self
            .playlists
            .items
            .iter_mut()
            .find(|p| p.id == playlist_id)
        else {
            return;
        };
        let old = std::mem::replace(&mut p.snapshot_id, snapshot.clone());
        p.total += 1;
        if let Some(c) = self.contents.get_mut(playlist_id) {
            // A check that failed is asked again under the new snapshot.
            if c.snapshot == old && !c.failed {
                c.snapshot = snapshot;
                c.uris.insert(uri);
            } else {
                self.contents.remove(playlist_id);
            }
        }
    }

    /// Add `uri` to the playlist now.
    fn add(&mut self, playlist_id: String, name: String, uri: String, unchecked: bool) {
        let request = Request::AddToPlaylist {
            playlist_id: playlist_id.clone(),
            uris: vec![uri.clone()],
        };
        let want = Want::Add {
            playlist_id,
            uri,
            name,
            unchecked,
        };
        self.request(request, want);
    }

    /// The playlists the user can add to (owned or collaborative), by
    /// index into [`Self::playlists`]; empty until who they are is known.
    pub fn editable(&self) -> Vec<usize> {
        let Some(me) = &self.me else {
            return Vec::new();
        };
        (0..self.playlists.items.len())
            .filter(|&i| self.playlists.items[i].editable_by(me))
            .collect()
    }

    /// Once a frame: connect while `on`, drain the worker's events, and ask
    /// for what the widget needs (who I am, whether `uri`, the playing
    /// Spotify track, is liked, the player's state again when `track`, the
    /// desktop player's own id for it, changes). Returns toasts to show
    /// (the last wins).
    pub fn sync(
        &mut self,
        on: bool,
        now: Instant,
        track: Option<&str>,
        uri: Option<&str>,
    ) -> Vec<String> {
        if !on {
            if self.web.is_some() {
                self.disconnect();
            }
            return Vec::new();
        }
        if self.web.is_none() {
            self.web = (self.connect)();
        }
        let mut toasts = Vec::new();
        for _ in 0..EVENTS_PER_FRAME {
            let Some(event) = self.web.as_mut().and_then(|w| w.poll()) else {
                break;
            };
            if let Some(toast) = self.event(event, now) {
                toasts.push(toast);
            }
        }
        if !self.logged_in() {
            return toasts;
        }
        if !self.me_asked && self.me_retry.is_none_or(|at| now >= at) {
            self.me_asked = true;
            self.request(Request::Me, Want::Me);
        }
        let retry = self.liked_retry.is_some_and(|at| now >= at);
        if let Some(uri) = uri
            && (self.liked_asked.as_deref() != Some(uri) || retry)
        {
            self.liked_asked = Some(uri.to_owned());
            self.liked_retry = None;
            let uris = vec![uri.to_owned()];
            self.request(
                Request::LibraryContains { uris },
                Want::Liked(uri.to_owned()),
            );
        }
        let p = &self.player;
        let stale =
            p.asked_at.is_none_or(|at| now - at >= PLAYER_EVERY) || p.asked_for.as_deref() != track;
        if p.allowed != Some(false) && !p.in_flight && stale {
            self.player.in_flight = true;
            self.player.asked_at = Some(now);
            self.player.asked_for = track.map(str::to_owned);
            self.request(Request::Player, Want::Player);
        }
        toasts
    }

    fn event(&mut self, event: Event, now: Instant) -> Option<String> {
        self.find.stale = true;
        match event {
            Event::LoggedIn { saved } => {
                self.forget();
                self.login_url = None;
                self.login_error = None;
                self.saved = saved;
                self.unlocked = true;
                Some(if saved {
                    "logged in to Spotify".into()
                } else {
                    "logged in to Spotify · this session only".into()
                })
            }
            Event::LoginFailed(e) => {
                self.login_url = None;
                self.login_error = Some(e.to_string());
                Some(e.to_string())
            }
            Event::Unlocked { logged_in } => {
                self.unlocked = true;
                self.saved = logged_in;
                None
            }
            Event::Moved { logged_in, saved } => {
                self.unlocked = true;
                self.saved = logged_in && saved;
                match (logged_in, saved) {
                    (false, _) => None,
                    (true, true) => Some(format!(
                        "Spotify login now kept in {}",
                        store_name(self.store)
                    )),
                    (true, false) => {
                        Some("couldn't save the Spotify login · this session only".into())
                    }
                }
            }
            Event::LoggedOut { expired } => {
                self.forget();
                self.saved = false;
                Some(if expired {
                    "Spotify login expired · A i to log in again".into()
                } else {
                    "logged out of Spotify".into()
                })
            }
            Event::Reply { id, result } => {
                let at = self.wants.iter().position(|(w, _)| *w == id)?;
                let (_, want) = self.wants.remove(at);
                self.reply(want, result, now)
            }
        }
    }

    fn reply(&mut self, want: Want, result: Result<Reply, Error>, now: Instant) -> Option<String> {
        // A lost login says so itself (`LoggedOut`).
        if let Err(e) = &result
            && e.needs_login()
        {
            if want == Want::Player {
                self.player.in_flight = false;
            }
            if let Want::Uris { .. } = want {
                self.reading = false;
            }
            return None;
        }
        match (want, result) {
            (Want::Me, Ok(Reply::User(me))) => self.me = Some(me),
            // Every request would be refused: say so once, and how to fix
            // it, rather than asking again and again.
            (Want::Me, Err(Error::Forbidden(why))) => {
                self.refused = Some(why);
                return Some(
                    "Spotify refused this account · settings (,) › spotify says why".into(),
                );
            }
            (Want::Me, result) => {
                self.me_asked = false;
                let wait = result.err().and_then(|e| e.retry_after(RETRY));
                self.me_retry = Some(now + wait.unwrap_or(RETRY));
            }
            (Want::Playlists, Ok(Reply::Playlists(lists))) => {
                // What was read of a playlist that changed since is stale.
                self.contents.retain(|id, c| {
                    lists
                        .iter()
                        .any(|p| p.id == *id && p.snapshot_id == c.snapshot)
                });
                self.playlists = Listing {
                    items: lists,
                    loaded: true,
                    loading: false,
                    error: None,
                }
            }
            (Want::Playlists, result) => {
                self.playlists.loading = false;
                self.playlists.error = Some(message(result));
            }
            (
                Want::Tracks {
                    playlist_id,
                    opening,
                    offset,
                },
                result,
            ) => {
                // Only the page this opening is waiting for.
                let open = self.open.as_mut().filter(|o| {
                    o.playlist.id == playlist_id
                        && o.opening == opening
                        && o.next_offset == offset
                        && o.tracks.loading
                })?;
                open.tracks.loading = false;
                match result {
                    Ok(Reply::Tracks(page)) => {
                        if page.offset == offset {
                            open.tracks.items.extend(page.items);
                            open.next_offset = page.next_offset;
                        }
                        open.tracks.loaded = true;
                        open.tracks.error = None;
                        open.has_more = page.has_more;
                        open.playlist.total = page.total;
                    }
                    Err(e) => {
                        open.retry = Some(now + e.retry_after(RETRY).unwrap_or(RETRY));
                        open.tracks.error = Some(e.to_string());
                    }
                    other => {
                        open.retry = Some(now + RETRY);
                        open.tracks.error = Some(message(other));
                    }
                }
            }
            (Want::Liked(uri), Ok(Reply::Contains(found))) => {
                self.liked = Some((uri, found.first().copied().unwrap_or(false)));
            }
            (Want::Liked(uri), Err(e)) => {
                crate::diag::note(|| {
                    format!("library: is {} liked? failed: {e}", crate::diag::tag(&uri))
                });
                if self.liked_asked.as_deref() == Some(&uri) {
                    self.liked_retry = Some(now + e.retry_after(RETRY).unwrap_or(RETRY));
                }
            }
            (Want::Liked(_), Ok(_)) => self.liked_retry = Some(now + RETRY),
            (Want::Like { .. }, Ok(_)) => {}
            (Want::Like { uri, on }, Err(e)) => {
                if self.liked.as_ref().is_some_and(|(u, _)| *u == uri) {
                    self.liked = Some((uri, !on));
                }
                return Some(e.to_string());
            }
            (
                Want::Add {
                    playlist_id,
                    uri,
                    name,
                    unchecked,
                },
                Ok(reply),
            ) => {
                if let Reply::Snapshot(snapshot) = reply {
                    self.added(&playlist_id, uri, snapshot);
                }
                return Some(if unchecked {
                    format!("added to {name} · couldn't check it first")
                } else {
                    format!("added to {name}")
                });
            }
            (Want::Add { .. }, Err(e)) => return Some(e.to_string()),
            (
                Want::Uris {
                    playlist_id,
                    snapshot,
                    offset,
                },
                result,
            ) => self.read(&playlist_id, &snapshot, offset, result, now),
            (Want::Player, result) => {
                let p = &mut self.player;
                p.in_flight = false;
                match result {
                    Ok(Reply::Player(state)) => {
                        // Only a device to control says anything about it.
                        if state.is_some() {
                            p.allowed.get_or_insert(true);
                        }
                        p.state = state;
                        p.state_for.clone_from(&p.asked_for);
                    }
                    Err(Error::Forbidden(why)) => {
                        p.allowed = Some(false);
                        p.needs_login = why.contains("scope");
                        p.state = None;
                    }
                    _ => {}
                }
            }
            (Want::Mode, Ok(_)) => {}
            (Want::Mode, Err(e)) => {
                // Read it back: our optimistic guess may be wrong.
                self.player.asked_at = None;
                match &e {
                    Error::Forbidden(why) if refuses_account(why) => {
                        self.player.allowed = Some(false);
                        self.player.needs_login = why.contains("scope");
                        self.player.state = None;
                    }
                    // "Restriction violated": not here, not now (a lone
                    // track, an ad); the next read says what's allowed.
                    Error::Forbidden(_) => {
                        return Some("Spotify won't change that right now".into());
                    }
                    _ => {}
                }
                return Some(e.to_string());
            }
            (Want::Play(_), Ok(_)) => self.player.asked_at = Some(now - PLAYER_EVERY),
            (Want::Play(playing), Err(e)) => {
                if matches!(&e, Error::Forbidden(why) if refuses_account(why)) {
                    self.player.allowed = Some(false);
                }
                if !matches!(e, Error::Forbidden(_)) {
                    return Some(e.to_string());
                }
                self.fallback = Some(playing);
            }
        }
        None
    }
}

/// Where `store` keeps the login, in plain words.
pub fn store_name(store: LoginStore) -> &'static str {
    match store {
        LoginStore::System if cfg!(target_os = "macos") => "the Keychain",
        LoginStore::System => "the system's password store",
        LoginStore::File => "a private file",
    }
}

/// Two names for the same thing: the same words, case and punctuation
/// aside.
fn same_name(a: &str, b: &str) -> bool {
    let a = words(a);
    !a.is_empty() && a == words(b)
}

/// Lowercase letters and digits, split at anything else.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// How far apart two reports of one track's length may be (the desktop
/// app rounds to whole seconds; Spotify says milliseconds).
const SAME_LENGTH: Duration = Duration::from_secs(2);

/// Whether `track`, from a player that names no Spotify URI (Windows'
/// media controls), is the Web API's `item` (lava-1xk.35). Titles alone
/// say little ("Intro", covers, remasters): the artists must agree too,
/// then the length (within [`SAME_LENGTH`]) and the album wherever both
/// say, and at least one of them must be said. Local files and anything
/// that isn't a catalog track never match; in doubt, it's not the same.
fn same_track(track: &crate::media::Track, item: &Track) -> bool {
    if item.is_local || !item.uri.starts_with("spotify:track:") {
        return false;
    }
    if !same_name(&track.name, &item.name) {
        return false;
    }
    // The desktop lists every artist ("A, B") or just the first.
    let artists = words(&track.artist);
    let all: Vec<String> = item.artists.iter().flat_map(|a| words(a)).collect();
    let first = item.artists.first().map(|a| words(a)).unwrap_or_default();
    if artists.is_empty() || (artists != all && artists != first) {
        return false;
    }
    // Then whatever else both say must agree, and something must.
    let length = Duration::from_millis(u64::from(item.duration_ms));
    let lengths = (!track.duration.is_zero() && !length.is_zero())
        .then(|| track.duration.abs_diff(length) <= SAME_LENGTH);
    let albums = (!words(&track.album).is_empty() && !words(&item.album).is_empty())
        .then(|| same_name(&track.album, &item.album));
    lengths != Some(false) && albums != Some(false) && (lengths.is_some() || albums.is_some())
}

/// A 403 about the account (no Premium, a login without the playback
/// scopes) rather than the moment ("Restriction violated").
fn refuses_account(why: &str) -> bool {
    let why = why.to_ascii_lowercase();
    why.contains("premium") || why.contains("scope")
}

fn message(result: Result<Reply, Error>) -> String {
    match result {
        Err(e) => e.to_string(),
        Ok(_) => "unexpected reply".into(),
    }
}

/// Which list the library overlay shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    /// All my playlists: open the ones I can read, play any.
    Playlists,
    /// The open playlist's tracks.
    Tracks,
    /// The playlists the playing track can be added to.
    AddTo,
}

/// The library overlay: which list, where the cursor is. The items live
/// on the [`Library`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListView {
    pub kind: ListKind,
    pub cursor: usize,
    /// First row shown; follows the cursor.
    pub top: usize,
    /// The playlist (its index in all of them) to go back to from its
    /// tracks.
    pub back: usize,
    /// The filter row is open: keys type into it (lava-75z.17).
    pub typing: bool,
}

impl ListView {
    fn new(kind: ListKind) -> Self {
        Self {
            kind,
            cursor: 0,
            top: 0,
            back: 0,
            typing: false,
        }
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            ListKind::Playlists | ListKind::Tracks => "playlists",
            ListKind::AddTo => "add to playlist",
        }
    }
}

/// One row of the library overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub name: String,
    /// Right-hand detail (a count) or the artist, dim.
    pub detail: String,
    /// Shown quieter: playlists that can't be opened, local files.
    pub quiet: bool,
}

impl Model {
    /// Connect / drain the library and patch the snapshot with the Web
    /// API's shuffle / repeat. Once a frame.
    pub(super) fn sync_library(&mut self) {
        let track = self
            .music
            .snapshot
            .as_ref()
            .and_then(|s| s.track.as_ref())
            .map(|t| t.id.clone());
        let uri = self.playing_uri().map(str::to_owned);
        let toasts = self.library.sync(
            self.music_on() || self.spotify_setup_open(),
            self.now,
            track.as_deref(),
            uri.as_deref(),
        );
        if let Some(last) = toasts.into_iter().last() {
            self.toast(last);
        }
        // The setup says how the login stands: read it.
        if self.spotify_setup_open() {
            self.unlock_library(None);
        }
        // The demo's made-up login is never written down.
        if !self.library.demo && self.settings.spotify.logged_in != self.library.saved {
            self.settings.spotify.logged_in = self.library.saved;
            self.changed(self.now);
        }
        if let Some(playing) = self.library.fallback.take() {
            self.play_here(&playing);
        }
        if let Overlay::Library(view) = self.overlay
            && std::mem::take(&mut self.library.find.stale)
        {
            self.refind(view.kind);
            self.overlay = Overlay::Library(self.follow_list(view));
            self.load_more(&view);
        }
        // A page that failed is asked for again once the wait is over.
        if let Overlay::Library(view) = self.overlay
            && let Some(open) = &self.library.open
            && open.retry.is_some_and(|at| self.now >= at)
        {
            self.load_more(&view);
        }
        self.check_adding();
        self.patch_modes();
        self.replay_pending();
    }

    /// The add picker reads its playlists' songs ahead, and an add waiting
    /// on that check goes on: to the question when the song is there, else
    /// straight in (also when the check failed or stalled: never lost).
    fn check_adding(&mut self) {
        let view = match self.overlay {
            Overlay::Library(view) if view.kind == ListKind::AddTo => view,
            _ => {
                self.library.adding = None;
                self.library.read_budget = 0;
                return;
            }
        };
        let now = self.now;
        // The cursor's playlist first, then the rest in order.
        let editable = self.library.editable();
        let at = self
            .item_index(ListKind::AddTo, view.cursor)
            .and_then(|i| editable.get(i).copied());
        let order: Vec<usize> = at.into_iter().chain(editable).collect();
        let lib = &mut self.library;
        let chosen = lib.adding.as_ref().map(|a| a.playlist_id.clone());
        lib.read_ahead(chosen.as_deref(), &order, now);

        let Some(adding) = &lib.adding else {
            return;
        };
        let Stage::Checking { since, read, total } = adding.stage else {
            return;
        };
        if lib.playlists.loading {
            // Waiting for the playlists to be read again isn't a stall.
            if let Some(a) = &mut lib.adding {
                a.stage = Stage::Checking {
                    since: now,
                    read,
                    total,
                };
            }
            return;
        }
        let playlist = lib
            .playlists
            .items
            .iter()
            .find(|p| p.id == adding.playlist_id);
        let has = playlist.and_then(|p| lib.has(p, &adding.uri));
        let failed = playlist.is_none_or(|p| {
            lib.contents
                .get(&p.id)
                .is_some_and(|c| c.snapshot == p.snapshot_id && c.failed)
        });
        match has {
            Some(true) => {
                if let Some(a) = &mut lib.adding {
                    a.stage = Stage::Confirm;
                }
            }
            Some(false) => self.finish_adding(false),
            None if failed || now - since >= CHECK_STALL => self.finish_adding(true),
            None => {}
        }
    }

    /// Add the waiting song now, and close the picker.
    fn finish_adding(&mut self, unchecked: bool) {
        if let Some(a) = self.library.adding.take() {
            self.library.add(a.playlist_id, a.name, a.uri, unchecked);
            self.overlay = Overlay::None;
        }
    }

    /// Keys while an add waits: `⏎` adds (again, or without waiting for
    /// the check), `esc` goes back to the list, `q` closes it. Everything
    /// else waits. Returns whether the action was its own.
    fn adding_action(&mut self, view: ListView, action: Action) -> bool {
        match action {
            Action::Keep => {
                // Asked and answered: it's not unchecked.
                self.finish_adding(false);
                return true;
            }
            Action::Back | Action::ClearFind => self.library.adding = None,
            Action::Close => {
                self.library.adding = None;
                self.overlay = Overlay::None;
                return true;
            }
            Action::Up
            | Action::Down
            | Action::Page(_)
            | Action::Edge(_)
            | Action::Find
            | Action::Type(_)
            | Action::Erase
            | Action::PlayAll
            | Action::Click { .. } => {}
            _ => return false,
        }
        self.overlay = Overlay::Library(view);
        true
    }

    /// Read the saved login, first saying that macOS may ask about it;
    /// `key` runs again once it's read. Nothing to do once it's been read.
    pub(super) fn unlock_library(&mut self, key: Option<PlayerKey>) {
        if !self.library.unlock() {
            return;
        }
        self.library.pending = key.map(|k| (k, self.now));
        self.toast(KEYCHAIN_HEADS_UP);
    }

    /// The key that waited for the saved login, once it's read (shuffle and
    /// repeat also wait for the player's state, which says whether they go
    /// through the account).
    fn replay_pending(&mut self) {
        let Some((key, at)) = self.library.pending else {
            return;
        };
        if self.now - at > PENDING_FOR {
            self.library.pending = None;
            return;
        }
        let lib = &self.library;
        let waiting = !lib.unlocked
            || (matches!(key, PlayerKey::Shuffle | PlayerKey::Repeat)
                && lib.logged_in()
                && lib.player.allowed != Some(false)
                && (lib.player.in_flight || lib.player.asked_at.is_none()));
        if waiting {
            return;
        }
        self.library.pending = None;
        if key == PlayerKey::Account && self.library.logged_in() {
            // It was a saved login, not a new one to start.
            self.toast("logged in to Spotify");
            return;
        }
        self.player_key(key, self.now);
    }

    /// Shuffle / repeat from the Web API, where the desktop app's own
    /// controls can't change them, and only while the Web API's player is
    /// the one shown (lava-1xk.27): another player keeps its own modes.
    pub(super) fn patch_modes(&mut self) {
        let state = self.web_modes();
        let modes = state.map(|p| (p.shuffle, p.repeat != Repeat::Off));
        self.music.web_caps = state.map_or(Capabilities::NONE, |p| Capabilities {
            shuffle: !p.shuffle_blocked,
            repeat: !p.repeat_blocked,
            ..Capabilities::NONE
        });
        if let (Some((shuffle, repeat)), Some(snap)) = (modes, &mut self.music.snapshot) {
            snap.shuffle = shuffle;
            snap.repeat = repeat;
        }
    }

    /// The Web API's player state when it is about what the music widget
    /// shows (lava-1xk.24, lava-1xk.27): the same Spotify track by URI,
    /// or, for a player that names no URI (Windows' media controls), the
    /// Spotify app playing the same track by its title, artists and length
    /// ([`same_track`]), in a state asked for since that track began.
    /// Another player, or Spotify playing something else (or something
    /// it can't be sure is this) on another device, is not it: in doubt,
    /// no match.
    fn web_player(&self) -> Option<&PlayerState> {
        if !self.library.logged_in() {
            return None;
        }
        let player = &self.library.player;
        let state = player.state.as_ref()?;
        let snap = self.music.snapshot.as_ref()?;
        let track = snap.track.as_ref()?;
        let item = state.item.as_ref()?;
        let same = match &track.uri {
            Some(uri) => *uri == item.uri,
            None => {
                snap.is_spotify()
                    && player.state_for.as_deref() == Some(track.id.as_str())
                    && same_track(track, item)
            }
        };
        same.then_some(state)
    }

    /// [`Self::web_player`] when shuffle / repeat can go through it
    /// (Spotify said yes to this account).
    pub(super) fn web_modes(&self) -> Option<&PlayerState> {
        self.library.modes()?;
        self.web_player()
    }

    /// The playing track's Spotify URI: what the player says, else (on
    /// Windows) what the Web API's matching player says. `None` for local
    /// files, ads, episodes and other players: like and add-to-playlist
    /// say there's nothing to act on.
    pub(crate) fn playing_uri(&self) -> Option<&str> {
        let track = self.music.snapshot.as_ref()?.track.as_ref()?;
        if let Some(uri) = &track.uri {
            return Some(uri);
        }
        let item = self.web_player()?.item_uri()?;
        item.starts_with("spotify:track:").then_some(item)
    }

    /// Whether the playing track is liked (for the heart), once known.
    pub fn liked(&self) -> Option<bool> {
        self.library.liked(self.playing_uri()?)
    }

    /// A library key with no Client ID (or music off) says why; one that
    /// Spotify refused opens the setup, which says how to fix it; one that
    /// needs the saved login read first reads it, and `key` runs again
    /// when it's done.
    fn library_ready(&mut self, key: PlayerKey) -> bool {
        if self.library.refused.is_some() {
            self.open_settings_at(super::settings_screen::Page::Spotify, true);
            return false;
        }
        if self.library.account() == Account::Unavailable {
            if !self.music_on() {
                self.toast("music is off · a to show it");
            } else if !self.spotify_set_up() {
                // Nothing to log in with yet: the guided setup says how.
                self.open_settings_at(super::settings_screen::Page::Spotify, true);
            } else {
                self.toast("connecting to Spotify…");
            }
            return false;
        }
        if self.library.locked() {
            self.unlock_library(Some(key));
            return false;
        }
        if self.library.pending.is_some() {
            // Still reading it (macOS may be asking): the first key runs.
            return false;
        }
        true
    }

    /// `i`: log in (browser); while waiting, cancel; logged in, twice to
    /// log out.
    pub(super) fn account_key(&mut self, now: Instant) {
        if !self.library_ready(PlayerKey::Account) {
            return;
        }
        let account = self.library.account();
        let lib = &mut self.library;
        let toast = match account {
            Account::Unavailable => return,
            Account::LoggingIn => {
                lib.cancel_login();
                "login cancelled".to_owned()
            }
            Account::LoggedIn => {
                let armed = lib
                    .logout_armed
                    .take()
                    .is_some_and(|at| now - at < LOGOUT_WINDOW);
                if armed {
                    // `LoggedOut` toasts when it's done.
                    lib.logout();
                    return;
                }
                lib.logout_armed = Some(now);
                "press i again to log out of Spotify".to_owned()
            }
            Account::LoggedOut => lib
                .start_login()
                .unwrap_or_else(|| "log in to Spotify in your browser".to_owned()),
        };
        self.toast(toast);
    }

    /// `s`: like or unlike the playing track (the heart changes at once).
    pub(super) fn like_key(&mut self) {
        if !self.library_ready(PlayerKey::Like) || !self.logged_in_or_say() {
            return;
        }
        let Some(uri) = self.playing_uri().map(str::to_owned) else {
            self.toast("nothing to like");
            return;
        };
        let on = !self.library.liked(&uri).unwrap_or(false);
        self.library.liked = Some((uri.clone(), on));
        self.library.liked_asked = Some(uri.clone());
        let uris = vec![uri.clone()];
        let request = if on {
            Request::Like { uris }
        } else {
            Request::Unlike { uris }
        };
        self.library.request(request, Want::Like { uri, on });
        let g = self.glyphs();
        self.toast(if on { g.liked_toast } else { g.unliked_toast });
    }

    /// A Client ID to log in with (or the demo's made-up account).
    pub(super) fn spotify_set_up(&self) -> bool {
        self.library.demo || self.settings.spotify_client_id().is_some()
    }

    fn logged_in_or_say(&mut self) -> bool {
        match self.library.account() {
            Account::LoggedIn => true,
            Account::LoggingIn => {
                self.toast("finish logging in in your browser");
                false
            }
            _ => {
                self.toast("not logged in to Spotify · i to log in");
                false
            }
        }
    }

    /// `b` / `a`: the playlist browser or the add-to-playlist picker.
    /// Logged out, it opens anyway and offers the login.
    pub(super) fn open_library(&mut self, kind: ListKind) {
        let key = match kind {
            ListKind::AddTo => PlayerKey::AddToPlaylist,
            _ => PlayerKey::Playlists,
        };
        if !self.library_ready(key) {
            return;
        }
        if kind == ListKind::AddTo && self.library.logged_in() && self.playing_uri().is_none() {
            self.toast("nothing playing to add");
            return;
        }
        self.refresh_playlists();
        self.last_click = None;
        self.library.find = Find::default();
        self.library.adding = None;
        if kind == ListKind::AddTo {
            self.library.read_budget = READ_AHEAD_PAGES;
        }
        self.overlay = Overlay::Library(self.follow_list(ListView::new(kind)));
    }

    /// Ask for my playlists again (shown as they were until it answers).
    fn refresh_playlists(&mut self) {
        if self.library.logged_in() && !self.library.playlists.loading {
            self.library.playlists.loading = true;
            self.library.playlists.error = None;
            self.library.request(Request::MyPlaylists, Want::Playlists);
        }
    }

    /// How many rows `kind` shows right now (those matching the filter).
    pub fn list_len(&self, kind: ListKind) -> usize {
        match self.hits(kind) {
            Some(hits) => hits.len(),
            None => self.list_total(kind),
        }
    }

    /// The filter's matches in `kind`, while one is typed.
    fn hits(&self, kind: ListKind) -> Option<&[usize]> {
        match &self.library.find.hits {
            Some((k, hits)) if *k == kind => Some(hits),
            _ => None,
        }
    }

    /// Which item of all of `kind` row `i` shows.
    fn item_index(&self, kind: ListKind, i: usize) -> Option<usize> {
        match self.hits(kind) {
            Some(hits) => hits.get(i).copied(),
            None => (i < self.list_total(kind)).then_some(i),
        }
    }

    /// How many items `kind` has, filter or not.
    pub fn list_total(&self, kind: ListKind) -> usize {
        let lib = &self.library;
        if !lib.logged_in() {
            return 0;
        }
        match kind {
            ListKind::Playlists => lib.playlists.items.len(),
            ListKind::AddTo => lib.editable().len(),
            ListKind::Tracks => lib.open.as_ref().map_or(0, |o| o.tracks.items.len()),
        }
    }

    /// Row `i` of `kind` (as filtered).
    pub fn list_row(&self, kind: ListKind, i: usize) -> Option<ListRow> {
        self.item_row(kind, self.item_index(kind, i)?)
    }

    /// Item `i` of all of `kind`.
    fn item_row(&self, kind: ListKind, i: usize) -> Option<ListRow> {
        let lib = &self.library;
        let playlist_row = |p: &Playlist, quiet: bool| ListRow {
            name: p.name.clone(),
            detail: p.total.to_string(),
            quiet,
        };
        match kind {
            ListKind::Playlists => {
                let p = lib.playlists.items.get(i)?;
                let open = lib.me.as_ref().is_none_or(|me| p.editable_by(me));
                Some(playlist_row(p, !open))
            }
            ListKind::AddTo => {
                let p = &lib.playlists.items[*lib.editable().get(i)?];
                let mut row = playlist_row(p, false);
                // A quiet mark on the playlists that have the song already.
                if let Some(uri) = self.playing_uri()
                    && lib.has(p, uri) == Some(true)
                {
                    row.detail = format!("{} {}", self.glyphs().has, row.detail);
                }
                Some(row)
            }
            ListKind::Tracks => {
                let t = lib.open.as_ref()?.tracks.items.get(i)?;
                Some(ListRow {
                    name: t.name.clone(),
                    detail: t.artist_line(),
                    quiet: t.is_local,
                })
            }
        }
    }

    /// What the list says when it has no rows (loading, logged out, …).
    pub fn list_message(&self, kind: ListKind) -> String {
        let lib = &self.library;
        match lib.account() {
            Account::Unavailable => return "no Spotify Client ID".into(),
            Account::LoggedOut => return "not logged in · Enter to log in".into(),
            Account::LoggingIn => return "finish logging in in your browser…".into(),
            Account::LoggedIn if lib.refused.is_some() => {
                return "Spotify refused this account · settings (,) › spotify says why".into();
            }
            Account::LoggedIn => {}
        }
        if self.hits(kind).is_some() && self.list_total(kind) > 0 {
            let more = kind == ListKind::Tracks && lib.open.as_ref().is_some_and(|o| o.has_more);
            return if more {
                "looking…".into()
            } else {
                format!("nothing matches “{}”", lib.find.text.trim())
            };
        }
        let (listing_loading, error, loaded) = match kind {
            ListKind::Tracks => match &lib.open {
                Some(o) => (o.tracks.loading, o.tracks.error.clone(), o.tracks.loaded),
                None => (false, None, true),
            },
            _ => (
                lib.playlists.loading || lib.me.is_none(),
                lib.playlists.error.clone(),
                lib.playlists.loaded,
            ),
        };
        if let Some(e) = error {
            return e;
        }
        if listing_loading || !loaded {
            return "loading…".into();
        }
        match kind {
            ListKind::Playlists => "no playlists".into(),
            ListKind::AddTo => "no playlists you can add to".into(),
            ListKind::Tracks => "an empty playlist".into(),
        }
    }

    /// The title of the open list: the playlist's name in its tracks.
    pub fn list_title(&self, view: &ListView) -> String {
        match (view.kind, &self.library.open) {
            (ListKind::Tracks, Some(o)) => o.playlist.name.clone(),
            _ => view.title().into(),
        }
    }

    /// Keys and clicks while the library overlay is open. Returns whether
    /// the action was its own.
    pub(super) fn library_action(&mut self, mut view: ListView, action: Action) -> bool {
        if self.library.adding.is_some() {
            return self.adding_action(view, action);
        }
        let n = self.list_len(view.kind);
        let rows = self.list_rows(&view);
        match action {
            Action::Up if n > 0 => view.cursor = (view.cursor + n - 1) % n,
            Action::Down if n > 0 => view.cursor = (view.cursor + 1) % n,
            Action::Page(down) if n > 0 => {
                view.cursor = if down {
                    (view.cursor + rows).min(n - 1)
                } else {
                    view.cursor.saturating_sub(rows)
                }
            }
            Action::Edge(end) if n > 0 => view.cursor = if end { n - 1 } else { 0 },
            Action::Up | Action::Down | Action::Page(_) | Action::Edge(_) => {}
            Action::Keep => return self.choose(view),
            Action::PlayAll => self.play_all(view),
            Action::Find => view.typing = true,
            Action::Type(c) if view.typing => {
                self.library.find.text.push(c);
                view = self.refound(view, None);
            }
            Action::Erase if view.typing => {
                if self.library.find.text.pop().is_none() {
                    view.typing = false;
                }
                view = self.refound(view, None);
            }
            // The row under the cursor stays under it, all rows back.
            Action::ClearFind => {
                let at = self.item_index(view.kind, view.cursor);
                self.library.find.text.clear();
                view.typing = false;
                view = self.refound(view, at);
            }
            Action::Back if view.kind == ListKind::Tracks => {
                self.library.find.text = std::mem::take(&mut self.library.find.back);
                let mut back = ListView::new(ListKind::Playlists);
                back.typing = !self.library.find.text.is_empty();
                view = self.refound(back, Some(view.back));
            }
            Action::Back | Action::Close => {
                self.overlay = Overlay::None;
                return true;
            }
            Action::Click { col, row } => {
                let now = self.now;
                match self.list_hit(&view, col, row) {
                    Some(Hit::Prev) if n > 0 => view.cursor = (view.cursor + n - 1) % n,
                    Some(Hit::Next) if n > 0 => view.cursor = (view.cursor + 1) % n,
                    Some(Hit::Item(i)) if i < n => {
                        let double = self
                            .last_click
                            .is_some_and(|(j, at)| j == i && now - at < DOUBLE_CLICK);
                        view.cursor = i;
                        if double {
                            self.last_click = None;
                            return self.choose(view);
                        }
                        self.last_click = Some((i, now));
                    }
                    // A click on the logged-out message logs in.
                    Some(Hit::Item(_)) if self.library.account() == Account::LoggedOut => {
                        return self.choose(view);
                    }
                    _ => {}
                }
            }
            _ => return false,
        }
        self.overlay = Overlay::Library(self.follow_list(view));
        self.load_more(&view);
        true
    }

    /// Work out the filter's matches in `kind` again.
    fn refind(&mut self, kind: ListKind) {
        let query: Vec<String> = self
            .library
            .find
            .text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let hits = (!query.is_empty()).then(|| {
            (0..self.list_total(kind))
                .filter(|&i| {
                    self.item_row(kind, i).is_some_and(|row| {
                        let hay = match kind {
                            ListKind::Tracks => format!("{} {}", row.name, row.detail),
                            // A playlist's detail is its count.
                            ListKind::Playlists | ListKind::AddTo => row.name,
                        }
                        .to_lowercase();
                        query.iter().all(|word| hay.contains(word.as_str()))
                    })
                })
                .collect()
        });
        self.library.find.hits = hits.map(|h| (kind, h));
        self.library.find.stale = false;
    }

    /// `view` after the filter changed: on item `at` (of all of them) if
    /// it's still shown, else the first row.
    fn refound(&mut self, mut view: ListView, at: Option<usize>) -> ListView {
        self.refind(view.kind);
        view.cursor = at
            .and_then(|at| match self.hits(view.kind) {
                Some(hits) => hits.iter().position(|&i| i == at),
                None => Some(at),
            })
            .unwrap_or(0);
        view.top = 0;
        view
    }

    /// `⏎` on the cursor's row.
    fn choose(&mut self, view: ListView) -> bool {
        let now = self.now;
        if !self.library.logged_in() {
            if self.library.account() == Account::LoggedOut {
                self.account_key(now);
            }
            self.overlay = Overlay::Library(view);
            return true;
        }
        let Some(at) = self.item_index(view.kind, view.cursor) else {
            self.overlay = Overlay::Library(view);
            return true;
        };
        match view.kind {
            ListKind::Playlists => {
                let Some(p) = self.library.playlists.items.get(at).cloned() else {
                    return true;
                };
                let readable = self.library.me.as_ref().is_some_and(|me| p.editable_by(me));
                if !readable {
                    // Spotify only shows the items of playlists you own or
                    // collaborate on: play it instead.
                    self.play_context(&p.uri, None, &p.name);
                    self.overlay = Overlay::Library(view);
                    return true;
                }
                let id = p.id.clone();
                self.library.openings += 1;
                let opening = self.library.openings;
                self.library.open = Some(OpenPlaylist {
                    playlist: p,
                    tracks: Listing {
                        loading: true,
                        ..Listing::default()
                    },
                    next_offset: 0,
                    has_more: false,
                    opening,
                    retry: None,
                });
                let request = Request::PlaylistTracks {
                    playlist_id: id.clone(),
                    offset: 0,
                };
                let want = Want::Tracks {
                    playlist_id: id,
                    opening,
                    offset: 0,
                };
                self.library.request(request, want);
                let find = &mut self.library.find;
                find.back = std::mem::take(&mut find.text);
                self.refind(ListKind::Tracks);
                self.overlay = Overlay::Library(ListView {
                    back: at,
                    ..ListView::new(ListKind::Tracks)
                });
            }
            ListKind::Tracks => {
                let Some(open) = &self.library.open else {
                    return true;
                };
                if let Some(t) = open.tracks.items.get(at) {
                    let (context, uri, name) =
                        (open.playlist.uri.clone(), t.uri.clone(), t.name.clone());
                    self.play_context(&context, Some(&uri), &name);
                }
                self.overlay = Overlay::Library(view);
            }
            ListKind::AddTo => {
                let Some(&at) = self.library.editable().get(at) else {
                    return true;
                };
                let Some(uri) = self.playing_uri().map(str::to_owned) else {
                    self.toast("nothing playing to add");
                    return true;
                };
                let p = &self.library.playlists.items[at];
                // Asks first if it's there already (`check_adding`).
                self.library.adding = Some(Adding {
                    playlist_id: p.id.clone(),
                    name: p.name.clone(),
                    uri,
                    stage: Stage::Checking {
                        since: self.now,
                        read: 0,
                        total: p.total,
                    },
                });
                self.overlay = Overlay::Library(view);
                self.check_adding();
            }
        }
        true
    }

    /// `p` in the browser: play the cursor's playlist (or the open one)
    /// from the top.
    fn play_all(&mut self, view: ListView) {
        let at = self.item_index(view.kind, view.cursor);
        let lib = &self.library;
        let playlist = match view.kind {
            ListKind::Playlists => at.and_then(|at| lib.playlists.items.get(at)),
            ListKind::Tracks => lib.open.as_ref().map(|o| &o.playlist),
            ListKind::AddTo => None,
        };
        if let Some(p) = playlist {
            let (uri, name) = (p.uri.clone(), p.name.clone());
            self.play_context(&uri, None, &name);
        }
    }

    /// Play `context` (a playlist) from `track` in it: through the Web
    /// API's player when it's there (Premium), else the desktop app
    /// (lava-75z.18: on macOS it plays the track in its playlist too, on
    /// Linux the track alone; Windows' media controls can't be told what
    /// to play, lava-1xk.25).
    fn play_context(&mut self, context: &str, track: Option<&str>, name: &str) {
        let playing = Playing {
            context: context.to_owned(),
            track: track.map(str::to_owned),
            name: name.to_owned(),
        };
        if self.library.modes().is_some() {
            let request = Request::Play {
                context_uri: playing.context.clone(),
                offset_uri: playing.track.clone(),
            };
            self.library.request(request, Want::Play(playing));
            self.toast(format!("playing {name}"));
            return;
        }
        self.play_here(&playing);
    }

    /// The desktop app plays it, or the toast says truthfully why not.
    fn play_here(&mut self, playing: &Playing) {
        let now = self.now;
        let caps = self.music.source_capabilities();
        let available = self
            .music
            .snapshot
            .as_ref()
            .is_some_and(|s| s.status.is_available());
        if available && !caps.uris {
            let player = &self.library.player;
            self.toast(if player.needs_login {
                "log in to Spotify again to play from here (i twice, then i)"
            } else if player.allowed == Some(false) {
                "playing from here needs Spotify Premium"
            } else {
                "press play in Spotify first, then pick it again"
            });
            return;
        }
        let played = match &playing.track {
            Some(track) => self.music.play_in_context(track, &playing.context, now),
            None => self.music.play_uri(&playing.context, now),
        };
        match played {
            Ok(()) if playing.track.is_some() && !caps.contexts => {
                self.toast(format!("playing {} · just this song", playing.name))
            }
            Ok(()) => self.toast(format!("playing {}", playing.name)),
            Err(why) => self.toast(why),
        }
    }

    /// Item rows the open sheet shows (1 inline).
    fn list_rows(&self, view: &ListView) -> usize {
        match self.list_placement(view) {
            Some(Placement::Sheet { list, .. }) => usize::from(list.height).max(1),
            _ => 1,
        }
    }

    pub fn list_placement(&self, view: &ListView) -> Option<Placement> {
        crate::ui::library::placement(self.layout.area, &self.layout, view, self)
    }

    fn list_hit(&self, view: &ListView, col: u16, row: u16) -> Option<Hit> {
        let place = self.list_placement(view)?;
        let n = self.list_len(view.kind).max(1);
        picker::hit_in(place, view.top, view.cursor, n, col, row)
    }

    /// Keep the cursor in view (and in range).
    fn follow_list(&self, mut view: ListView) -> ListView {
        let n = self.list_len(view.kind);
        view.cursor = view.cursor.min(n.saturating_sub(1));
        let rows = self.list_rows(&view);
        view.top = picker::visible_top(view.top, view.cursor, rows, n);
        view
    }

    /// Ask for the open playlist's next page when the cursor nears the end
    /// of what's shown, or while a filter is typed (it looks through all).
    fn load_more(&mut self, view: &ListView) {
        if view.kind != ListKind::Tracks {
            return;
        }
        let shown = self.list_len(ListKind::Tracks);
        let finding = self.hits(ListKind::Tracks).is_some();
        let Some(open) = &mut self.library.open else {
            return;
        };
        let next_offset = open.next_offset;
        let near_end = finding || view.cursor + PREFETCH >= shown;
        if open.retry.is_some_and(|at| self.now < at) {
            return;
        }
        open.retry = None;
        if open.has_more && !open.tracks.loading && near_end {
            open.tracks.loading = true;
            let playlist_id = open.playlist.id.clone();
            let request = Request::PlaylistTracks {
                playlist_id: playlist_id.clone(),
                offset: next_offset,
            };
            let want = Want::Tracks {
                playlist_id,
                opening: open.opening,
                offset: next_offset,
            };
            self.library.request(request, want);
        }
    }

    /// `x` / `r` through the Web API (lava-75z.12): optimistic, read back if
    /// Spotify refuses.
    pub(super) fn web_mode(&mut self, shuffle: bool) {
        let Some(state) = self.library.player.state.as_mut() else {
            return;
        };
        let (request, toast) = if shuffle {
            state.shuffle = !state.shuffle;
            let toast = if state.shuffle {
                "shuffle on"
            } else {
                "shuffle off"
            };
            (Request::SetShuffle(state.shuffle), toast)
        } else {
            state.repeat = match state.repeat {
                Repeat::Off => Repeat::Context,
                Repeat::Context | Repeat::Track => Repeat::Off,
            };
            let toast = if state.repeat == Repeat::Off {
                "repeat off"
            } else {
                "repeat on"
            };
            (Request::SetRepeat(state.repeat), toast)
        };
        self.toast(toast);
        self.library.request(request, Want::Mode);
        self.patch_modes();
    }
}
