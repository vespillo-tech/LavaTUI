//! `--demo` (hidden): a made-up player for screenshots and the README
//! demo, so no real song, cover or lyric ever lands in a committed image.
//!
//! Three invented tracks by invented artists, each with an abstract cover
//! drawn here (handed to the art worker through [`art::stash`], so nothing
//! is downloaded) and invented synced lyrics served by a canned LRCLIB
//! ([`Canned`]: nothing goes to lrclib.net and nothing is cached). The
//! player is a [`FakeSource`]: it plays in real time and every player key
//! works. The Spotify library stays off (no Client ID, no keyring).

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::lyrics::LyricsService;
use crate::lyrics::client::{Http, Lrclib, Reply};
use crate::media::{FakeSource, Snapshot, Status, Track, art};

/// Title (letters and spaces only: it's matched in the lookup URL),
/// artist, album, length in seconds, the cover's two hues, the lyrics.
struct Song {
    title: &'static str,
    artist: &'static str,
    album: &'static str,
    secs: u64,
    hues: (f32, f32),
    words: &'static [&'static str],
}

const SONGS: &[Song] = &[
    Song {
        title: "Slow Rise",
        artist: "The Paraffins",
        album: "Heat Rises",
        secs: 214,
        hues: (12.0, 40.0),
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
    },
    Song {
        title: "Blob Merge",
        artist: "The Paraffins",
        album: "Heat Rises",
        secs: 187,
        hues: (300.0, 330.0),
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
    },
    Song {
        title: "Warm Light Falling",
        artist: "Wax and Wane",
        album: "Lamplight",
        secs: 402,
        hues: (180.0, 150.0),
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
    },
];

/// The demo player: the first song playing, 42 s in.
pub fn source(now: Instant) -> FakeSource {
    let playlist: Vec<Track> = SONGS
        .iter()
        .enumerate()
        .map(|(i, song)| Track {
            id: format!("demo:track:{i}"),
            uri: None,
            name: song.title.into(),
            artist: song.artist.into(),
            album: song.album.into(),
            duration: Duration::from_secs(song.secs),
            artwork_url: art::stash(cover(song.hues)).unwrap_or_default(),
        })
        .collect();
    let snapshot = Snapshot {
        track: Some(Arc::new(playlist[0].clone())),
        position: Duration::from_secs(42),
        volume: 70,
        ..Snapshot::new(Status::Playing, now)
    };
    FakeSource::new(snapshot, playlist)
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
            "instrumental": false,
            "syncedLyrics": lrc(song),
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
/// verses over and over, an empty line (a break) between them.
fn lrc(song: &Song) -> String {
    let mut out = String::new();
    let mut at = 8.0;
    for line in song.words.iter().cycle() {
        if at > song.secs as f64 - 5.0 {
            break;
        }
        let (m, s) = ((at / 60.0) as u64, at % 60.0);
        out.push_str(&format!("[{m:02}:{s:05.2}]{line}\n"));
        at += if line.is_empty() { 6.0 } else { 4.5 };
    }
    out
}

/// An abstract cover, 400 px square, as PNG: a dark gradient in the first
/// hue with three soft glowing discs in both.
fn cover((a, b): (f32, f32)) -> Vec<u8> {
    const N: u32 = 400;
    let discs = [
        (0.32, 0.38, 0.26, a),
        (0.68, 0.62, 0.22, b),
        (0.55, 0.22, 0.12, b),
    ];
    let image = image::RgbImage::from_fn(N, N, |x, y| {
        let (u, v) = (x as f32 / N as f32, y as f32 / N as f32);
        let mut rgb = hsl(a, 0.45, 0.08 + 0.10 * v);
        for &(cx, cy, r, hue) in &discs {
            let d = ((u - cx).powi(2) + (v - cy).powi(2)).sqrt() / r;
            let glow = (1.0 - d).clamp(0.0, 1.0).powf(0.6);
            let disc = hsl(hue, 0.85, 0.35 + 0.30 * glow);
            for (c, d) in rgb.iter_mut().zip(disc) {
                *c += (d - *c) * glow;
            }
        }
        image::Rgb(rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
    });
    let mut png = Vec::new();
    let _ = image.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png);
    png
}

/// HSL (hue in degrees) to RGB in 0..=1.
fn hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::{Lyrics, Track as LyricsTrack};

    #[test]
    fn every_song_has_a_cover_and_synced_lyrics() {
        let client = Lrclib::with_http(Canned, "demo:");
        for song in SONGS {
            let png = cover(song.hues);
            assert!(art::Art::decode(&png, false).is_ok(), "{}", song.title);
            let track = LyricsTrack {
                title: song.title.into(),
                artist: song.artist.into(),
                album: song.album.into(),
                duration: Some(Duration::from_secs(song.secs)),
            };
            let raw = client.fetch(&track).expect("fetch").expect("found");
            let lyrics = Lyrics::from_parts(raw.instrumental, raw.synced.as_deref(), None);
            assert!(matches!(lyrics, Some(Lyrics::Synced(_))), "{}", song.title);
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
    fn the_demo_plays_the_first_song() {
        let now = Instant::now();
        let snapshot = source(now).snapshot_at(now);
        assert_eq!(snapshot.status, Status::Playing);
        let track = snapshot.track.expect("a track");
        assert_eq!(track.name, "Slow Rise");
        assert!(track.artwork_url.starts_with(art::THUMB_SCHEME));
    }
}
