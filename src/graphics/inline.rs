//! Pictures placed at the cursor: sixel and the iTerm2 inline image
//! protocol (iTerm2, WezTerm, foot, mlterm, Konsole, …).
//!
//! Unlike kitty's placeholders these images aren't cells: they're painted
//! over the cells at the cursor, and any text later written into those
//! cells paints over them again. So the 60 fps lamp must never write
//! there while a picture is up, and must write there as soon as it's gone:
//!
//! * The cover draws [`SENTINEL`] cells where the picture goes
//!   ([`draw`]) once [`Inline::shows`] says it's ready for that spot.
//! * After the whole frame is drawn, [`Inline::settle`] looks at those
//!   cells. All still sentinels (nothing drew over them): the picture is
//!   up. The frame it's placed, they become blanks in the cover's mean
//!   colour (written, so the edges the picture doesn't reach match); every
//!   frame after, they're [`CellDiffOption::Skip`]: never rewritten.
//! * When it moves, goes or is covered, its old cells are marked
//!   [`CellDiffOption::AlwaysUpdate`]: whatever is drawn there now is
//!   written over the picture, whatever the last frame held.
//! * [`Inline::write`] then writes the picture itself, after the frame's
//!   cells and inside its synchronized update (cursor saved, moved,
//!   restored): it appears in the same frame its blanks do.
//!
//! A picture is placed once and left alone until its spot, its size, the
//! cover, the window size or the screen (ctrl-l) changes. Leaving the
//! alternate screen takes it away on exit. It's never placed on the
//! screen's last row: a picture reaching the bottom could scroll it.
//!
//! The iTerm2 protocol takes the PNG as is, scaled by the terminal to the
//! cells. Sixel is drawn at its own pixel size, so it needs the cell size
//! in pixels and is encoded off the UI thread (resize, quantise, encode:
//! a few milliseconds), the cover showing as text cells meanwhile.

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::{Key, Protocol, sixel};

/// What the cover draws where its picture will be: never reaches the
/// terminal ([`Inline::settle`] turns every one into a blank or a skip).
pub const SENTINEL: &str = "\u{10EEED}";

/// This frame's wish: `key` (from `png`, base64) at `at`.
#[derive(Debug, Clone)]
pub struct Wish {
    pub protocol: Protocol,
    pub key: Key,
    pub at: Rect,
    pub png: Arc<String>,
    /// A cell's size in pixels, if the terminal says (sixel needs it).
    pub cell: Option<(u16, u16)>,
    /// What shows where a square picture doesn't fill its cells.
    pub bg: [u8; 3],
}

/// An encoded picture: which and how.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Spec {
    protocol: Protocol,
    key: Key,
    /// Pixels per cell (sixel; `(0, 0)` for iTerm2).
    cell: (u16, u16),
}

type Job = (Spec, Receiver<Option<Vec<u8>>>);

/// What the terminal shows and what's on its way.
#[derive(Debug, Default)]
pub struct Inline {
    /// The picture wanted this frame, ready, and where.
    target: Option<(Spec, Rect)>,
    /// The last picture encoded.
    image: Option<(Spec, Arc<Vec<u8>>)>,
    /// One being encoded.
    job: Option<Job>,
    /// One that couldn't be (not tried again).
    failed: Option<Spec>,
    /// On screen now, and where.
    placed: Option<(Spec, Rect)>,
    /// The screen the picture was placed on.
    screen: Rect,
    /// Due after this frame's cells: the picture and where.
    out: Option<(Rect, Arc<Vec<u8>>)>,
}

impl Inline {
    /// This frame's wish (after the layout): the picture to show and
    /// where on `screen`, or none. Starts encoding a new one; until it's
    /// done [`Inline::shows`] says no.
    pub fn want(&mut self, wish: Option<Wish>, screen: Rect) {
        self.poll();
        self.target = None;
        let Some(wish) = wish else {
            return;
        };
        let at = wish.at;
        if at.is_empty() || at.intersection(screen) != at || at.bottom() >= screen.bottom() {
            return;
        }
        let cell = match wish.protocol {
            Protocol::Sixel => match wish.cell {
                Some(c) if c.0 > 0 && c.1 > 0 => c,
                _ => return,
            },
            _ => (0, 0),
        };
        let spec = Spec {
            protocol: wish.protocol,
            key: wish.key.clone(),
            cell,
        };
        if self.image.as_ref().is_some_and(|(s, _)| *s == spec) {
            self.target = Some((spec, at));
            return;
        }
        if self.failed.as_ref() == Some(&spec) || self.job.as_ref().is_some_and(|(s, _)| *s == spec)
        {
            return;
        }
        match spec.protocol {
            Protocol::Iterm => {
                let bytes = iterm(&wish.png, wish.key.cols, wish.key.rows);
                self.image = Some((spec.clone(), Arc::new(bytes)));
                self.target = Some((spec, at));
            }
            Protocol::Sixel => {
                let (tx, rx) = mpsc::channel();
                let png = wish.png.clone();
                let (w, h) = (
                    usize::from(wish.key.cols) * usize::from(cell.0),
                    usize::from(wish.key.rows) * usize::from(cell.1),
                );
                let (bg, crisp) = (wish.bg, wish.key.blocks.is_some());
                let started = std::thread::Builder::new()
                    .name("lavatui-sixel".into())
                    .spawn(move || {
                        crate::thread_qos::worker();
                        let _ = tx.send(sixel_picture(&png, (w, h), bg, crisp));
                    });
                match started {
                    Ok(_) => self.job = Some((spec, rx)),
                    Err(_) => self.failed = Some(spec),
                }
            }
            // Kitty has its own state (`super::Kitty`).
            Protocol::Kitty => {}
        }
    }

    /// Collect a finished encoding, if any.
    fn poll(&mut self) {
        let Some((spec, rx)) = self.job.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Some(bytes)) => self.image = Some((spec, Arc::new(bytes))),
            Ok(None) | Err(TryRecvError::Disconnected) => self.failed = Some(spec),
            Err(TryRecvError::Empty) => self.job = Some((spec, rx)),
        }
    }

    /// Whether `key`'s picture is ready to be shown at `at` this frame
    /// (else the cover draws itself in text cells).
    pub fn shows(&self, key: &Key, at: Rect) -> bool {
        self.target
            .as_ref()
            .is_some_and(|(s, r)| s.key == *key && *r == at)
    }

    /// Whether frames shouldn't sleep: a picture being encoded or due.
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.out.is_some()
    }

    /// The screen was cleared (ctrl-l): place the picture again.
    pub fn invalidate(&mut self) {
        self.placed = None;
    }

    /// After everything is drawn into `buf`, before it's flushed: decide
    /// whether the picture is up, and keep the lamp's redraws off it (or
    /// make them cover where it was).
    pub fn settle(&mut self, buf: &mut Buffer) {
        if buf.area != self.screen {
            // A new size: the whole screen was cleared and is redrawn.
            self.screen = buf.area;
            self.placed = None;
        }
        let target = self
            .target
            .clone()
            .filter(|(_, at)| buf.area.intersection(*at) == *at);
        let intact = target
            .as_ref()
            .is_some_and(|(_, at)| at.positions().all(|p| buf[p].symbol() == SENTINEL));
        if let Some((spec, at)) = self.placed.take() {
            if intact && target.as_ref() == Some(&(spec.clone(), at)) {
                self.placed = Some((spec, at));
            } else {
                for p in at.intersection(buf.area).positions() {
                    buf[p].set_diff_option(CellDiffOption::AlwaysUpdate);
                }
            }
        }
        let Some((spec, at)) = target else {
            return;
        };
        if !intact {
            // Drawn over in part (never expected): no picture, and no
            // sentinel reaches the terminal.
            for p in at.positions() {
                if buf[p].symbol() == SENTINEL {
                    buf[p].set_symbol(" ");
                }
            }
            return;
        }
        if self.placed.is_some() {
            for p in at.positions() {
                buf[p].set_diff_option(CellDiffOption::Skip);
            }
            return;
        }
        for p in at.positions() {
            buf[p]
                .set_symbol(" ")
                .set_diff_option(CellDiffOption::AlwaysUpdate);
        }
        let bytes = self
            .image
            .as_ref()
            .filter(|(s, _)| *s == spec)
            .map(|(_, b)| b.clone());
        if let Some(bytes) = bytes {
            self.out = Some((at, bytes));
            self.placed = Some((spec, at));
        }
    }

    /// Write the picture due this frame, if any: after the frame's cells,
    /// at its spot, the cursor put back after.
    pub fn write(&mut self, out: &mut impl Write) -> io::Result<()> {
        if let Some((at, bytes)) = self.out.take() {
            write!(out, "\x1b7\x1b[{};{}H", at.y + 1, at.x + 1)?;
            out.write_all(&bytes)?;
            out.write_all(b"\x1b8")?;
        }
        Ok(())
    }
}

/// Fill `area` with sentinels on `bg` (the picture's spot; see
/// [`Inline::settle`]).
pub fn draw(buf: &mut Buffer, area: Rect, bg: Color) {
    for p in area.intersection(buf.area).positions() {
        buf[p].set_symbol(SENTINEL).set_fg(bg).set_bg(bg);
    }
}

/// The iTerm2 inline image for `png` (base64) in `cols × rows` cells:
/// square, top left, the cursor left where it was.
fn iterm(png: &str, cols: u16, rows: u16) -> Vec<u8> {
    let pad = png.bytes().rev().take_while(|&b| b == b'=').count();
    let size = png.len() / 4 * 3 - pad.min(2);
    let mut out = format!(
        "\x1b]1337;File=inline=1;size={size};width={cols};height={rows};\
         preserveAspectRatio=1;doNotMoveCursor=1:"
    )
    .into_bytes();
    out.extend_from_slice(png.as_bytes());
    out.push(0x07);
    out
}

/// The sixel image for `png` (base64) at `w × h` pixels: the square cover
/// as big as fits, centred on `bg`; the height rounded down to whole
/// bands so the last one never spills past the cells; pixel art (`crisp`)
/// scaled without blending its blocks' edges. `None` if it can't be
/// decoded.
fn sixel_picture(png: &str, (w, h): (usize, usize), bg: [u8; 3], crisp: bool) -> Option<Vec<u8>> {
    use base64::Engine;
    let h = h / 6 * 6;
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(png).ok()?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgb8();
    let side = w.min(h) as u32;
    let filter = match crisp {
        true => image::imageops::FilterType::Nearest,
        false => image::imageops::FilterType::CatmullRom,
    };
    let square = image::imageops::resize(&image, side, side, filter);
    let (x0, y0) = ((w - side as usize) / 2, (h - side as usize) / 2);
    let mut pixels = vec![bg; w * h];
    for (x, y, p) in square.enumerate_pixels() {
        pixels[(y0 + y as usize) * w + x0 + x as usize] = p.0;
    }
    Some(sixel::encode(&pixels, w, h))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use unicode_width::UnicodeWidthStr;

    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 40, 20);
    const AT: Rect = Rect::new(3, 2, 8, 4);

    fn key(cols: u16, rows: u16) -> Key {
        Key {
            source: "https://i.example/a".into(),
            cols,
            rows,
            blocks: None,
        }
    }

    /// A real 4 × 4 PNG, base64.
    fn png() -> Arc<String> {
        use base64::Engine;
        let img = image::RgbImage::from_fn(4, 4, |x, _| {
            if x < 2 {
                image::Rgb([250, 10, 10])
            } else {
                image::Rgb([10, 10, 250])
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        Arc::new(base64::engine::general_purpose::STANDARD.encode(out.into_inner()))
    }

    fn wish(protocol: Protocol, at: Rect) -> Wish {
        Wish {
            protocol,
            key: key(at.width, at.height),
            at,
            png: png(),
            cell: Some((5, 10)),
            bg: [1, 2, 3],
        }
    }

    /// One frame: the wish, the cover's cells (when it shows), whatever
    /// `over` draws on top, settle; the bytes written after.
    fn frame(
        inline: &mut Inline,
        want: Option<Wish>,
        over: impl Fn(&mut Buffer),
    ) -> (Buffer, Vec<u8>) {
        let mut buf = Buffer::empty(SCREEN);
        if let Some(w) = &want {
            inline.want(Some(w.clone()), SCREEN);
            if inline.shows(&w.key, w.at) {
                draw(&mut buf, w.at, Color::Rgb(1, 2, 3));
            }
        } else {
            inline.want(None, SCREEN);
        }
        over(&mut buf);
        inline.settle(&mut buf);
        let mut out = Vec::new();
        inline.write(&mut out).unwrap();
        (buf, out)
    }

    fn options(buf: &Buffer, r: Rect) -> Vec<CellDiffOption> {
        r.positions().map(|p| buf[p].diff_option).collect()
    }

    fn all(buf: &Buffer, r: Rect, o: CellDiffOption) -> bool {
        options(buf, r).iter().all(|&x| x == o)
    }

    fn wait_for_encoding(inline: &mut Inline, w: &Wish) {
        let start = Instant::now();
        while !inline.shows(&w.key, w.at) {
            assert!(start.elapsed() < Duration::from_secs(10), "never encoded");
            std::thread::sleep(Duration::from_millis(2));
            inline.want(Some(w.clone()), SCREEN);
        }
    }

    #[test]
    fn the_sentinel_is_one_cell_wide() {
        assert_eq!(SENTINEL.width(), 1);
    }

    #[test]
    fn iterm_pictures_are_placed_once_then_skipped() {
        let mut inline = Inline::default();
        let w = wish(Protocol::Iterm, AT);
        let (buf, out) = frame(&mut inline, Some(w.clone()), |_| {});
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.starts_with(
                "\x1b7\x1b[3;4H\x1b]1337;File=inline=1;size=\
                 "
            ),
            "{text:?}"
        );
        assert!(
            text.contains(";width=8;height=4;preserveAspectRatio=1;doNotMoveCursor=1:"),
            "{text:?}"
        );
        assert!(text.ends_with(&format!("{}\x07\x1b8", w.png)), "{text:?}");
        // The placing frame writes blanks in the cover's colour under it.
        assert!(all(&buf, AT, CellDiffOption::AlwaysUpdate));
        assert!(AT.positions().all(|p| buf[p].symbol() == " "));
        assert_eq!(buf[(AT.x, AT.y)].bg, Color::Rgb(1, 2, 3));
        assert!(!inline.busy());
        // Then the cells are left alone, frame after frame.
        for _ in 0..3 {
            let (buf, out) = frame(&mut inline, Some(w.clone()), |_| {});
            assert!(out.is_empty());
            assert!(all(&buf, AT, CellDiffOption::Skip));
            assert!(all(&buf, Rect::new(0, 0, 40, 1), CellDiffOption::None));
        }
    }

    #[test]
    fn the_size_payload_is_the_decoded_png() {
        use base64::Engine;
        let png = png();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png.as_str())
            .unwrap();
        let text = String::from_utf8(iterm(&png, 8, 4)).unwrap();
        assert!(text.contains(&format!("size={};", bytes.len())), "{text}");
        for (b64, n) in [("QUJD", 3), ("QUI=", 2), ("QQ==", 1)] {
            assert!(
                String::from_utf8(iterm(b64, 1, 1))
                    .unwrap()
                    .contains(&format!("size={n};"))
            );
        }
    }

    #[test]
    fn moving_hiding_or_covering_repaints_where_it_was() {
        let mut inline = Inline::default();
        frame(&mut inline, Some(wish(Protocol::Iterm, AT)), |_| {});
        // Moved: the old spot is rewritten, the new one placed.
        let moved = Rect::new(20, 5, 8, 4);
        let (buf, out) = frame(&mut inline, Some(wish(Protocol::Iterm, moved)), |_| {});
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("\x1b7\x1b[6;21H")
        );
        assert!(all(&buf, AT, CellDiffOption::AlwaysUpdate));
        // Covered (an overlay drew over its cells): not shown, repainted.
        let (buf, out) = frame(&mut inline, Some(wish(Protocol::Iterm, moved)), |b| {
            b.set_string(21, 6, "help", ratatui::style::Style::new());
        });
        assert!(out.is_empty());
        assert!(all(&buf, moved, CellDiffOption::AlwaysUpdate));
        assert!(moved.positions().all(|p| buf[p].symbol() != SENTINEL));
        // Uncovered: placed again.
        let (_, out) = frame(&mut inline, Some(wish(Protocol::Iterm, moved)), |_| {});
        assert!(!out.is_empty());
        // Hidden: its cells are rewritten, nothing more is sent.
        let (buf, out) = frame(&mut inline, None, |_| {});
        assert!(out.is_empty());
        assert!(all(&buf, moved, CellDiffOption::AlwaysUpdate));
        let (buf, _) = frame(&mut inline, None, |_| {});
        assert!(all(&buf, moved, CellDiffOption::None), "only once");
    }

    #[test]
    fn a_cleared_or_resized_screen_gets_it_again() {
        let mut inline = Inline::default();
        let w = wish(Protocol::Iterm, AT);
        frame(&mut inline, Some(w.clone()), |_| {});
        inline.invalidate();
        let (_, out) = frame(&mut inline, Some(w.clone()), |_| {});
        assert!(!out.is_empty(), "ctrl-l");
        // Another window size: the screen was cleared.
        inline.want(Some(w.clone()), Rect::new(0, 0, 50, 20));
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 20));
        draw(&mut buf, AT, Color::Black);
        inline.settle(&mut buf);
        let mut out = Vec::new();
        inline.write(&mut out).unwrap();
        assert!(!out.is_empty(), "resized");
    }

    #[test]
    fn never_on_the_last_row() {
        let mut inline = Inline::default();
        let low = Rect::new(3, 16, 8, 4);
        inline.want(Some(wish(Protocol::Iterm, low)), SCREEN);
        assert!(!inline.shows(&key(8, 4), low));
        let fits = Rect::new(3, 15, 8, 4);
        inline.want(Some(wish(Protocol::Iterm, fits)), SCREEN);
        assert!(inline.shows(&key(8, 4), fits));
    }

    #[test]
    fn sixel_is_encoded_off_thread_at_the_cells_pixel_size() {
        let mut inline = Inline::default();
        let w = wish(Protocol::Sixel, AT);
        inline.want(Some(w.clone()), SCREEN);
        assert!(!inline.shows(&w.key, AT), "not yet");
        assert!(inline.busy());
        wait_for_encoding(&mut inline, &w);
        let (_, out) = frame(&mut inline, Some(w.clone()), |_| {});
        let text = String::from_utf8(out).unwrap();
        let body = text
            .strip_prefix("\x1b7\x1b[3;4H")
            .and_then(|t| t.strip_suffix("\x1b8"))
            .expect("positioned, cursor restored");
        // 8 × 4 cells of 5 × 10 pixels: 40 × 40, the square cover filling it.
        let (pw, ph, pixels) = sixel::tests::decode(body.as_bytes());
        assert_eq!((pw, ph), (40, 36), "height in whole bands");
        let left = pixels[10 * 40 + 2].unwrap();
        let right = pixels[10 * 40 + 37].unwrap();
        assert!(left[0] > 200 && left[2] < 60, "{left:?}");
        assert!(right[2] > 200 && right[0] < 60, "{right:?}");
        // Another cell size is another picture.
        let mut bigger = w.clone();
        bigger.cell = Some((10, 20));
        inline.want(Some(bigger.clone()), SCREEN);
        assert!(!inline.shows(&w.key, AT));
        wait_for_encoding(&mut inline, &bigger);
    }

    #[test]
    fn sixel_needs_the_cell_size_and_a_decodable_picture() {
        let mut inline = Inline::default();
        let mut w = wish(Protocol::Sixel, AT);
        w.cell = None;
        inline.want(Some(w.clone()), SCREEN);
        assert!(!inline.busy(), "nothing to size it by");
        w.cell = Some((5, 10));
        w.png = Arc::new("not a png".into());
        inline.want(Some(w.clone()), SCREEN);
        let start = Instant::now();
        while inline.busy() {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(2));
            inline.want(Some(w.clone()), SCREEN);
        }
        assert!(!inline.shows(&w.key, AT));
        inline.want(Some(w.clone()), SCREEN);
        assert!(!inline.busy(), "not tried again");
    }

    #[test]
    fn letterboxed_on_the_background() {
        let bytes = sixel_picture(&png(), (30, 12), [0, 255, 0], false).unwrap();
        let (w, h, pixels) = sixel::tests::decode(&bytes);
        assert_eq!((w, h), (30, 12));
        let edge = pixels[6 * 30].unwrap();
        assert!(edge[1] > 200 && edge[0] < 40, "{edge:?}");
        assert!(
            pixels[6 * 30 + 15].unwrap()[1] < 100,
            "the cover in the middle"
        );
        assert!(
            sixel_picture(&png(), (30, 5), [0; 3], false).is_none(),
            "under a band"
        );
    }
}
