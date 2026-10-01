//! The data the UI gets: small, flat, owned structs, decoded from Spotify's
//! JSON (only the fields we use; everything else is ignored, so removed or
//! added fields don't break us).

use serde::Deserialize;

/// The logged-in user (`GET /me`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct User {
    pub id: String,
    pub display_name: Option<String>,
    pub uri: String,
}

impl User {
    /// What to call them: display name, else the user id.
    pub fn name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub owner_id: String,
    pub owner_name: Option<String>,
    pub collaborative: bool,
    pub public: Option<bool>,
    pub snapshot_id: String,
    /// Number of items (tracks + episodes).
    pub total: u32,
    /// Largest cover image, if any.
    pub image_url: Option<String>,
}

impl Playlist {
    /// Whether `user` can add to it (and read its items: since Feb 2026 a
    /// development-mode app only gets the items of playlists the user owns
    /// or collaborates on).
    pub fn editable_by(&self, user: &User) -> bool {
        self.owner_id == user.id || self.collaborative
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// `None` for local files.
    pub id: Option<String>,
    /// `spotify:track:…` / `spotify:episode:…` (or `spotify:local:…`).
    pub uri: String,
    pub name: String,
    pub artists: Vec<String>,
    /// Album name (a show's name, for an episode).
    pub album: String,
    pub duration_ms: u32,
    pub is_local: bool,
    pub image_url: Option<String>,
}

impl Track {
    /// Artists joined for display: `A, B`.
    pub fn artist_line(&self) -> String {
        self.artists.join(", ")
    }

    /// True if it can be liked or added to a playlist (not a local file).
    pub fn is_spotify(&self) -> bool {
        !self.is_local && self.id.is_some()
    }
}

/// One page of a paged collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub offset: u32,
    pub total: u32,
    /// Another page follows.
    pub has_more: bool,
}

impl<T> Page<T> {
    /// The offset to ask for next.
    pub fn next_offset(&self) -> u32 {
        self.offset + self.items.len() as u32
    }
}

// ---- raw JSON shapes ------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(super) struct RawPage<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<Option<T>>,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub total: u32,
    pub next: Option<String>,
}

impl<T> RawPage<T> {
    /// Converts the non-null items that `f` accepts.
    pub fn map<U>(self, f: impl FnMut(T) -> Option<U>) -> Page<U> {
        Page {
            items: self.items.into_iter().flatten().filter_map(f).collect(),
            offset: self.offset,
            total: self.total,
            has_more: self.next.is_some(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Image {
    url: String,
    width: Option<u32>,
}

fn largest(images: &[Image]) -> Option<String> {
    images
        .iter()
        .max_by_key(|i| i.width.unwrap_or(0))
        .map(|i| i.url.clone())
}

#[derive(Debug, Deserialize)]
struct Owner {
    id: String,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Count {
    #[serde(default)]
    total: u32,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawPlaylist {
    id: String,
    uri: String,
    #[serde(default)]
    name: String,
    owner: Owner,
    #[serde(default)]
    collaborative: bool,
    public: Option<bool>,
    #[serde(default)]
    snapshot_id: String,
    /// `items` since Feb 2026; `tracks` is the deprecated name.
    items: Option<Count>,
    tracks: Option<Count>,
    #[serde(default)]
    images: Option<Vec<Image>>,
}

impl From<RawPlaylist> for Playlist {
    fn from(p: RawPlaylist) -> Self {
        Playlist {
            total: p.items.or(p.tracks).map_or(0, |c| c.total),
            image_url: largest(p.images.as_deref().unwrap_or_default()),
            id: p.id,
            uri: p.uri,
            name: p.name,
            owner_id: p.owner.id,
            owner_name: p.owner.display_name,
            collaborative: p.collaborative,
            public: p.public,
            snapshot_id: p.snapshot_id,
        }
    }
}

#[derive(Debug, Deserialize)]
struct Named {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawAlbum {
    #[serde(default)]
    name: String,
    #[serde(default)]
    images: Vec<Image>,
}

/// A track or an episode (`type` tells), as search and playlist items
/// return them.
#[derive(Debug, Deserialize)]
pub(super) struct RawTrack {
    id: Option<String>,
    uri: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    artists: Vec<Named>,
    album: Option<RawAlbum>,
    /// Episodes: the show.
    show: Option<RawAlbum>,
    /// Episodes carry their own images.
    #[serde(default)]
    images: Vec<Image>,
    #[serde(default)]
    duration_ms: u32,
    #[serde(default)]
    is_local: bool,
}

impl RawTrack {
    /// `None` without a URI (nothing we could act on).
    pub fn into_track(self) -> Option<Track> {
        let uri = self.uri?;
        let (album, mut image_url) = match self.album.or(self.show) {
            Some(a) => (a.name, largest(&a.images)),
            None => (String::new(), None),
        };
        if image_url.is_none() {
            image_url = largest(&self.images);
        }
        Some(Track {
            id: self.id,
            uri,
            name: self.name,
            artists: self.artists.into_iter().map(|a| a.name).collect(),
            album,
            duration_ms: self.duration_ms,
            is_local: self.is_local,
            image_url,
        })
    }
}

/// A playlist entry: the track under `item` (since Feb 2026) or the
/// deprecated `track`.
#[derive(Debug, Deserialize)]
pub(super) struct RawPlaylistItem {
    item: Option<RawTrack>,
    track: Option<RawTrack>,
    #[serde(default)]
    is_local: bool,
}

impl RawPlaylistItem {
    pub fn into_track(self) -> Option<Track> {
        let mut track = self.item.or(self.track)?.into_track()?;
        track.is_local |= self.is_local;
        Some(track)
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct RawSearch {
    pub tracks: Option<RawPage<RawTrack>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawSnapshot {
    pub snapshot_id: String,
}

/// Repeat as the Web API has it (`off`, `context`, `track`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    /// The playlist or album.
    Context,
    Track,
}

impl Repeat {
    /// The `state` query value.
    pub fn as_str(self) -> &'static str {
        match self {
            Repeat::Off => "off",
            Repeat::Context => "context",
            Repeat::Track => "track",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "context" => Repeat::Context,
            "track" => Repeat::Track,
            _ => Repeat::Off,
        }
    }
}

/// What the user's active Spotify device is doing (`GET /me/player`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerState {
    pub shuffle: bool,
    pub repeat: Repeat,
    pub is_playing: bool,
    /// The device's name ("MacBook Pro"), if it says.
    pub device: Option<String>,
    /// URI of what's playing, if anything.
    pub item_uri: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RawPlayer {
    #[serde(default)]
    shuffle_state: bool,
    #[serde(default)]
    repeat_state: String,
    #[serde(default)]
    is_playing: bool,
    device: Option<Named>,
    item: Option<RawUri>,
}

#[derive(Debug, Deserialize)]
struct RawUri {
    uri: Option<String>,
}

impl From<RawPlayer> for PlayerState {
    fn from(p: RawPlayer) -> Self {
        PlayerState {
            shuffle: p.shuffle_state,
            repeat: Repeat::parse(&p.repeat_state),
            is_playing: p.is_playing,
            device: p.device.map(|d| d.name).filter(|n| !n.is_empty()),
            item_uri: p.item.and_then(|i| i.uri),
        }
    }
}
