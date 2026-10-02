//! The Spotify library (Web API): login, the playlist browser and the
//! add-to-playlist picker, like / unlike, and shuffle / repeat through the
//! player endpoints where the account allows them.
//!
//! All network work happens on the [`SpotifyWeb`] worker: the model only
//! queues [`Request`]s and drains [`Event`]s once a frame ([`Library::sync`],
//! bounded), so nothing here blocks a frame. The client exists only while
//! the music widget is placed and a Client ID is configured
//! (`spotify.client_id` or `LAVATUI_SPOTIFY_CLIENT_ID`); tests plug in a
//! `FakeWeb`.

use std::time::{Duration, Instant};

use super::{Model, Overlay};
use crate::media::Capabilities;
use crate::spotify_web::{
    Error, Event, PlayerState, Playlist, Repeat, Reply, Request, RequestId, Track, User, Web,
};
use crate::ui::keymap::Action;
use crate::ui::picker::{self, Hit, Placement};

/// How often the player state is read again while nothing changes.
const PLAYER_EVERY: Duration = Duration::from_secs(30);
/// The second `i` within this logs out.
const LOGOUT_WINDOW: Duration = Duration::from_secs(2);
/// Events handled per frame at most (the rest wait for the next one).
const EVENTS_PER_FRAME: usize = 32;
/// The next page of a playlist is asked for when the cursor comes this
/// close to the end of what has loaded.
const PREFETCH: usize = 10;
/// Two clicks on one row this close together choose it.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

type Connect = Box<dyn Fn() -> Option<Box<dyn Web>>>;

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
    pub has_more: bool,
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
    /// The track playing when the state was last asked for.
    asked_for: Option<String>,
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
    Tracks {
        playlist_id: String,
    },
    Liked(String),
    Like {
        uri: String,
        on: bool,
    },
    Add {
        name: String,
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
    wants: Vec<(RequestId, Want)>,
    /// The consent page, while a login waits for the browser.
    pub login_url: Option<String>,
    pub me: Option<User>,
    me_asked: bool,
    pub playlists: Listing<Playlist>,
    pub open: Option<OpenPlaylist>,
    /// The playing track's URI and whether it's liked.
    pub liked: Option<(String, bool)>,
    liked_asked: Option<String>,
    pub player: WebPlayer,
    logout_armed: Option<Instant>,
    /// A refused context play: the desktop app should play it instead.
    fallback: Option<Playing>,
    /// What the open list is filtered by (lava-75z.17).
    pub find: Find,
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
    /// The library for `client_id` (none: off). Tests never reach the real
    /// thing (a keyring read could prompt): they plug in a fake.
    pub fn new(client_id: Option<String>) -> Self {
        #[cfg(not(test))]
        let connect: Connect = Box::new(move || {
            client_id
                .clone()
                .map(|id| Box::new(crate::spotify_web::SpotifyWeb::new(id)) as Box<dyn Web>)
        });
        #[cfg(test)]
        let connect: Connect = {
            let _ = client_id;
            Box::new(|| None)
        };
        Self {
            web: None,
            connect,
            wants: Vec::new(),
            login_url: None,
            me: None,
            me_asked: false,
            playlists: Listing::default(),
            open: None,
            liked: None,
            liked_asked: None,
            player: WebPlayer::default(),
            logout_armed: None,
            fallback: None,
            find: Find::default(),
        }
    }

    /// Use `connect` for the client, from the next time music is placed.
    #[cfg(test)]
    pub fn connect_with(&mut self, connect: impl Fn() -> Option<Box<dyn Web>> + 'static) {
        self.connect = Box::new(connect);
        self.disconnect();
    }

    fn disconnect(&mut self) {
        if let Some(web) = &mut self.web {
            web.cancel_login();
        }
        self.web = None;
        self.forget();
        self.login_url = None;
    }

    /// Drop everything that belonged to a login.
    fn forget(&mut self) {
        self.wants.clear();
        self.me = None;
        self.me_asked = false;
        self.playlists = Listing::default();
        self.open = None;
        self.liked = None;
        self.liked_asked = None;
        self.player = WebPlayer::default();
        self.logout_armed = None;
    }

    pub fn account(&self) -> Account {
        match &self.web {
            None => Account::Unavailable,
            Some(_) if self.login_url.is_some() => Account::LoggingIn,
            Some(web) if web.is_logged_in() => Account::LoggedIn,
            Some(_) => Account::LoggedOut,
        }
    }

    pub fn logged_in(&self) -> bool {
        self.account() == Account::LoggedIn
    }

    /// Requests in flight or a login pending: worth looking again soon.
    pub fn busy(&self) -> bool {
        !self.wants.is_empty() || self.login_url.is_some()
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
    /// for what the widget needs (who I am, whether `track` is liked, the
    /// player's shuffle / repeat). Returns toasts to show (the last wins).
    pub fn sync(&mut self, on: bool, now: Instant, track: Option<&str>) -> Vec<String> {
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
        if !self.me_asked {
            self.me_asked = true;
            self.request(Request::Me, Want::Me);
        }
        let track = track.filter(|t| t.starts_with("spotify:track:"));
        if let Some(uri) = track
            && self.liked_asked.as_deref() != Some(uri)
        {
            self.liked_asked = Some(uri.to_owned());
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
                Some(if saved {
                    "logged in to Spotify".into()
                } else {
                    "logged in to Spotify · this session only".into()
                })
            }
            Event::LoginFailed(e) => {
                self.login_url = None;
                Some(e.to_string())
            }
            Event::LoggedOut { expired } => {
                self.forget();
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
            return None;
        }
        match (want, result) {
            (Want::Me, Ok(Reply::User(me))) => self.me = Some(me),
            (Want::Me, _) => self.me_asked = false,
            (Want::Playlists, Ok(Reply::Playlists(lists))) => {
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
            (Want::Tracks { playlist_id }, result) => {
                let open = self
                    .open
                    .as_mut()
                    .filter(|o| o.playlist.id == playlist_id)?;
                open.tracks.loading = false;
                match result {
                    Ok(Reply::Tracks(page)) => {
                        if page.offset as usize == open.tracks.items.len() {
                            open.tracks.items.extend(page.items);
                        }
                        open.tracks.loaded = true;
                        open.has_more = page.has_more;
                        open.playlist.total = page.total;
                    }
                    other => open.tracks.error = Some(message(other)),
                }
            }
            (Want::Liked(uri), Ok(Reply::Contains(found))) => {
                self.liked = Some((uri, found.first().copied().unwrap_or(false)));
            }
            (Want::Liked(_), _) => self.liked_asked = None,
            (Want::Like { .. }, Ok(_)) => {}
            (Want::Like { uri, on }, Err(e)) => {
                if self.liked.as_ref().is_some_and(|(u, _)| *u == uri) {
                    self.liked = Some((uri, !on));
                }
                return Some(e.to_string());
            }
            (Want::Add { name }, Ok(_)) => return Some(format!("added to {name}")),
            (Want::Add { .. }, Err(e)) => return Some(e.to_string()),
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
        let toasts = self
            .library
            .sync(self.music_on(), self.now, track.as_deref());
        if let Some(last) = toasts.into_iter().last() {
            self.toast(last);
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
        self.patch_modes();
    }

    /// Shuffle / repeat from the Web API, where the desktop app's own
    /// controls can't change them.
    pub(super) fn patch_modes(&mut self) {
        let state = self.library.modes();
        let modes = state.map(|p| (p.shuffle, p.repeat != Repeat::Off));
        self.music.web_caps = state.map_or(Capabilities::NONE, |p| Capabilities {
            shuffle: !p.shuffle_blocked,
            repeat: !p.repeat_blocked,
            volume: false,
        });
        if let (Some((shuffle, repeat)), Some(snap)) = (modes, &mut self.music.snapshot) {
            snap.shuffle = shuffle;
            snap.repeat = repeat;
        }
    }

    /// The playing track's URI, if it's a Spotify track.
    fn playing_uri(&self) -> Option<String> {
        let snap = self.music.snapshot.as_ref()?;
        let track = snap.track.as_ref()?;
        track
            .id
            .starts_with("spotify:track:")
            .then(|| track.id.clone())
    }

    /// Whether the playing track is liked (for the heart), once known.
    pub fn liked(&self) -> Option<bool> {
        self.library.liked(&self.playing_uri()?)
    }

    /// A library key with no Client ID (or music off) says why.
    fn library_ready(&mut self) -> bool {
        if self.library.account() == Account::Unavailable {
            self.toast(if self.music_on() {
                "Spotify library needs a Client ID · see docs/spotify.md"
            } else {
                "music is off · a to show it"
            });
            return false;
        }
        true
    }

    /// `i`: log in (browser); while waiting, cancel; logged in, twice to
    /// log out.
    pub(super) fn account_key(&mut self, now: Instant) {
        if !self.library_ready() {
            return;
        }
        let account = self.library.account();
        let lib = &mut self.library;
        let Some(web) = lib.web.as_mut() else {
            return;
        };
        let toast = match account {
            Account::Unavailable => return,
            Account::LoggingIn => {
                web.cancel_login();
                lib.login_url = None;
                "login cancelled".to_owned()
            }
            Account::LoggedIn => {
                let armed = lib
                    .logout_armed
                    .take()
                    .is_some_and(|at| now - at < LOGOUT_WINDOW);
                if armed {
                    // `LoggedOut` toasts when it's done.
                    web.logout();
                    return;
                }
                lib.logout_armed = Some(now);
                "press i again to log out of Spotify".to_owned()
            }
            Account::LoggedOut => match web.login() {
                Ok(url) => {
                    lib.login_url = Some(url);
                    "log in to Spotify in your browser".to_owned()
                }
                Err(e) => e.to_string(),
            },
        };
        self.toast(toast);
    }

    /// `s`: like or unlike the playing track (the heart changes at once).
    pub(super) fn like_key(&mut self) {
        if !self.library_ready() || !self.logged_in_or_say() {
            return;
        }
        let Some(uri) = self.playing_uri() else {
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
        self.toast(if on { "♥ liked" } else { "♡ unliked" });
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
        if !self.library_ready() {
            return;
        }
        if kind == ListKind::AddTo && self.library.logged_in() && self.playing_uri().is_none() {
            self.toast("nothing playing to add");
            return;
        }
        self.refresh_playlists();
        self.last_click = None;
        self.library.find = Find::default();
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
                let at = *lib.editable().get(i)?;
                Some(playlist_row(&lib.playlists.items[at], false))
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
        if self.library.account() != Account::LoggedIn {
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
                self.library.open = Some(OpenPlaylist {
                    playlist: p,
                    tracks: Listing {
                        loading: true,
                        ..Listing::default()
                    },
                    has_more: false,
                });
                let request = Request::PlaylistTracks {
                    playlist_id: id.clone(),
                    offset: 0,
                };
                self.library
                    .request(request, Want::Tracks { playlist_id: id });
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
                let Some(uri) = self.playing_uri() else {
                    self.toast("nothing playing to add");
                    return true;
                };
                let p = &self.library.playlists.items[at];
                let (id, name) = (p.id.clone(), p.name.clone());
                let request = Request::AddToPlaylist {
                    playlist_id: id,
                    uris: vec![uri],
                };
                self.library.request(request, Want::Add { name });
                self.overlay = Overlay::None;
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
    /// (lava-75z.18: on macOS it plays the track in its playlist too; the
    /// other players play the track alone).
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

    /// The desktop app plays it.
    fn play_here(&mut self, playing: &Playing) {
        let now = self.now;
        let played = match &playing.track {
            Some(track) => self.music.play_in_context(track, &playing.context, now),
            None => self.music.play_uri(&playing.context, now),
        };
        match played {
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
        let loaded = open.tracks.items.len();
        let near_end = finding || view.cursor + PREFETCH >= shown;
        if open.has_more && !open.tracks.loading && near_end {
            open.tracks.loading = true;
            let playlist_id = open.playlist.id.clone();
            let request = Request::PlaylistTracks {
                playlist_id: playlist_id.clone(),
                offset: loaded as u32,
            };
            self.library.request(request, Want::Tracks { playlist_id });
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
