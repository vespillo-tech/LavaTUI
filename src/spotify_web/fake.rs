//! [`FakeWeb`]: an in-memory Spotify account for the app's tests. Answers
//! every request at once from its state (playlists, tracks, liked songs,
//! the player) and records what was asked; clones share the state, so a
//! test keeps one to look at while the model owns another.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use super::{
    Error, Event, LoginStore, Page, PlayerState, Playlist, Reply, Request, RequestId, Track, Uris,
    User, Web,
};

pub struct FakeState {
    pub logged_in: bool,
    /// The saved login is unread (the macOS Keychain): `unlock` reads it.
    pub locked: bool,
    /// What an unlock finds: a saved login or none.
    pub saved: bool,
    /// Unlocks asked for.
    pub unlocks: u32,
    /// An unlock waiting for `release` (macOS still asking).
    unlock_held: bool,
    /// The stores `set_store` was asked to move to.
    pub moves: Vec<LoginStore>,
    /// A login is waiting for the browser (`finish_login` completes it).
    pub login_pending: bool,
    pub me: Option<User>,
    pub playlists: Vec<Playlist>,
    /// Items per playlist id.
    pub tracks: HashMap<String, Vec<Track>>,
    pub liked: BTreeSet<String>,
    /// What `Player` answers; `Err` also refuses the setters.
    pub player: Result<Option<PlayerState>, Error>,
    /// Requests seen, in order.
    pub requests: Vec<Request>,
    /// Hold replies until `release` (to look at loading states).
    pub hold: bool,
    /// The next request fails with this.
    pub fail: Option<Error>,
    /// Every `PlaylistUris` fails with this (the duplicate check).
    pub fail_uris: Option<Error>,
    /// Adds so far: each gives the playlist a new snapshot id.
    adds: u32,
    held: Vec<(RequestId, Request)>,
    events: VecDeque<Event>,
    next_id: RequestId,
}

impl Default for FakeState {
    fn default() -> Self {
        Self {
            logged_in: false,
            locked: false,
            saved: false,
            unlocks: 0,
            unlock_held: false,
            moves: Vec::new(),
            login_pending: false,
            me: None,
            playlists: Vec::new(),
            tracks: HashMap::new(),
            liked: BTreeSet::new(),
            player: Ok(None),
            requests: Vec::new(),
            hold: false,
            fail: None,
            fail_uris: None,
            adds: 0,
            held: Vec::new(),
            events: VecDeque::new(),
            next_id: 0,
        }
    }
}

#[derive(Clone, Default)]
pub struct FakeWeb(pub Arc<Mutex<FakeState>>);

pub const PAGE: usize = 50;

impl FakeWeb {
    /// Logged in as `me`, with `playlists` (and their `tracks`).
    /// Logged in as `me`, with `playlists` and the tracks of those `me`
    /// can read (others answer `Forbidden`, as Spotify does).
    pub fn account(me: User, playlists: Vec<(Playlist, Vec<Track>)>) -> Self {
        let fake = FakeWeb::default();
        {
            let mut s = fake.state();
            s.logged_in = true;
            for (p, tracks) in playlists {
                if p.editable_by(&me) {
                    s.tracks.insert(p.id.clone(), tracks);
                }
                s.playlists.push(p);
            }
            s.me = Some(me);
        }
        fake
    }

    /// The same account, its login saved but not read yet (macOS).
    pub fn locked(self) -> Self {
        {
            let mut s = self.state();
            s.saved = s.logged_in;
            s.logged_in = false;
            s.locked = true;
        }
        self
    }

    pub fn state(&self) -> MutexGuard<'_, FakeState> {
        self.0.lock().unwrap()
    }

    /// The browser came back: logged in.
    pub fn finish_login(&self) {
        let mut s = self.state();
        s.login_pending = false;
        s.logged_in = true;
        s.events.push_back(Event::LoggedIn { saved: true });
    }

    /// The browser came back with an error.
    pub fn fail_login(&self, error: Error) {
        let mut s = self.state();
        s.login_pending = false;
        s.events.push_back(Event::LoginFailed(error));
    }

    /// Answer the held unlock and requests.
    pub fn release(&self) {
        let mut s = self.state();
        s.hold = false;
        if std::mem::take(&mut s.unlock_held) {
            s.logged_in = s.saved;
            let logged_in = s.logged_in;
            s.events.push_back(Event::Unlocked { logged_in });
        }
        let held = std::mem::take(&mut s.held);
        for (id, request) in held {
            let result = answer(&mut s, request);
            s.events.push_back(Event::Reply { id, result });
        }
    }
}

fn answer(s: &mut FakeState, request: Request) -> Result<Reply, Error> {
    if !s.logged_in {
        return Err(Error::NotLoggedIn);
    }
    if let Some(e) = s.fail.take() {
        return Err(e);
    }
    Ok(match request {
        Request::Me => Reply::User(s.me.clone().ok_or(Error::NotLoggedIn)?),
        Request::MyPlaylists => Reply::Playlists(s.playlists.clone()),
        Request::PlaylistTracks {
            playlist_id,
            offset,
        } => {
            let all = s
                .tracks
                .get(&playlist_id)
                .ok_or_else(|| Error::Forbidden("not yours".into()))?;
            let from = (offset as usize).min(all.len());
            let to = (from + PAGE).min(all.len());
            Reply::Tracks(Page {
                items: all[from..to].to_vec(),
                offset,
                total: all.len() as u32,
                has_more: to < all.len(),
            })
        }
        Request::PlaylistUris {
            playlist_id,
            offset,
        } => {
            if let Some(e) = &s.fail_uris {
                return Err(e.clone());
            }
            let all = s
                .tracks
                .get(&playlist_id)
                .ok_or_else(|| Error::Forbidden("not yours".into()))?;
            let from = (offset as usize).min(all.len());
            let to = (from + PAGE).min(all.len());
            Reply::Uris(Uris {
                uris: all[from..to].iter().map(|t| t.uri.clone()).collect(),
                next: (to < all.len()).then_some(to as u32),
                total: all.len() as u32,
            })
        }
        Request::AddToPlaylist { playlist_id, uris } => {
            if !s.tracks.contains_key(&playlist_id) {
                return Err(Error::Forbidden("not yours".into()));
            }
            s.adds += 1;
            let snapshot = format!("snap{}", s.adds);
            let n = uris.len() as u32;
            if let Some(p) = s.playlists.iter_mut().find(|p| p.id == playlist_id) {
                p.snapshot_id.clone_from(&snapshot);
                p.total += n;
            }
            let list = s.tracks.entry(playlist_id).or_default();
            for uri in uris {
                list.push(Track {
                    id: uri.rsplit(':').next().map(str::to_owned),
                    uri,
                    name: "added".into(),
                    artists: Vec::new(),
                    album: String::new(),
                    duration_ms: 0,
                    is_local: false,
                    image_url: None,
                });
            }
            Reply::Snapshot(snapshot)
        }
        Request::LibraryContains { uris } => {
            Reply::Contains(uris.iter().map(|u| s.liked.contains(u)).collect())
        }
        Request::Like { uris } => {
            s.liked.extend(uris);
            Reply::Done
        }
        Request::Unlike { uris } => {
            for u in &uris {
                s.liked.remove(u);
            }
            Reply::Done
        }
        Request::Player => Reply::Player(s.player.clone()?),
        Request::SetShuffle(on) => {
            if let Some(p) = s.player.clone()?.as_mut() {
                p.shuffle = on;
                s.player = Ok(Some(p.clone()));
            }
            Reply::Done
        }
        Request::SetRepeat(repeat) => {
            if let Some(p) = s.player.clone()?.as_mut() {
                p.repeat = repeat;
                s.player = Ok(Some(p.clone()));
            }
            Reply::Done
        }
        Request::Play { .. } => {
            s.player.clone()?;
            Reply::Done
        }
        Request::CreatePlaylist { .. }
        | Request::SearchTracks { .. }
        | Request::ArtistTracks { .. } => return Err(Error::NotFound("fake".into())),
    })
}

impl Web for FakeWeb {
    fn is_logged_in(&self) -> bool {
        self.state().logged_in
    }

    fn locked(&self) -> bool {
        self.state().locked
    }

    fn unlock(&mut self) {
        let mut s = self.state();
        if !std::mem::replace(&mut s.locked, false) {
            return;
        }
        s.unlocks += 1;
        if s.hold {
            s.unlock_held = true;
            return;
        }
        s.logged_in = s.saved;
        let logged_in = s.logged_in;
        s.events.push_back(Event::Unlocked { logged_in });
    }

    fn set_store(&mut self, choice: LoginStore) {
        let mut s = self.state();
        if std::mem::replace(&mut s.locked, false) {
            s.logged_in = s.saved;
        }
        s.moves.push(choice);
        let logged_in = s.logged_in;
        s.events.push_back(Event::Moved {
            logged_in,
            saved: true,
        });
    }

    fn login(&mut self) -> Result<String, Error> {
        self.state().login_pending = true;
        Ok("https://accounts.spotify.com/authorize?fake".into())
    }

    fn cancel_login(&mut self) {
        self.state().login_pending = false;
    }

    fn logout(&mut self) {
        let mut s = self.state();
        s.logged_in = false;
        s.events.push_back(Event::LoggedOut { expired: false });
    }

    fn request(&mut self, request: Request) -> RequestId {
        let mut s = self.state();
        s.next_id += 1;
        let id = s.next_id;
        s.requests.push(request.clone());
        if s.hold {
            s.held.push((id, request));
        } else {
            let result = answer(&mut s, request);
            s.events.push_back(Event::Reply { id, result });
        }
        id
    }

    fn poll(&mut self) -> Option<Event> {
        self.state().events.pop_front()
    }
}

/// A playlist for tests.
pub fn playlist(id: &str, name: &str, owner: &str, collaborative: bool, total: u32) -> Playlist {
    Playlist {
        id: id.into(),
        uri: format!("spotify:playlist:{id}"),
        name: name.into(),
        owner_id: owner.into(),
        owner_name: None,
        collaborative,
        public: Some(false),
        snapshot_id: "s".into(),
        total,
        image_url: None,
    }
}

/// A track for tests.
pub fn track(id: &str, name: &str, artist: &str) -> Track {
    Track {
        id: Some(id.into()),
        uri: format!("spotify:track:{id}"),
        name: name.into(),
        artists: vec![artist.into()],
        album: "Lamplight".into(),
        duration_ms: 200_000,
        is_local: false,
        image_url: None,
    }
}

/// The account the app's tests use: `me` owns "Lamplight Mix" (60
/// tracks, so it pages) and "lavatui test" (empty), collaborates on
/// "Shared Jams", and follows "Discover Weekly" (not readable).
pub fn demo() -> FakeWeb {
    let me = User {
        id: "me".into(),
        display_name: Some("Me".into()),
        uri: "spotify:user:me".into(),
    };
    let mix: Vec<Track> = (0..60)
        .map(|i| track(&format!("t{i}"), &format!("Slow Rise {i}"), "Wax & Wane"))
        .collect();
    FakeWeb::account(
        me,
        vec![
            (playlist("mix", "Lamplight Mix", "me", false, 60), mix),
            (playlist("test", "lavatui test", "me", false, 0), Vec::new()),
            (
                playlist("dw", "Discover Weekly", "spotify", false, 30),
                Vec::new(),
            ),
            (
                playlist("jams", "Shared Jams", "friend", true, 1),
                vec![track("j1", "Convection", "Wax & Wane")],
            ),
        ],
    )
}
