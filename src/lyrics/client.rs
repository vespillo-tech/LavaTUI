//! The LRCLIB client (<https://lrclib.net/docs>; free, no key).
//!
//! 1. `GET /api/get?track_name&artist_name&album_name&duration`: the exact
//!    signature; LRCLIB matches duration within ±2 s. `404` means no match.
//! 2. On a miss (or without a duration, which `/api/get` needs):
//!    `GET /api/search?track_name&artist_name`, and pick the best result
//!    (close duration, synced lyrics, matching names).
//!
//! Records are JSON: `trackName, artistName, albumName, duration (s, f64),
//! instrumental, plainLyrics?, syncedLyrics?` (plus fields read past, e.g.
//! `hasWordSync`, `lyricsfile`). `/api/get` can answer `503` while it looks
//! a track up upstream; that and every other failure is [`FetchError`],
//! which callers retry and never cache.

use std::time::Duration;

use serde::Deserialize;

use super::{RawLyrics, Track};

pub const BASE_URL: &str = "https://lrclib.net";
/// LRCLIB asks clients to identify themselves: `lavatui/<version>`.
pub const USER_AGENT: &str = concat!("lavatui/", env!("CARGO_PKG_VERSION"));
const TIMEOUT: Duration = Duration::from_secs(10);
/// Search results further than this from the track's duration are another
/// version (live, remix, radio edit) whose timings won't line up.
const SEARCH_TOLERANCE: f64 = 3.0;

/// A response, whatever its status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

/// The HTTP layer: one GET. `Err` is a transport failure (no reply).
pub trait Http: Send {
    fn get(&self, url: &str) -> Result<Reply, String>;
}

/// Why a lookup reached no answer. Never cached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// No reply: offline, DNS, TLS, timeout.
    Network(String),
    /// An unexpected status (`429`, `5xx`, …).
    Status(u16),
    /// A reply that isn't the JSON we expect.
    Malformed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(e) => write!(f, "offline ({e})"),
            Self::Status(code) => write!(f, "lrclib.net answered {code}"),
            Self::Malformed(e) => write!(f, "unexpected reply from lrclib.net ({e})"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl Record {
    fn has_synced(&self) -> bool {
        self.synced_lyrics
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
    }

    fn has_any(&self) -> bool {
        self.instrumental
            || self.has_synced()
            || self
                .plain_lyrics
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
    }

    fn into_raw(self) -> RawLyrics {
        RawLyrics {
            instrumental: self.instrumental,
            synced: self.synced_lyrics,
            plain: self.plain_lyrics,
        }
    }
}

pub struct Lrclib<H> {
    http: H,
    base: String,
}

impl Lrclib<Ureq> {
    /// The real service over HTTPS.
    pub fn new() -> Self {
        Self::with_http(Ureq::new(), BASE_URL)
    }
}

impl<H: Http> Lrclib<H> {
    pub fn with_http(http: H, base: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    /// The lyrics for `track`: `Ok(None)` when LRCLIB has none.
    pub fn fetch(&self, track: &Track) -> Result<Option<RawLyrics>, FetchError> {
        if let Some(duration) = track.duration {
            let mut query = vec![
                ("track_name", track.title.as_str()),
                ("artist_name", track.artist.as_str()),
            ];
            if !track.album.trim().is_empty() {
                query.push(("album_name", track.album.as_str()));
            }
            let secs = duration.as_secs_f64().round().to_string();
            query.push(("duration", &secs));
            let reply = self.get("/api/get", &query)?;
            match reply.status {
                200 => {
                    let record: Record = parse(&reply.body)?;
                    if record.has_any() {
                        return Ok(Some(record.into_raw()));
                    }
                }
                404 => {}
                status => return Err(FetchError::Status(status)),
            }
        }
        self.search(track)
    }

    fn search(&self, track: &Track) -> Result<Option<RawLyrics>, FetchError> {
        let query = [
            ("track_name", track.title.as_str()),
            ("artist_name", track.artist.as_str()),
        ];
        let reply = self.get("/api/search", &query)?;
        if reply.status != 200 {
            return Err(FetchError::Status(reply.status));
        }
        let records: Vec<Record> = parse(&reply.body)?;
        Ok(best(records, track).map(Record::into_raw))
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Reply, FetchError> {
        let query: Vec<String> = query
            .iter()
            .map(|(k, v)| format!("{k}={}", encode(v)))
            .collect();
        let url = format!("{}{path}?{}", self.base, query.join("&"));
        self.http.get(&url).map_err(FetchError::Network)
    }
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, FetchError> {
    serde_json::from_str(body).map_err(|e| FetchError::Malformed(e.to_string()))
}

/// The search result most likely to be this recording, if any is close
/// enough: within [`SEARCH_TOLERANCE`] of the duration (when known), then
/// synced over plain, matching names, nearest duration.
fn best(records: Vec<Record>, track: &Track) -> Option<Record> {
    let want = track.duration.map(|d| d.as_secs_f64());
    let off = |r: &Record| match (want, r.duration) {
        (Some(want), Some(got)) => (want - got).abs(),
        _ => 0.0,
    };
    let title = norm(&track.title);
    let artist = norm(&track.artist);
    records
        .into_iter()
        .filter(|r| r.has_any() && off(r) <= SEARCH_TOLERANCE)
        .min_by(|a, b| {
            let rank = |r: &Record| {
                (
                    !r.has_synced(),
                    norm(&r.track_name) != title,
                    norm(&r.artist_name) != artist,
                )
            };
            rank(a).cmp(&rank(b)).then(off(a).total_cmp(&off(b)))
        })
}

fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Percent-encodes a query value (RFC 3986 unreserved characters kept).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// [`Http`] over `ureq` (rustls, so no system TLS needed on any platform).
pub struct Ureq(ureq::Agent);

impl Ureq {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .build();
        Self(config.into())
    }
}

impl Http for Ureq {
    fn get(&self, url: &str) -> Result<Reply, String> {
        let mut response = self.0.get(url).call().map_err(|e| e.to_string())?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_string()
            .map_err(|e| e.to_string())?;
        Ok(Reply { status, body })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::lyrics::cache::tests::track;

    /// Replies from a script, in order, recording each URL asked for.
    #[derive(Clone, Default)]
    pub(crate) struct Mock {
        pub replies: Arc<Mutex<VecDeque<Result<Reply, String>>>>,
        pub urls: Arc<Mutex<Vec<String>>>,
    }

    impl Mock {
        pub(crate) fn new(replies: impl IntoIterator<Item = Result<Reply, String>>) -> Self {
            Self {
                replies: Arc::new(Mutex::new(replies.into_iter().collect())),
                ..Self::default()
            }
        }

        pub(crate) fn urls(&self) -> Vec<String> {
            self.urls.lock().unwrap().clone()
        }
    }

    impl Http for Mock {
        fn get(&self, url: &str) -> Result<Reply, String> {
            self.urls.lock().unwrap().push(url.to_string());
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err("mock: no more replies".into()))
        }
    }

    pub(crate) fn ok(body: &str) -> Result<Reply, String> {
        Ok(Reply {
            status: 200,
            body: body.into(),
        })
    }

    pub(crate) fn status(status: u16) -> Result<Reply, String> {
        Ok(Reply {
            status,
            body: r#"{"message":"x","name":"TrackNotFound","statusCode":404}"#.into(),
        })
    }

    pub(crate) const RECORD: &str = r#"{"id":1,"trackName":"Song","artistName":"Artist","albumName":"Album",
        "duration":200.0,"instrumental":false,"hasWordSync":false,"plainLyrics":"hi",
        "syncedLyrics":"[00:01.00] hi","lyricsfile":"version: '1.0'"}"#;

    fn client(mock: &Mock) -> Lrclib<Mock> {
        Lrclib::with_http(mock.clone(), "http://test/")
    }

    #[test]
    fn get_hit_sends_the_full_signature() {
        let mock = Mock::new([ok(RECORD)]);
        let mut t = track("Don't Stop / Me Now", 200);
        t.artist = "Queen & Co".into();
        t.duration = Some(Duration::from_millis(209_600));
        let raw = client(&mock).fetch(&t).unwrap().unwrap();
        assert_eq!(raw.synced.as_deref(), Some("[00:01.00] hi"));
        assert_eq!(raw.plain.as_deref(), Some("hi"));
        assert_eq!(
            mock.urls(),
            [
                "http://test/api/get?track_name=Don%27t%20Stop%20%2F%20Me%20Now&artist_name=Queen%20%26%20Co&album_name=Album&duration=210"
            ]
        );
    }

    #[test]
    fn miss_falls_back_to_search_and_picks_the_best() {
        let results = r#"[
            {"trackName":"Song","artistName":"Artist","duration":260.0,"instrumental":false,"plainLyrics":"far","syncedLyrics":"[00:01.00]far"},
            {"trackName":"Song","artistName":"Artist","duration":199.0,"instrumental":false,"plainLyrics":"plain only","syncedLyrics":null},
            {"trackName":"Song (Live)","artistName":"Artist","duration":201.0,"instrumental":false,"plainLyrics":"x","syncedLyrics":"[00:01.00]other name"},
            {"trackName":"song","artistName":"ARTIST","duration":202.5,"instrumental":false,"plainLyrics":"x","syncedLyrics":"[00:01.00]best"},
            {"trackName":"Song","artistName":"Artist","duration":200.0,"instrumental":false,"plainLyrics":null,"syncedLyrics":null}
        ]"#;
        let mock = Mock::new([status(404), ok(results)]);
        let mut t = track("Song", 200);
        t.album = String::new();
        let raw = client(&mock).fetch(&t).unwrap().unwrap();
        assert_eq!(raw.synced.as_deref(), Some("[00:01.00]best"));
        let urls = mock.urls();
        assert_eq!(
            urls[0],
            "http://test/api/get?track_name=Song&artist_name=Artist&duration=200"
        );
        assert_eq!(
            urls[1],
            "http://test/api/search?track_name=Song&artist_name=Artist"
        );
    }

    #[test]
    fn no_duration_goes_straight_to_search() {
        let mock = Mock::new([ok(&format!("[{RECORD}]"))]);
        let mut t = track("Song", 0);
        t.duration = None;
        assert!(client(&mock).fetch(&t).unwrap().is_some());
        assert_eq!(mock.urls().len(), 1);
        assert!(mock.urls()[0].contains("/api/search?"));
    }

    #[test]
    fn not_found_is_ok_none() {
        let mock = Mock::new([status(404), ok("[]")]);
        assert_eq!(client(&mock).fetch(&track("Song", 200)), Ok(None));
        // Only results too far off in duration: also none.
        let far = r#"[{"trackName":"Song","artistName":"Artist","duration":300.0,"instrumental":false,"plainLyrics":"x"}]"#;
        let mock = Mock::new([status(404), ok(far)]);
        assert_eq!(client(&mock).fetch(&track("Song", 200)), Ok(None));
    }

    #[test]
    fn instrumental_record() {
        let body = r#"{"trackName":"Song","artistName":"Artist","duration":200.0,"instrumental":true,"plainLyrics":null,"syncedLyrics":null}"#;
        let mock = Mock::new([ok(body)]);
        let raw = client(&mock).fetch(&track("Song", 200)).unwrap().unwrap();
        assert!(raw.instrumental);
        assert_eq!(raw.lyrics(), Some(crate::lyrics::Lyrics::Instrumental));
    }

    #[test]
    fn an_empty_get_record_falls_back_to_search() {
        let empty =
            r#"{"trackName":"Song","artistName":"Artist","duration":200.0,"instrumental":false}"#;
        let mock = Mock::new([ok(empty), ok(&format!("[{RECORD}]"))]);
        assert!(client(&mock).fetch(&track("Song", 200)).unwrap().is_some());
    }

    #[test]
    fn failures_are_errors() {
        let t = track("Song", 200);
        let net = Mock::new([Err("dns".into())]);
        assert_eq!(
            client(&net).fetch(&t),
            Err(FetchError::Network("dns".into()))
        );
        let busy = Mock::new([status(503)]);
        assert_eq!(client(&busy).fetch(&t), Err(FetchError::Status(503)));
        let limited = Mock::new([status(404), status(429)]);
        assert_eq!(client(&limited).fetch(&t), Err(FetchError::Status(429)));
        let junk = Mock::new([ok("<html>")]);
        assert!(matches!(
            client(&junk).fetch(&t),
            Err(FetchError::Malformed(_))
        ));
        assert!(FetchError::Status(503).to_string().contains("503"));
    }

    #[test]
    fn encoding() {
        assert_eq!(encode("a-z_0.9~"), "a-z_0.9~");
        assert_eq!(encode("é ?&="), "%C3%A9%20%3F%26%3D");
    }

    /// One real lookup, to confirm the API shape:
    /// `cargo test -- --ignored --nocapture lrclib_live`.
    #[test]
    #[ignore = "network"]
    fn lrclib_live() {
        let t = Track {
            title: "Bohemian Rhapsody".into(),
            artist: "Queen".into(),
            album: "A Night at the Opera".into(),
            duration: Some(Duration::from_secs(355)),
        };
        let raw = Lrclib::new().fetch(&t).expect("fetch").expect("found");
        let Some(crate::lyrics::Lyrics::Synced(synced)) = raw.lyrics() else {
            panic!("expected synced lyrics: {raw:?}");
        };
        println!("{} lines; first: {:?}", synced.lines.len(), synced.lines[0]);
        assert!(synced.lines.len() > 30);
        assert!(synced.lines[0].text.starts_with("Is this the real life"));
        let none = Track {
            title: "zzqx no such song lavatui".into(),
            artist: "nobody lavatui".into(),
            album: String::new(),
            duration: Some(Duration::from_secs(123)),
        };
        assert_eq!(Lrclib::new().fetch(&none), Ok(None));
    }
}
