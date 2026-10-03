//! `--demo` (hidden): a made-up player for screenshots and the README
//! demo, so no real song, cover or lyric ever lands in a committed image.
//!
//! Five invented tracks by invented artists, with an original cover per
//! album embedded here (Heat Rises: `suspended-melt.jpg`, Lamplight:
//! `waxwood-hymns.jpg`; handed to the art worker through [`art::stash`], so nothing
//! is downloaded) and, for three of them, invented synced lyrics served by
//! a canned LRCLIB ([`Canned`]: nothing goes to lrclib.net and nothing is
//! cached). The last two show the lyrics widget's fallbacks: LRCLIB marks
//! one instrumental and has nothing for the other. The
//! player is a [`FakeSource`]: it plays in real time and every player key
//! works.
//!
//! The Spotify library is a made-up account ([`account`], a [`FakeWeb`]:
//! no Client ID, no network, no keyring, its login never saved): a few
//! playlists of the demo songs and some more invented ones ([`MORE`], no
//! lyrics), liked songs, add-to (the first playlist has the first song
//! already, so adding it asks first). Playing from the browser plays in the
//! [`FakeSource`], which knows the playlists.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::lyrics::LyricsService;
use crate::lyrics::client::{Http, Lrclib, Reply};
use crate::media::{FakeSource, Snapshot, Status, Track, art};
use crate::spotify_web::fake::{FakeWeb, playlist};
use crate::spotify_web::{self as web, User};

/// What the canned LRCLIB knows about a song.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Synced lyrics (`Song::words`).
    Sung,
    /// A record flagged instrumental.
    Instrumental,
    /// No record at all.
    Missing,
}

/// Title (letters and spaces only: it's matched in the lookup URL),
/// artist, album, length in seconds, the cover's encoded bytes, the lyrics,
/// and whether they time each word (enhanced LRC) or only lines.
struct Song {
    title: &'static str,
    artist: &'static str,
    album: &'static str,
    secs: u64,
    cover: &'static [u8],
    kind: Kind,
    words: &'static [&'static str],
    word_times: bool,
}

const SONGS: &[Song] = &[
    Song {
        title: "Slow Rise",
        artist: "The Paraffins",
        album: "Heat Rises",
        secs: 214,
        cover: include_bytes!("../assets/demo/suspended-melt.jpg"),
        kind: Kind::Sung,
        words: &[
            "Down at the bottom where the warm light grows",
            "A little wax is waking, and it slowly goes",
            "Up through the amber, taking its time",
            "Nothing in a hurry, nothing on the line",
            "",
            "Slow rise, slow rise",
            "Floating like a thought behind your eyes",
            "Slow rise, slow rise",
            "Cooling at the top and coming down to try again",
            "",
            "Round and round the evening turns",
            "Every little bubble learns",
            "What goes up will settle in",
            "And then it starts to rise again",
        ],
        word_times: false,
    },
    Song {
        title: "Blob Merge",
        artist: "The Paraffins",
        album: "Heat Rises",
        secs: 187,
        cover: include_bytes!("../assets/demo/suspended-melt.jpg"),
        kind: Kind::Sung,
        words: &[
            "Two slow shapes in the purple glow",
            "Drifting closer, moving slow",
            "One more inch and then they touch",
            "Never needed very much",
            "",
            "Merge, merge, now we are one",
            "Split again when the heat is done",
            "Merge, merge, a softer shape",
            "Nowhere else we would escape",
        ],
        word_times: true,
    },
    Song {
        title: "Warm Light Falling",
        artist: "Wax and Wane",
        album: "Lamplight",
        secs: 402,
        cover: include_bytes!("../assets/demo/waxwood-hymns.jpg"),
        kind: Kind::Sung,
        words: &[
            "Late at night the room is blue",
            "And the lamp is humming through",
            "Every color that it knows",
            "As the quiet water flows",
            "",
            "Warm light falling, warm light rising",
            "Nothing here is a surprise",
            "Warm light falling, warm light rising",
            "Watch it with your sleepy eyes",
        ],
        word_times: false,
    },
    Song {
        title: "Long Cooldown",
        artist: "The Paraffins",
        album: "Heat Rises",
        secs: 251,
        cover: include_bytes!("../assets/demo/suspended-melt.jpg"),
        kind: Kind::Instrumental,
        words: &[],
        word_times: false,
    },
    Song {
        title: "Bare Wax",
        artist: "Wax and Wane",
        album: "Lamplight",
        secs: 196,
        cover: include_bytes!("../assets/demo/waxwood-hymns.jpg"),
        kind: Kind::Missing,
        words: &[],
        word_times: false,
    },
];

/// Songs only the made-up account has, after [`SONGS`] (their album's
/// cover, else Heat Rises'; no lyrics): title, artist, album, length in seconds.
const MORE: &[(&str, &str, &str, u64)] = &[
    ("Lamp Left On", "Wax and Wane", "Lamplight", 233),
    ("Amber Drift", "The Paraffins", "Heat Rises", 205),
    ("Paraffin Dreams", "Glass Bottom", "Low Heat", 251),
    ("Convection", "Wax and Wane", "Lamplight", 198),
    ("Cooling at the Top", "Molten Hour", "Bubble Theory", 222),
    ("Ninety Minutes to Warm", "Glass Bottom", "Low Heat", 176),
    ("Little Blob, Big Room", "Molten Hour", "Bubble Theory", 164),
    ("Tidal Wax", "Glass Bottom", "Low Heat", 239),
];

/// The made-up account's playlists: name, owner, collaborative, songs by
/// title. The first has the first song (the one playing at the start).
const PLAYLISTS: &[(&str, &str, bool, &[&str])] = &[
    (
        "Late Night Lava",
        ME,
        false,
        &[
            "Slow Rise",
            "Warm Light Falling",
            "Amber Drift",
            "Paraffin Dreams",
            "Cooling at the Top",
        ],
    ),
    (
        "Slow Sunday",
        ME,
        false,
        &[
            "Lamp Left On",
            "Convection",
            "Blob Merge",
            "Tidal Wax",
            "Long Cooldown",
        ],
    ),
    (
        "Pomodoro Focus",
        ME,
        false,
        &[
            "Convection",
            "Ninety Minutes to Warm",
            "Little Blob, Big Room",
            "Tidal Wax",
            "Lamp Left On",
            "Bare Wax",
        ],
    ),
    (
        "Rising Heat",
        "glass.bottom.fan",
        true,
        &["Little Blob, Big Room", "Blob Merge", "Paraffin Dreams"],
    ),
];

/// Liked at the start, by title.
const LIKED: &[&str] = &["Slow Rise", "Amber Drift"];

/// The made-up account's user id.
const ME: &str = "wax.collector";

/// One song as both the player and the Web API see it.
struct Tune {
    title: &'static str,
    artist: &'static str,
    album: &'static str,
    secs: u64,
}

/// Every song: [`SONGS`], then [`MORE`].
fn tunes() -> impl Iterator<Item = Tune> {
    let songs = SONGS.iter().map(|s| Tune {
        title: s.title,
        artist: s.artist,
        album: s.album,
        secs: s.secs,
    });
    let more = MORE.iter().map(|&(title, artist, album, secs)| Tune {
        title,
        artist,
        album,
        secs,
    });
    songs.chain(more)
}

/// A made-up Spotify id: 22 letters and digits, like a real one.
fn id(kind: &str, i: usize) -> String {
    format!("lavatuidemo{kind}{i:0w$}", w = 11 - kind.len())
}

/// The `n`th song's (in [`tunes`] order) track URI.
fn uri(n: usize) -> String {
    format!("spotify:track:{}", id("t", n))
}

/// Where `title` is in [`tunes`].
fn index(title: &str) -> usize {
    tunes()
        .position(|t| t.title == title)
        .unwrap_or_else(|| panic!("no demo song {title:?}"))
}

/// Every song as the player has it: its album's cover where a demo song
/// is on that album, else the first song's.
fn player_tracks() -> Vec<Track> {
    let cover = |album: &str| {
        let song = SONGS.iter().find(|s| s.album == album).unwrap_or(&SONGS[0]);
        art::stash(song.cover.to_vec()).unwrap_or_default()
    };
    tunes()
        .enumerate()
        .map(|(n, tune)| Track {
            id: uri(n),
            uri: Some(uri(n)),
            name: tune.title.into(),
            artist: tune.artist.into(),
            album: tune.album.into(),
            duration: Duration::from_secs(tune.secs),
            artwork_url: cover(tune.album),
        })
        .collect()
}

/// The playlists' URIs and songs.
fn playlists<T: Clone>(tracks: &[T]) -> Vec<(web::Playlist, Vec<T>)> {
    PLAYLISTS
        .iter()
        .enumerate()
        .map(|(i, &(name, owner, collaborative, titles))| {
            let list = playlist(&id("p", i), name, owner, collaborative, titles.len() as u32);
            let songs = titles.iter().map(|t| tracks[index(t)].clone()).collect();
            (list, songs)
        })
        .collect()
}

/// The demo player: the first song playing, 42 s in. Next / previous walk
/// [`SONGS`]; it can play the made-up account's playlists.
pub fn source(now: Instant) -> FakeSource {
    let tracks = player_tracks();
    let contexts = playlists(&tracks)
        .into_iter()
        .map(|(p, songs)| (p.uri, songs))
        .collect();
    let snapshot = Snapshot {
        track: Some(Arc::new(tracks[0].clone())),
        position: Duration::from_secs(42),
        volume: 70,
        ..Snapshot::new(Status::Playing, now)
    };
    FakeSource::new(snapshot, tracks[..SONGS.len()].to_vec()).with_contexts(contexts)
}

/// The made-up Spotify account, logged in. Logging out and in again
/// needs no browser.
pub fn account() -> FakeWeb {
    let tracks: Vec<web::Track> = tunes()
        .enumerate()
        .map(|(n, tune)| web::Track {
            id: Some(id("t", n)),
            uri: uri(n),
            name: tune.title.into(),
            artists: vec![tune.artist.into()],
            album: tune.album.into(),
            duration_ms: (tune.secs * 1000) as u32,
            is_local: false,
            image_url: None,
        })
        .collect();
    let me = User {
        id: ME.into(),
        display_name: Some("Wax Collector".into()),
        uri: format!("spotify:user:{ME}"),
    };
    let fake = FakeWeb::account(me, playlists(&tracks));
    {
        let mut s = fake.state();
        s.instant_login = true;
        s.liked = LIKED.iter().map(|t| uri(index(t))).collect();
    }
    fake
}

/// The lyrics service, answered by [`Canned`] (no cache, no retries).
pub fn lyrics() -> Option<LyricsService> {
    LyricsService::spawn(Lrclib::with_http(Canned, "demo:"), None, Vec::new()).ok()
}

/// An LRCLIB that knows only the demo songs.
struct Canned;

impl Http for Canned {
    fn get(&self, url: &str) -> Result<Reply, String> {
        let song = SONGS
            .iter()
            .filter(|s| s.kind != Kind::Missing)
            .find(|s| url.contains(&format!("track_name={}", s.title.replace(' ', "%20"))));
        let Some(song) = song else {
            let (status, body) = if url.contains("/api/search") {
                (200, "[]")
            } else {
                (404, "{}")
            };
            return Ok(Reply {
                status,
                body: body.into(),
            });
        };
        let body = serde_json::json!({
            "trackName": song.title,
            "artistName": song.artist,
            "albumName": song.album,
            "duration": song.secs,
            "instrumental": song.kind == Kind::Instrumental,
            "syncedLyrics": (song.kind == Kind::Sung).then(|| lrc(song)),
        });
        let body = if url.contains("/api/search") {
            serde_json::Value::Array(vec![body])
        } else {
            body
        };
        Ok(Reply {
            status: 200,
            body: body.to_string(),
        })
    }
}

/// The song's words as LRC: a line every 4.5 s after an 8 s intro, the
/// verses over and over, an empty line (a break) between them. With
/// `word_times`, each word tagged too (a lazy, even beat, the last word
/// held), sung over the first 3.4 s of its line.
fn lrc(song: &Song) -> String {
    let stamp = |at: f64| {
        let cs = (at * 100.0).round() as u64;
        format!("{:02}:{:02}.{:02}", cs / 6000, cs / 100 % 60, cs % 100)
    };
    let mut out = String::new();
    let mut at = 8.0;
    for line in song.words.iter().cycle() {
        if at > song.secs as f64 - 5.0 {
            break;
        }
        out.push_str(&format!("[{}]", stamp(at)));
        if song.word_times && !line.is_empty() {
            let words: Vec<&str> = line.split(' ').collect();
            let beat = 2.6 / words.len() as f64;
            for (i, word) in words.iter().enumerate() {
                out.push_str(&format!("<{}>{word} ", stamp(at + i as f64 * beat)));
            }
            out.push_str(&format!("<{}>", stamp(at + 3.4)));
        } else {
            out.push_str(line);
        }
        out.push('\n');
        at += if line.is_empty() { 6.0 } else { 4.5 };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::{Lyrics, Track as LyricsTrack};

    #[test]
    fn every_song_has_a_cover_and_its_kind_of_lyrics() {
        let client = Lrclib::with_http(Canned, "demo:");
        for song in SONGS {
            assert!(
                art::Art::decode(song.cover, false).is_ok(),
                "{}",
                song.title
            );
            let pixels = art::Art::decode(song.cover, true).expect("full-picture cover");
            assert!(pixels.hires.is_some(), "{}", song.title);
            let track = LyricsTrack {
                title: song.title.into(),
                artist: song.artist.into(),
                album: song.album.into(),
                duration: Some(Duration::from_secs(song.secs)),
            };
            let raw = client.fetch(&track).expect("fetch");
            let lyrics = raw.and_then(|r| r.lyrics());
            match song.kind {
                Kind::Sung => {
                    let Some(Lyrics::Synced(synced)) = lyrics else {
                        panic!("{}", song.title)
                    };
                    // Both kinds of timing on show: words from the file,
                    // estimated.
                    let sung = synced.lines.iter().filter(|l| !l.is_gap());
                    assert!(
                        sung.clone().all(|l| l.exact == song.word_times),
                        "{}",
                        song.title
                    );
                    assert!(
                        sung.clone()
                            .all(|l| l.words.len() == l.text.split(' ').count())
                    );
                }
                Kind::Instrumental => assert_eq!(lyrics, Some(Lyrics::Instrumental)),
                Kind::Missing => assert_eq!(lyrics, None, "{}", song.title),
            }
        }
        let other = LyricsTrack {
            title: "Something Else".into(),
            artist: String::new(),
            album: String::new(),
            duration: None,
        };
        assert_eq!(client.fetch(&other), Ok(None));
    }

    #[test]
    fn each_album_has_its_own_cover() {
        let tracks = player_tracks();
        let cover = |name: &str| {
            let track = tracks.iter().find(|t| t.name == name).expect(name);
            track.artwork_url.clone()
        };
        assert_eq!(cover("Slow Rise"), cover("Blob Merge"));
        assert_eq!(cover("Warm Light Falling"), cover("Bare Wax"));
        assert_eq!(cover("Warm Light Falling"), cover("Convection"));
        assert_ne!(cover("Slow Rise"), cover("Warm Light Falling"));
        assert_eq!(cover("Slow Rise"), cover("Tidal Wax"), "other albums");
    }

    #[test]
    fn the_demo_plays_the_first_song() {
        let now = Instant::now();
        let snapshot = source(now).snapshot_at(now);
        assert_eq!(snapshot.status, Status::Playing);
        let track = snapshot.track.expect("a track");
        assert_eq!(track.name, "Slow Rise");
        assert!(track.artwork_url.starts_with(art::THUMB_SCHEME));
    }
}
