//! Album art: fetched, cached on disk and decoded off the UI thread.
//!
//! [`ArtLoader`] is a handle to a worker thread. The UI says which cover it
//! wants ([`ArtLoader::want`], once per track change; repeating the same
//! URL is free) and reads what has arrived with [`ArtLoader::get`], a short
//! lock. The worker looks in the disk cache first, else downloads the image
//! (`https` only, size-capped), stores the raw bytes, then decodes it,
//! crops it square and shrinks it to [`ART_PX`]² ([`Art`]): small enough to
//! scale to any cell size every frame for free. When pixels are wanted
//! ([`ArtLoader::set_hires`]: kitty, iTerm2 or sixel images) it also keeps a
//! sharper copy, up to [`HIRES_PX`]² and ready to send: PNG, base64. Pixel
//! art (the cover quality's small, medium and big pixels) is made from the
//! small copy when it's wanted, as many blocks as the text cells show
//! ([`Art::pixel_art`]).
//!
//! Cache: `$XDG_CACHE_HOME/lavatui/art`, else the platform cache dir; one
//! file per URL (named by its SHA-256). A cover read from it is marked
//! used; after each download the least recently used go, past
//! [`CACHE_LIMITS`] (files, bytes, months unused). Every cache problem is
//! a miss, and a failed cover is just no cover: nothing here ever reaches
//! the UI as an error.
//!
//! Players that hand out the cover's bytes rather than a URL (Windows'
//! media controls, lava-75z.16) [`stash`] them on their own worker and put
//! the `lavatui-thumb:` URL it returns in `Track::artwork_url`; the art
//! worker decodes stashed covers like downloaded ones, without the disk.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::disk_cache::{self, Limits};
use crate::theme::Rgb;

/// Decoded covers are this many pixels square.
pub const ART_PX: u32 = 128;
/// The sharp copy for real pixels is at most this many pixels square
/// (Spotify's covers are 640): sharp up to a ~40-column cover on a
/// 10-pixel-wide cell, and ~200-300 KB to send.
pub const HIRES_PX: u32 = 400;
/// The cover cache's files' extension.
pub const CACHE_EXT: &str = "img";
/// Covers kept on disk: a Spotify cover is ~60 KB (~15 MB for 256), the
/// byte cap is for players that hand out bigger pictures.
pub const CACHE_LIMITS: Limits = Limits {
    files: 256,
    bytes: 50 * 1000 * 1000,
    idle: Duration::from_secs(180 * 24 * 60 * 60),
};
/// Bigger downloads are refused (Spotify's 640 px covers are ~100 KB).
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
/// Waits before each retry of a download that failed (a newer cover
/// interrupts them).
pub const RETRY_AFTER: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(4)];
/// After those, how long until the cover still wanted is asked for again.
pub const ASK_AGAIN: Duration = Duration::from_secs(30);
/// The scheme of covers handed over as bytes ([`stash`]).
pub const THUMB_SCHEME: &str = "lavatui-thumb:";
/// Stashed covers kept (the playing one and a few before it).
const STASHED: usize = 4;

/// Covers handed over as bytes, newest last.
type Stash = Mutex<Vec<(String, Arc<[u8]>)>>;
static STASH: Stash = Mutex::new(Vec::new());

/// Keep a cover's encoded bytes (JPEG / PNG) for the art worker, and
/// return the URL to ask for it by (`lavatui-thumb:` + its SHA-256, so the
/// same picture is the same URL). `None` for nothing or too much.
pub fn stash(bytes: Vec<u8>) -> Option<String> {
    stash_in(&STASH, bytes)
}

fn stash_in(stash: &Stash, bytes: Vec<u8>) -> Option<String> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let hash = Sha256::digest(&bytes);
    let hex: String = hash[..16].iter().map(|b| format!("{b:02x}")).collect();
    let url = format!("{THUMB_SCHEME}{hex}");
    let mut stash = lock(stash);
    stash.retain(|(u, _)| *u != url);
    stash.push((url.clone(), Arc::from(bytes)));
    let over = stash.len().saturating_sub(STASHED);
    stash.drain(..over);
    Some(url)
}

/// The bytes [`stash`] kept for `url`, while it still has them.
fn stashed(url: &str) -> Option<Arc<[u8]>> {
    stashed_in(&STASH, url)
}

fn stashed_in(stash: &Stash, url: &str) -> Option<Arc<[u8]>> {
    lock(stash)
        .iter()
        .find(|(u, _)| u == url)
        .map(|(_, bytes)| Arc::clone(bytes))
}

/// A cover, decoded and shrunk: `ART_PX`² pixels, row by row, and the
/// sharp copy when it was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Art {
    pixels: Vec<Rgb>,
    /// The average colour, worked out once (drawn behind the picture
    /// every frame).
    mean: Rgb,
    /// Up to [`HIRES_PX`]² as PNG, base64: what the kitty and iTerm2
    /// protocols send (and what a sixel picture is made from).
    pub hires: Option<Arc<String>>,
}

impl Art {
    /// Decode a JPEG or PNG, crop it to its centre square and shrink it
    /// (and keep the sharp copy too, with `hires`).
    pub fn decode(bytes: &[u8], hires: bool) -> Result<Self, String> {
        let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
        let (w, h) = (image.width(), image.height());
        let side = w.min(h);
        if side == 0 {
            return Err("empty image".into());
        }
        let square = image.crop_imm((w - side) / 2, (h - side) / 2, side, side);
        let small = square.thumbnail_exact(ART_PX, ART_PX).to_rgb8();
        let pixels = small.pixels().map(|p| Rgb(p[0], p[1], p[2])).collect();
        let mut art = Self::from_pixels(pixels);
        if hires {
            let n = side.min(HIRES_PX);
            let sharp = if n == side {
                square.to_rgb8()
            } else {
                square
                    .resize_exact(n, n, image::imageops::FilterType::CatmullRom)
                    .to_rgb8()
            };
            art.hires = encode(&sharp);
        }
        Ok(art)
    }

    /// The cover as pixel art, `across` × `down` flat blocks (each the
    /// mean of what it covers, as in text cells), ready to send: PNG,
    /// base64, about [`HIRES_PX`] pixels on its longer side so the
    /// terminal's own scaling keeps the edges crisp. About a millisecond:
    /// made once per cover and size, and kept by the caller.
    pub fn pixel_art(&self, (across, down): (u16, u16)) -> Option<Arc<String>> {
        let (nx, ny) = (across.max(1), down.max(1));
        let px = self.scaled(nx, ny);
        let (nx, ny) = (u32::from(nx), u32::from(ny));
        let f = HIRES_PX.div_ceil(nx.max(ny));
        let image = image::RgbImage::from_fn(nx * f, ny * f, |x, y| {
            let Rgb(r, g, b) = px[((y / f) * nx + x / f) as usize];
            image::Rgb([r, g, b])
        });
        encode(&image)
    }

    /// A flat colour, for tests.
    #[cfg(test)]
    pub fn solid(c: Rgb) -> Self {
        Self::from_pixels(vec![c; (ART_PX * ART_PX) as usize])
    }

    /// Pixel (`x`, `y`) of the square source from `f`, for tests.
    #[cfg(test)]
    pub fn from_fn(f: impl Fn(u32, u32) -> Rgb) -> Self {
        Self::from_pixels(
            (0..ART_PX * ART_PX)
                .map(|i| f(i % ART_PX, i / ART_PX))
                .collect(),
        )
    }

    /// With a sharp copy (`b64`, standing in for a PNG), for tests.
    #[cfg(test)]
    pub fn with_hires(mut self, b64: &str) -> Self {
        self.hires = Some(Arc::new(b64.to_owned()));
        self
    }

    /// `ART_PX`² pixels, row by row, and no sharp copy.
    fn from_pixels(pixels: Vec<Rgb>) -> Self {
        let mut art = Self {
            pixels,
            mean: Rgb::default(),
            hires: None,
        };
        art.mean = art.scaled(1, 1)[0];
        art
    }

    /// The cover's average colour.
    pub fn mean(&self) -> Rgb {
        self.mean
    }

    /// The cover at `w` × `h` pixels, row by row, each the average of the
    /// source pixels it covers (a box filter: no shimmer at small sizes).
    pub fn scaled(&self, w: u16, h: u16) -> Vec<Rgb> {
        let n = ART_PX as usize;
        let (w, h) = (usize::from(w), usize::from(h));
        let span = |i: usize, len: usize| {
            let a = i * n / len;
            (a, ((i + 1) * n / len).max(a + 1).min(n))
        };
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            let (y0, y1) = span(y, h);
            for x in 0..w {
                let (x0, x1) = span(x, w);
                let mut sum = [0u32; 3];
                for row in self.pixels[y0 * n..y1 * n].chunks(n) {
                    for p in &row[x0..x1] {
                        sum[0] += u32::from(p.0);
                        sum[1] += u32::from(p.1);
                        sum[2] += u32::from(p.2);
                    }
                }
                let count = ((y1 - y0) * (x1 - x0)) as u32;
                let avg = |s: u32| ((s + count / 2) / count) as u8;
                out.push(Rgb(avg(sum[0]), avg(sum[1]), avg(sum[2])));
            }
        }
        out
    }
}

/// `image` as PNG, base64.
fn encode(image: &image::RgbImage) -> Option<Arc<String>> {
    use base64::Engine;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    Some(Arc::new(b64))
}

/// What the loader has for the cover it was last asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtState {
    /// Nothing asked for yet, or still on its way.
    Loading,
    Ready(Arc<Art>),
    /// Couldn't be had (offline, a bad image): draw no cover.
    Missing,
}

/// Gets the bytes behind a URL (blocking; the worker's only I/O seam).
pub trait Fetch: Send + 'static {
    fn get(&mut self, url: &str) -> Result<Vec<u8>, String>;
}

/// [`Fetch`] over `ureq`.
pub struct Ureq(ureq::Agent);

impl Ureq {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(crate::lyrics::client::USER_AGENT)
            .build();
        Self(config.into())
    }
}

impl Fetch for Ureq {
    fn get(&mut self, url: &str) -> Result<Vec<u8>, String> {
        let mut response = self.0.get(url).call().map_err(|e| e.to_string())?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .read_to_vec()
            .map_err(|e| e.to_string())
    }
}

/// What the worker is asked for: a URL, and whether the sharp copy too.
type Request = (String, bool);

/// How a download that failed is tried again (lava-75z.22: a network blip
/// as a song began used to leave it without a cover until the next one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retry {
    /// Waits before each retry, on the worker.
    pub after: Vec<Duration>,
    /// After those, how long until [`ArtLoader::want`] asks again for the
    /// cover still wanted (and again, as long as it fails).
    pub again: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            after: RETRY_AFTER.to_vec(),
            again: ASK_AGAIN,
        }
    }
}

/// The cover the worker last finished.
#[derive(Debug)]
struct Done {
    request: Request,
    state: ArtState,
    /// When its download failed (worth asking again), if it did.
    failed_at: Option<Instant>,
}

type Latest = Arc<Mutex<Option<Done>>>;

/// Handle to the art worker. Dropping it stops the worker once it has
/// finished the cover in hand.
pub struct ArtLoader {
    requests: Option<Sender<Request>>,
    latest: Latest,
    wanted: Option<Request>,
    hires: bool,
    again: Duration,
}

impl ArtLoader {
    /// A loader over the network, caching in the default cache dir.
    pub fn start() -> Self {
        Self::spawn(Ureq::new(), cache_dir())
    }

    pub fn spawn<F: Fetch>(fetch: F, cache: Option<PathBuf>) -> Self {
        Self::spawn_with(fetch, cache, Retry::default())
    }

    pub fn spawn_with<F: Fetch>(fetch: F, cache: Option<PathBuf>, retry: Retry) -> Self {
        let latest = Latest::default();
        let (tx, rx) = mpsc::channel();
        let again = retry.again;
        let worker = Worker {
            fetch,
            cache,
            requests: rx,
            latest: Arc::clone(&latest),
            retry_after: retry.after,
        };
        let spawned = thread::Builder::new()
            .name("lavatui-art".into())
            .spawn(move || {
                crate::thread_qos::worker();
                worker.run()
            });
        Self {
            requests: spawned.is_ok().then_some(tx),
            latest,
            wanted: None,
            hires: false,
            again,
        }
    }

    /// A loader that already has `art` for `url` and never fetches, for
    /// tests of what draws it.
    #[cfg(test)]
    pub fn preloaded(url: &str, art: Art) -> Self {
        let hires = art.hires.is_some();
        let request = (url.to_owned(), hires);
        Self {
            requests: None,
            latest: Arc::new(Mutex::new(Some(Done {
                request: request.clone(),
                state: ArtState::Ready(Arc::new(art)),
                failed_at: None,
            }))),
            wanted: Some(request),
            hires,
            again: ASK_AGAIN,
        }
    }

    /// Whether covers come with the sharp copy (from the next [`want`]).
    ///
    /// [`want`]: Self::want
    pub fn set_hires(&mut self, on: bool) {
        self.hires = on;
    }

    /// Ask for the cover at `url` (a no-op if it's the one already asked
    /// for, unless its download failed a while ago: see [`Retry`]).
    /// Anything but `https` is never fetched; stashed covers
    /// (`lavatui-thumb:`) are decoded from memory.
    pub fn want(&mut self, url: &str) {
        let request = (url.to_owned(), self.hires);
        if self.wanted.as_ref() == Some(&request) {
            let mut latest = lock(&self.latest);
            let due = latest.as_mut().filter(|done| {
                done.request == request
                    && done.failed_at.is_some_and(|at| at.elapsed() >= self.again)
            });
            if let Some(done) = due {
                // Still shown as missing until the worker says otherwise.
                done.failed_at = None;
                drop(latest);
                self.send(request);
            }
            return;
        }
        self.wanted = Some(request.clone());
        let fetchable = url.starts_with("https://") || url.starts_with(THUMB_SCHEME);
        if !(fetchable && self.send(request.clone())) {
            *lock(&self.latest) = Some(Done {
                request,
                state: ArtState::Missing,
                failed_at: None,
            });
        }
    }

    fn send(&self, request: Request) -> bool {
        self.requests
            .as_ref()
            .is_some_and(|tx| tx.send(request).is_ok())
    }

    /// The cover last asked for, as far as it has got.
    pub fn get(&self) -> ArtState {
        let Some(wanted) = &self.wanted else {
            return ArtState::Loading;
        };
        match &*lock(&self.latest) {
            Some(done) if done.request == *wanted => done.state.clone(),
            _ => ArtState::Loading,
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `$XDG_CACHE_HOME/lavatui/art`, else the platform cache dir's.
pub fn cache_dir() -> Option<PathBuf> {
    disk_cache::dir("art")
}

struct Worker<F> {
    fetch: F,
    cache: Option<PathBuf>,
    requests: Receiver<Request>,
    latest: Latest,
    retry_after: Vec<Duration>,
}

/// Why a cover couldn't be had.
enum Failure {
    /// The download failed: worth trying again.
    Fetch(String),
    /// A picture that won't decode, a stashed one that's gone: final.
    Bad,
}

impl<F: Fetch> Worker<F> {
    fn run(mut self) {
        let Ok(mut next) = self.requests.recv() else {
            return;
        };
        loop {
            // Skipped through tracks quickly: only the last one matters.
            let request = self.requests.try_iter().last().unwrap_or(next);
            match self.attempt(&request) {
                Ok((state, failed_at)) => {
                    *lock(&self.latest) = Some(Done {
                        request,
                        state,
                        failed_at,
                    });
                }
                // A newer cover came while waiting to retry: that one now.
                Err(newer) => {
                    next = newer;
                    continue;
                }
            }
            match self.requests.recv() {
                Ok(request) => next = request,
                Err(_) => return,
            }
        }
    }

    /// Load `request`, retrying a failed download after each wait in
    /// `retry_after`; or the newer request that came meanwhile.
    fn attempt(&mut self, request: &Request) -> Result<(ArtState, Option<Instant>), Request> {
        let mut tries = 0;
        loop {
            let why = match self.load(&request.0, request.1) {
                Ok(art) => return Ok((ArtState::Ready(Arc::new(art)), None)),
                Err(Failure::Bad) => return Ok((ArtState::Missing, None)),
                Err(Failure::Fetch(why)) => why,
            };
            let Some(&wait) = self.retry_after.get(tries) else {
                crate::diag::note(|| {
                    format!(
                        "art: cover {} not downloaded after {} tries: {why}",
                        crate::diag::tag(&request.0),
                        tries + 1
                    )
                });
                return Ok((ArtState::Missing, Some(Instant::now())));
            };
            tries += 1;
            match self.requests.recv_timeout(wait) {
                Ok(newer) => return Err(newer),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Ok((ArtState::Missing, None));
                }
            }
        }
    }

    fn load(&mut self, url: &str, hires: bool) -> Result<Art, Failure> {
        if url.starts_with(THUMB_SCHEME) {
            let bytes = stashed(url).ok_or(Failure::Bad)?;
            return Art::decode(&bytes, hires).map_err(|_| Failure::Bad);
        }
        let path = self.cache.as_ref().map(|dir| dir.join(file_name(url)));
        if let Some((p, art)) = path.as_ref().and_then(|p| {
            let bytes = fs::read(p).ok()?;
            Some((p, Art::decode(&bytes, hires).ok()?))
        }) {
            disk_cache::touch(p, SystemTime::now());
            return Ok(art);
        }
        let bytes = self.fetch.get(url).map_err(Failure::Fetch)?;
        let art = Art::decode(&bytes, hires).map_err(|_| Failure::Bad)?;
        if let (Some(dir), Some(path)) = (&self.cache, &path) {
            let _ = fs::create_dir_all(dir).and_then(|()| write_atomic(path, &bytes));
            disk_cache::prune(dir, CACHE_EXT, CACHE_LIMITS, SystemTime::now());
        }
        Ok(art)
    }
}

/// The cache file for `url`: its SHA-256, so any URL makes a safe name.
fn file_name(url: &str) -> String {
    let hash = Sha256::digest(url.as_bytes());
    let hex: String = hash[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}.{CACHE_EXT}")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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
mod tests {
    use std::time::Instant;

    use image::{ImageBuffer, ImageFormat, Rgb as Px};

    use super::*;
    use crate::lyrics::cache::tests::TempDir;

    /// A `w`×`h` PNG: left half red, right half blue.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = ImageBuffer::from_fn(w, h, |x, _| {
            if x < w / 2 {
                Px([255u8, 0, 0])
            } else {
                Px([0, 0, 255])
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn decodes_crops_square_and_scales() {
        // 200×100: the centre 100×100 square is still half red, half blue.
        let art = Art::decode(&png(200, 100), false).unwrap();
        assert_eq!(art.hires, None);
        let px = art.scaled(4, 2);
        assert_eq!(px.len(), 8);
        assert_eq!(px[0], Rgb(255, 0, 0));
        assert_eq!(px[3], Rgb(0, 0, 255));
        // Any size, never empty, never out of bounds.
        for (w, h) in [(1, 1), (3, 7), (64, 64), (100, 50)] {
            assert_eq!(art.scaled(w, h).len(), usize::from(w * h));
        }
        assert_eq!(art.mean(), art.scaled(1, 1)[0], "worked out once");
        let Rgb(r, g, b) = art.mean();
        assert!((127..=128).contains(&r) && g == 0 && (127..=128).contains(&b));
        assert!(Art::decode(b"not an image", false).is_err());
    }

    #[test]
    fn the_sharp_copy_is_a_square_png_capped_in_size() {
        use base64::Engine;
        let decode = |w, h| {
            let art = Art::decode(&png(w, h), true).unwrap();
            let b64 = art.hires.expect("asked for");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.as_bytes())
                .unwrap();
            let img = image::load_from_memory_with_format(&bytes, ImageFormat::Png).unwrap();
            (img.width(), img.height())
        };
        assert_eq!(decode(900, 640), (HIRES_PX, HIRES_PX));
        assert_eq!(decode(120, 300), (120, 120), "never enlarged");
    }

    /// Pixel art: flat blocks, as many as asked for across and down,
    /// about as big as the sharp copy.
    #[test]
    fn pixel_art_is_flat_blocks() {
        use base64::Engine;
        let art = Art::decode(&png(200, 100), false).unwrap();
        for (nx, ny) in [(32, 32), (12, 11), (5, 5)] {
            let b64 = art.pixel_art((nx, ny)).unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.as_bytes())
                .unwrap();
            let img = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                .unwrap()
                .to_rgb8();
            let f = img.width() / u32::from(nx);
            assert_eq!(
                (img.width(), img.height()),
                (f * u32::from(nx), f * u32::from(ny))
            );
            assert!(img.width() >= HIRES_PX);
            // Every block one colour: half red, half blue, a blend in the
            // middle column only where nx is odd.
            for by in 0..u32::from(ny) {
                for bx in 0..u32::from(nx) {
                    let c = img.get_pixel(bx * f, by * f);
                    assert!((0..f).all(|d| img.get_pixel(bx * f + d, by * f + f - 1 - d) == c));
                }
            }
            assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0]);
            assert_eq!(img.get_pixel(img.width() - 1, 0).0, [0, 0, 255]);
        }
    }

    /// Serves one PNG, counting requests; fails after `fail_after`.
    struct Served(Arc<Mutex<u32>>, u32);

    impl Fetch for Served {
        fn get(&mut self, url: &str) -> Result<Vec<u8>, String> {
            let mut n = self.0.lock().unwrap();
            *n += 1;
            if *n > self.1 || url.contains("broken") {
                return Err("offline".into());
            }
            Ok(png(8, 8))
        }
    }

    fn wait(loader: &ArtLoader) -> ArtState {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match loader.get() {
                ArtState::Loading if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(2));
                }
                state => return state,
            }
        }
    }

    #[test]
    fn loads_caches_and_serves_from_the_cache_offline() {
        let dir = TempDir::new("art");
        let count = Arc::new(Mutex::new(0));
        let mut loader = ArtLoader::spawn(Served(Arc::clone(&count), 1), Some(dir.0.clone()));
        assert_eq!(loader.get(), ArtState::Loading);
        loader.want("https://i.example/a");
        assert!(matches!(wait(&loader), ArtState::Ready(_)));
        loader.want("https://i.example/a"); // same: no new request
        assert_eq!(*count.lock().unwrap(), 1);
        // The sharp copy is another request (from the disk cache).
        loader.set_hires(true);
        loader.want("https://i.example/a");
        match wait(&loader) {
            ArtState::Ready(art) => assert!(art.hires.is_some()),
            other => panic!("{other:?}"),
        }
        assert_eq!(*count.lock().unwrap(), 1);

        // A new loader, offline: the cover comes from disk, and is marked
        // used (so pruning keeps it).
        let file = dir.0.join(file_name("https://i.example/a"));
        let long_ago = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        disk_cache::touch(&file, long_ago);
        let mut offline = ArtLoader::spawn_with(
            Served(Arc::new(Mutex::new(0)), 0),
            Some(dir.0.clone()),
            fast_retry(),
        );
        offline.want("https://i.example/a");
        assert!(matches!(wait(&offline), ArtState::Ready(_)));
        let used = fs::metadata(&file).unwrap().modified().unwrap();
        assert!(used > long_ago);
        offline.want("https://i.example/broken");
        assert_eq!(wait(&offline), ArtState::Missing);
    }

    fn fast_retry() -> Retry {
        Retry {
            after: vec![Duration::from_millis(5)],
            again: Duration::from_millis(30),
        }
    }

    /// Fails the first `fails` downloads, then serves a PNG; counts them.
    struct Flaky {
        fails: u32,
        count: Arc<Mutex<u32>>,
    }

    impl Fetch for Flaky {
        fn get(&mut self, _: &str) -> Result<Vec<u8>, String> {
            let mut n = self.count.lock().unwrap();
            *n += 1;
            if *n <= self.fails {
                return Err("network down".into());
            }
            Ok(png(8, 8))
        }
    }

    fn flaky(fails: u32) -> (ArtLoader, Arc<Mutex<u32>>) {
        let count = Arc::new(Mutex::new(0));
        let fetch = Flaky {
            fails,
            count: Arc::clone(&count),
        };
        (ArtLoader::spawn_with(fetch, None, fast_retry()), count)
    }

    /// lava-75z.22: a download that fails once (a network blip as the song
    /// starts) is retried on the worker, and the cover comes.
    #[test]
    fn a_failed_download_is_retried() {
        let (mut loader, count) = flaky(1);
        loader.want("https://i.example/a");
        assert!(matches!(wait(&loader), ArtState::Ready(_)));
        assert_eq!(*count.lock().unwrap(), 2);
    }

    /// When the retries fail too, the cover is missing for now, and asked
    /// for again (once) after `again` while it's still the one wanted.
    #[test]
    fn a_cover_still_missing_is_asked_for_again_later() {
        let count = Arc::new(Mutex::new(0));
        let fetch = Flaky {
            fails: 2,
            count: Arc::clone(&count),
        };
        let retry = Retry {
            again: Duration::from_millis(300),
            ..fast_retry()
        };
        let mut loader = ArtLoader::spawn_with(fetch, None, retry);
        loader.want("https://i.example/a");
        assert_eq!(wait(&loader), ArtState::Missing);
        assert_eq!(*count.lock().unwrap(), 2);
        // Too soon: nothing new.
        loader.want("https://i.example/a");
        thread::sleep(Duration::from_millis(20));
        assert_eq!(*count.lock().unwrap(), 2);
        thread::sleep(Duration::from_millis(350));
        // Each frame asks; one request goes.
        for _ in 0..5 {
            loader.want("https://i.example/a");
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(loader.get(), ArtState::Ready(_)) {
            assert!(Instant::now() < deadline, "never came");
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(*count.lock().unwrap(), 3);
    }

    /// A cover that won't decode isn't fetched again and again.
    #[test]
    fn a_bad_picture_is_not_retried() {
        struct Junk(Arc<Mutex<u32>>);
        impl Fetch for Junk {
            fn get(&mut self, _: &str) -> Result<Vec<u8>, String> {
                *self.0.lock().unwrap() += 1;
                Ok(b"not an image".to_vec())
            }
        }
        let count = Arc::new(Mutex::new(0));
        let mut loader = ArtLoader::spawn_with(Junk(Arc::clone(&count)), None, fast_retry());
        loader.want("https://i.example/a");
        assert_eq!(wait(&loader), ArtState::Missing);
        thread::sleep(Duration::from_millis(50));
        loader.want("https://i.example/a");
        thread::sleep(Duration::from_millis(20));
        assert_eq!(*count.lock().unwrap(), 1);
    }

    /// A newer cover wanted while one waits to retry goes first, and the
    /// old one's answer never shows.
    #[test]
    fn a_newer_cover_interrupts_a_retry_wait() {
        let count = Arc::new(Mutex::new(0));
        let fetch = Flaky {
            fails: 1,
            count: Arc::clone(&count),
        };
        let retry = Retry {
            after: vec![Duration::from_secs(30)],
            again: Duration::from_secs(30),
        };
        let mut loader = ArtLoader::spawn_with(fetch, None, retry);
        loader.want("https://i.example/a");
        let deadline = Instant::now() + Duration::from_secs(5);
        while *count.lock().unwrap() == 0 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        loader.want("https://i.example/b");
        assert!(matches!(wait(&loader), ArtState::Ready(_)));
        assert_eq!(*count.lock().unwrap(), 2);
    }

    #[test]
    fn only_https_is_fetched() {
        let count = Arc::new(Mutex::new(0));
        let mut loader = ArtLoader::spawn(Served(Arc::clone(&count), 9), None);
        for url in ["", "file:///etc/passwd", "http://i.example/a"] {
            loader.want(url);
            assert_eq!(loader.get(), ArtState::Missing, "{url:?}");
        }
        assert_eq!(*count.lock().unwrap(), 0);
    }

    #[test]
    fn stashed_covers_decode_without_fetching_or_the_disk() {
        let dir = TempDir::new("art-stash");
        let count = Arc::new(Mutex::new(0));
        let mut loader = ArtLoader::spawn(Served(Arc::clone(&count), 9), Some(dir.0.clone()));
        let url = stash(png(64, 32)).expect("a picture");
        assert!(url.starts_with(THUMB_SCHEME), "{url}");
        loader.want(&url);
        match wait(&loader) {
            ArtState::Ready(art) => {
                let px = art.scaled(4, 1);
                assert_eq!((px[0], px[3]), (Rgb(255, 0, 0), Rgb(0, 0, 255)));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(*count.lock().unwrap(), 0, "nothing fetched");
        let cached = fs::read_dir(&dir.0).map_or(0, |d| d.count());
        assert_eq!(cached, 0, "nothing cached");

        // Nothing, too much, or forgotten: no cover.
        assert_eq!(stash(Vec::new()), None);
        assert_eq!(stash(vec![0; MAX_BYTES as usize + 1]), None);
        loader.want(&format!("{THUMB_SCHEME}0000"));
        assert_eq!(wait(&loader), ArtState::Missing);
        let junk = stash(b"not an image".to_vec()).unwrap();
        loader.want(&junk);
        assert_eq!(wait(&loader), ArtState::Missing);
    }

    #[test]
    fn the_stash_keeps_only_the_newest() {
        let mine = Stash::default();
        let urls: Vec<String> = (0..STASHED as u8 + 2)
            .map(|i| stash_in(&mine, vec![i; 3]).unwrap())
            .collect();
        assert!(stashed_in(&mine, &urls[0]).is_none());
        assert!(stashed_in(&mine, &urls[2]).is_some());
        // The same picture again is the same URL, kept once, newest.
        assert_eq!(stash_in(&mine, vec![2; 3]).as_ref(), Some(&urls[2]));
        assert_eq!(lock(&mine).len(), STASHED);
        assert_eq!(lock(&mine).last().map(|(u, _)| u), Some(&urls[2]));
    }

    #[test]
    fn file_names_are_safe_and_distinct() {
        let a = file_name("https://i.scdn.co/image/ab67616d");
        assert_eq!(a.len(), 36);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '.'));
        assert_ne!(a, file_name("https://i.scdn.co/image/ab67616e"));
    }
}
