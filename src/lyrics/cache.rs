//! The on-disk lyrics cache: one small JSON file per track.
//!
//! Keyed by normalised artist + title + whole-second duration (the album
//! varies between releases of the same recording, so it isn't part of the
//! key). "Not found" is cached too, so a track without lyrics isn't asked
//! about on every play. Each kind of answer has its own lifetime
//! ([`Policy`]); an expired entry is still returned, marked stale, so the
//! caller can show it when a refresh fails (offline).
//!
//! Location: `$XDG_CACHE_HOME/lavatui/lyrics`, else the platform cache dir.
//! Every failure is a miss: a cache problem never blocks lyrics.
//!
//! Size: reading a file marks it used; [`Cache::prune`] (the worker runs it
//! after each write) keeps the [`LIMITS`] newest by last use, so the folder
//! stays at a few MB however many songs are played.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{RawLyrics, Track};
use crate::disk_cache::{self, Limits};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// The cache files' extension.
pub const EXT: &str = "json";
/// A file is ~1-4 KB: at most a few MB. Unused for half a year, a song's
/// lyrics go even below the cap (a stale entry is only an offline fallback).
pub const LIMITS: Limits = Limits {
    files: 2000,
    bytes: 16 * 1000 * 1000,
    idle: Duration::from_secs(180 * 24 * 60 * 60),
};
/// Bump when the file format changes; older files then read as misses.
const VERSION: u32 = 1;

/// How long each kind of answer stays fresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub synced: Duration,
    /// Plain only: synced lyrics may be contributed later.
    pub plain: Duration,
    pub instrumental: Duration,
    pub not_found: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            synced: DAY * 180,
            plain: DAY * 7,
            instrumental: DAY * 180,
            not_found: DAY,
        }
    }
}

impl Policy {
    fn ttl(&self, value: Option<&RawLyrics>) -> Duration {
        match value {
            None => self.not_found,
            Some(raw) if raw.instrumental => self.instrumental,
            Some(raw) if raw.synced.as_deref().is_some_and(|s| !s.trim().is_empty()) => self.synced,
            Some(_) => self.plain,
        }
    }
}

/// A cached answer: `value` is `None` for "LRCLIB has nothing".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub value: Option<RawLyrics>,
    /// Past its [`Policy`] lifetime: refresh, but usable meanwhile.
    pub stale: bool,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    /// The full key, checked on read (the file name is only its hash).
    key: String,
    /// Unix seconds.
    stored: u64,
    value: Option<RawLyrics>,
}

#[derive(Clone, Debug)]
pub struct Cache {
    dir: PathBuf,
    pub policy: Policy,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            policy: Policy::default(),
        }
    }

    /// The default location, if the platform has a cache dir.
    pub fn default_dir() -> Option<PathBuf> {
        disk_cache::dir("lyrics")
    }

    /// The cached answer for `track`; marks its file used `now`.
    pub fn get(&self, track: &Track, now: SystemTime) -> Option<Hit> {
        let key = key(track);
        let path = self.path(&key);
        let text = fs::read_to_string(&path).ok()?;
        let file: File = serde_json::from_str(&text).ok()?;
        if file.version != VERSION || file.key != key {
            return None;
        }
        disk_cache::touch(&path, now);
        let stored = UNIX_EPOCH + Duration::from_secs(file.stored);
        // A file from the future (clock change) counts as stale.
        let stale = match now.duration_since(stored) {
            Ok(age) => age >= self.policy.ttl(file.value.as_ref()),
            Err(_) => true,
        };
        Some(Hit {
            value: file.value,
            stale,
        })
    }

    pub fn put(&self, track: &Track, value: Option<&RawLyrics>, now: SystemTime) -> io::Result<()> {
        let key = key(track);
        let file = File {
            version: VERSION,
            stored: now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            value: value.cloned(),
            key,
        };
        let json = serde_json::to_vec(&file).map_err(io::Error::other)?;
        fs::create_dir_all(&self.dir)?;
        write_atomic(&self.path(&file.key), &json)
    }

    /// Remove what [`LIMITS`] doesn't keep (disk I/O: the worker's job).
    pub fn prune(&self, now: SystemTime) {
        disk_cache::prune(&self.dir, EXT, LIMITS, now);
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir
            .join(format!("{:016x}.{EXT}", fnv1a(key.as_bytes())))
    }
}

/// `artist \x1f title \x1f seconds`, case- and whitespace-insensitive.
fn key(track: &Track) -> String {
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let secs = track
        .duration
        .map_or_else(|| "?".to_string(), |d| d.as_secs_f64().round().to_string());
    format!(
        "{}\x1f{}\x1f{secs}",
        norm(&track.artist),
        norm(&track.title)
    )
}

/// FNV-1a: a hash that's stable across builds (unlike std's), for names.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Write to a temp file beside `path`, then rename over it: readers (or a
/// second lamp) never see half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let tmp = path.with_extension(format!("{}.{nonce:08x}.tmp", std::process::id()));
    let result = fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(bytes))
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh, empty temp dir, removed on drop.
    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub(crate) fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir()
                .join(format!("lavatui-{name}-{}-{nonce:x}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn track(title: &str, secs: u64) -> Track {
        Track {
            title: title.into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration: Some(Duration::from_secs(secs)),
        }
    }

    fn synced() -> RawLyrics {
        RawLyrics {
            instrumental: false,
            synced: Some("[00:01.00]hi".into()),
            plain: Some("hi".into()),
        }
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs)
    }

    #[test]
    fn miss_then_hit() {
        let tmp = TempDir::new("lyrics-hit");
        // Not created yet: the cache makes its dir on first put.
        let cache = Cache::new(tmp.0.join("lyrics"));
        let t = track("Song", 200);
        assert_eq!(cache.get(&t, at(0)), None);
        cache.put(&t, Some(&synced()), at(0)).unwrap();
        let hit = cache.get(&t, at(10)).unwrap();
        assert_eq!(hit.value, Some(synced()));
        assert!(!hit.stale);
    }

    #[test]
    fn key_ignores_case_spacing_album_and_subsecond_duration() {
        let tmp = TempDir::new("lyrics-key");
        let cache = Cache::new(tmp.0.clone());
        cache
            .put(&track("My  Song", 200), Some(&synced()), at(0))
            .unwrap();
        let mut same = track(" my song ", 200);
        same.album = "Deluxe".into();
        same.duration = Some(Duration::from_millis(200_400));
        assert!(cache.get(&same, at(1)).is_some());
        // A different duration (another version) is another entry.
        assert_eq!(cache.get(&track("My Song", 230), at(1)), None);
        let mut unknown = track("My Song", 0);
        unknown.duration = None;
        assert_eq!(cache.get(&unknown, at(1)), None);
    }

    #[test]
    fn negative_results_expire_sooner() {
        let tmp = TempDir::new("lyrics-ttl");
        let cache = Cache::new(tmp.0.clone());
        let missing = track("Missing", 100);
        let found = track("Found", 100);
        cache.put(&missing, None, at(0)).unwrap();
        cache.put(&found, Some(&synced()), at(0)).unwrap();

        let day = DAY.as_secs();
        let hit = cache.get(&missing, at(day - 1)).unwrap();
        assert_eq!(
            hit,
            Hit {
                value: None,
                stale: false
            }
        );
        // Expired: still returned, marked stale.
        assert!(cache.get(&missing, at(day)).unwrap().stale);
        assert!(!cache.get(&found, at(day * 30)).unwrap().stale);
        assert!(cache.get(&found, at(day * 180)).unwrap().stale);
        // A clock that went backwards: stale, not fresh forever.
        assert!(cache.get(&found, UNIX_EPOCH).unwrap().stale);
    }

    #[test]
    fn ttl_by_kind() {
        let p = Policy::default();
        let plain = RawLyrics {
            plain: Some("a".into()),
            synced: Some("  ".into()),
            ..RawLyrics::default()
        };
        let instrumental = RawLyrics {
            instrumental: true,
            ..RawLyrics::default()
        };
        assert_eq!(p.ttl(Some(&plain)), p.plain);
        assert_eq!(p.ttl(Some(&instrumental)), p.instrumental);
        assert_eq!(p.ttl(Some(&synced())), p.synced);
        assert_eq!(p.ttl(None), p.not_found);
    }

    #[test]
    fn overwrite_and_corrupt_files() {
        let tmp = TempDir::new("lyrics-corrupt");
        let cache = Cache::new(tmp.0.clone());
        let t = track("Song", 200);
        cache.put(&t, None, at(0)).unwrap();
        cache.put(&t, Some(&synced()), at(5)).unwrap();
        assert_eq!(cache.get(&t, at(6)).unwrap().value, Some(synced()));
        // Exactly one file, no temp files left behind.
        assert_eq!(fs::read_dir(&tmp.0).unwrap().count(), 1);

        fs::write(cache.path(&key(&t)), "{ not json").unwrap();
        assert_eq!(cache.get(&t, at(6)), None);
        // A hash collision (another key in the file) is a miss too.
        let other = File {
            version: VERSION,
            key: "someone\x1felse\x1f1".into(),
            stored: 0,
            value: None,
        };
        fs::write(cache.path(&key(&t)), serde_json::to_vec(&other).unwrap()).unwrap();
        assert_eq!(cache.get(&t, at(6)), None);
    }

    #[test]
    fn reading_marks_used_and_prune_keeps_the_recently_used() {
        let tmp = TempDir::new("lyrics-prune");
        let cache = Cache::new(tmp.0.clone());
        let n = LIMITS.files + 3;
        let day = DAY.as_secs();
        for i in 0..n {
            cache
                .put(&track(&format!("Song {i}"), 100), None, at(0))
                .unwrap();
            disk_cache::touch(
                &cache.path(&key(&track(&format!("Song {i}"), 100))),
                at(i as u64),
            );
        }
        // The very first song, played again: now the most recently used.
        assert!(cache.get(&track("Song 0", 100), at(n as u64)).is_some());
        cache.prune(at(n as u64));
        assert_eq!(disk_cache::usage(&tmp.0, EXT).files, LIMITS.files);
        assert!(cache.get(&track("Song 0", 100), at(n as u64)).is_some());
        for i in 1..=3 {
            let gone = track(&format!("Song {i}"), 100);
            assert_eq!(cache.get(&gone, at(n as u64)), None, "song {i}");
        }
        // Half a year later, everything unused since goes.
        cache.prune(at(n as u64 + 180 * day));
        assert_eq!(disk_cache::usage(&tmp.0, EXT).files, 0);
    }

    #[test]
    fn unwritable_dir_is_an_error_not_a_panic() {
        let tmp = TempDir::new("lyrics-ro");
        let blocker = tmp.0.join("file");
        fs::write(&blocker, "").unwrap();
        let cache = Cache::new(blocker.join("lyrics"));
        assert!(cache.put(&track("x", 1), None, at(0)).is_err());
        assert_eq!(cache.get(&track("x", 1), at(0)), None);
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
