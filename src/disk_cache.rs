//! Housekeeping shared by the on-disk caches (lyrics, album covers):
//! where they live, the limits that keep them small, and measuring and
//! clearing them for the settings screen.
//!
//! Every function here touches the disk: call them on a worker thread,
//! never on the frame path. Each cache keeps its files flat in its own
//! folder, one extension per cache, so nothing else is ever deleted.
//! Problems are ignored (a file that can't be removed is just kept).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Leftover temp files (a write cut short) older than this are removed.
const TEMP_AGE: Duration = Duration::from_secs(60 * 60);

/// `$XDG_CACHE_HOME/lavatui/<name>`, else the platform cache dir's.
pub fn dir(name: &str) -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from) {
        Some(xdg) if xdg.is_absolute() => xdg.join("lavatui"),
        _ => directories::ProjectDirs::from("", "", "lavatui")?
            .cache_dir()
            .to_path_buf(),
    };
    Some(base.join(name))
}

/// How big a cache may get. The least recently used files go first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub files: usize,
    pub bytes: u64,
    /// Files not used for this long are removed.
    pub idle: Duration,
}

/// What a cache holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub files: usize,
    pub bytes: u64,
}

impl std::ops::Add for Usage {
    type Output = Usage;
    fn add(self, other: Usage) -> Usage {
        Usage {
            files: self.files + other.files,
            bytes: self.bytes + other.bytes,
        }
    }
}

/// Mark a cached file as used `now` (its modified time), so pruning keeps
/// it. Never creates a file.
pub fn touch(path: &Path, now: SystemTime) {
    if let Ok(file) = fs::File::options().write(true).open(path) {
        let _ = file.set_modified(now);
    }
}

/// The cache's `.ext` files: path, size, last use.
fn entries(dir: &Path, ext: &str) -> Vec<(PathBuf, u64, SystemTime)> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    read.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == ext))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file()
                .then(|| Some((e.path(), meta.len(), meta.modified().ok()?)))?
        })
        .collect()
}

/// Remove `.ext` files past `limits`: unused for too long, or beyond the
/// newest that fit the file and byte caps. Old temp files go too. Returns
/// how many files were removed.
pub fn prune(dir: &Path, ext: &str, limits: Limits, now: SystemTime) -> usize {
    let age = |t: SystemTime| now.duration_since(t).unwrap_or(Duration::ZERO);
    let mut files = entries(dir, ext);
    // Newest first.
    files.sort_by_key(|f| std::cmp::Reverse(f.2));
    let (mut kept, mut bytes, mut removed) = (0, 0, 0);
    for (path, len, used) in files {
        let keep = kept < limits.files && bytes + len <= limits.bytes && age(used) < limits.idle;
        if keep {
            kept += 1;
            bytes += len;
        } else if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    for (path, _, used) in entries(dir, "tmp") {
        if age(used) >= TEMP_AGE && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// How many `.ext` files the cache holds, and their size.
pub fn usage(dir: &Path, ext: &str) -> Usage {
    entries(dir, ext)
        .into_iter()
        .fold(Usage::default(), |u, (_, len, _)| {
            u + Usage {
                files: 1,
                bytes: len,
            }
        })
}

/// Remove every `.ext` file (the folder stays).
pub fn clear(dir: &Path, ext: &str) {
    for (path, _, _) in entries(dir, ext) {
        let _ = fs::remove_file(path);
    }
}

/// A size in plain words: `0 KB`, `12 KB`, `3.4 MB`.
pub fn size_words(bytes: u64) -> String {
    const KB: u64 = 1000;
    const MB: u64 = KB * KB;
    if bytes < MB {
        format!("{} KB", bytes.div_ceil(KB))
    } else if bytes < 10 * MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{} MB", (bytes + MB / 2) / MB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lyrics::cache::tests::TempDir;

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    fn at(days: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000) + DAY * days as u32
    }

    /// A `len`-byte file `name`, last used on day `day`.
    fn file(dir: &Path, name: &str, len: usize, day: u64) {
        let path = dir.join(name);
        fs::write(&path, vec![b'x'; len]).unwrap();
        touch(&path, at(day));
    }

    fn left(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    const ROOMY: Limits = Limits {
        files: 100,
        bytes: 1_000_000,
        idle: Duration::from_secs(180 * 24 * 60 * 60),
    };

    #[test]
    fn prune_keeps_the_most_recently_used_within_the_file_cap() {
        let tmp = TempDir::new("disk-files");
        for i in 0..5 {
            file(&tmp.0, &format!("{i}.c"), 10, i);
        }
        // An old file used again today is kept.
        touch(&tmp.0.join("0.c"), at(10));
        let limits = Limits { files: 3, ..ROOMY };
        assert_eq!(prune(&tmp.0, "c", limits, at(10)), 2);
        assert_eq!(left(&tmp.0), ["0.c", "3.c", "4.c"]);
    }

    #[test]
    fn prune_keeps_the_newest_within_the_byte_cap() {
        let tmp = TempDir::new("disk-bytes");
        file(&tmp.0, "old.c", 400, 1);
        file(&tmp.0, "mid.c", 400, 2);
        file(&tmp.0, "new.c", 400, 3);
        let limits = Limits {
            bytes: 1000,
            ..ROOMY
        };
        assert_eq!(prune(&tmp.0, "c", limits, at(3)), 1);
        assert_eq!(left(&tmp.0), ["mid.c", "new.c"]);
        assert_eq!(
            usage(&tmp.0, "c"),
            Usage {
                files: 2,
                bytes: 800
            }
        );
    }

    #[test]
    fn prune_drops_files_unused_too_long_and_old_temp_files() {
        let tmp = TempDir::new("disk-idle");
        file(&tmp.0, "stale.c", 1, 0);
        file(&tmp.0, "fresh.c", 1, 179);
        file(&tmp.0, "other.txt", 1, 0);
        file(&tmp.0, "cut.1.2.tmp", 1, 0);
        file(&tmp.0, "writing.1.2.tmp", 1, 180);
        // From the future (the clock went back): kept.
        file(&tmp.0, "future.c", 1, 400);
        prune(&tmp.0, "c", ROOMY, at(180));
        assert_eq!(
            left(&tmp.0),
            ["fresh.c", "future.c", "other.txt", "writing.1.2.tmp"]
        );
    }

    #[test]
    fn clear_removes_only_the_cache_files() {
        let tmp = TempDir::new("disk-clear");
        file(&tmp.0, "a.c", 5, 0);
        file(&tmp.0, "b.c", 7, 0);
        file(&tmp.0, "keep.txt", 1, 0);
        assert_eq!(
            usage(&tmp.0, "c"),
            Usage {
                files: 2,
                bytes: 12
            }
        );
        clear(&tmp.0, "c");
        assert_eq!(usage(&tmp.0, "c"), Usage::default());
        assert_eq!(left(&tmp.0), ["keep.txt"]);
        // A missing folder is empty, not an error.
        let gone = tmp.0.join("missing");
        assert_eq!(usage(&gone, "c"), Usage::default());
        clear(&gone, "c");
        assert_eq!(prune(&gone, "c", ROOMY, at(0)), 0);
    }

    #[test]
    fn touch_never_creates() {
        let tmp = TempDir::new("disk-touch");
        touch(&tmp.0.join("none.c"), at(0));
        assert!(left(&tmp.0).is_empty());
    }

    #[test]
    fn sizes_in_words() {
        assert_eq!(size_words(0), "0 KB");
        assert_eq!(size_words(1), "1 KB");
        assert_eq!(size_words(12_300), "13 KB");
        assert_eq!(size_words(3_400_000), "3.4 MB");
        assert_eq!(size_words(48_600_000), "49 MB");
    }
}
