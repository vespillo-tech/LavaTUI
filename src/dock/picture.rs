//! A cover in text cells: each cell shows a few of the picture's pixels
//! with two colours and a block glyph.
//!
//! | mode | pixels a cell | glyphs |
//! |---|---|---|
//! | half block | 1 × 2 | `▀` (exact: top ink, bottom paper) |
//! | quadrant | 2 × 2 | `▘▝▀▖▌▞▛▗▚▐▜▄▙▟█` |
//! | sextant | 2 × 3 | U+1FB00..U+1FB3B (Unicode 13), `▌▐█` |
//!
//! Quadrants and sextants split each cell's pixels in the two groups whose
//! means lose the least (every split is tried: 8 or 32), one mean the
//! glyph's ink, the other its background. Colours go through
//! [`Theme::image`]: exact in truecolor, the nearest xterm index in 256.
//! The cells for a cover at a size are worked out once and kept (covers
//! don't change frame to frame).
//!
//! Opacity-safe, as the lamp's half blocks (`render::cell::half_block`):
//! where a terminal shows cell backgrounds see-through but glyphs opaque
//! (Ghostty's `background-opacity-cells`), a background colour shows
//! darker than an ink one. So a cell whose two colours look the same (or
//! that has only one) is a `█` in their mean, and with `translucent` (the
//! terminal is known to do this) every cell is: one colour a cell, but no
//! streaks.

use std::cell::RefCell;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::media::art::Art;
use crate::render::{quadrant, sextant};
use crate::theme::{NEAR, Rgb, Theme};

/// How a cover is drawn in text cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextMode {
    HalfBlock,
    Quadrant,
    Sextant,
}

impl TextMode {
    /// Pixels a cell, across and down.
    fn grid(self) -> (usize, usize) {
        match self {
            TextMode::HalfBlock => (1, 2),
            TextMode::Quadrant => (2, 2),
            TextMode::Sextant => (2, 3),
        }
    }
}

/// One cell of a picture: the glyph, its ink and its background.
type PictureCell = (char, Color, Color);

/// The cells last worked out, by what they were worked out for.
#[derive(Default)]
struct Cache {
    key: Option<(String, u16, u16, TextMode, u8, bool)>,
    cells: Vec<PictureCell>,
}

thread_local! {
    /// Drawing runs on one thread.
    static CACHE: RefCell<Cache> = RefCell::default();
}

/// Draw `art` (the cover at `source`) into `area` in `mode`; for
/// see-through cell backgrounds if `translucent`.
pub fn draw(
    buf: &mut Buffer,
    area: Rect,
    art: &Art,
    source: &str,
    (mode, translucent): (TextMode, bool),
    theme: &Theme,
) {
    let area = area.intersection(buf.area);
    if area.is_empty() || !theme.shows_images() {
        return;
    }
    let key = (
        source.to_owned(),
        area.width,
        area.height,
        mode,
        theme.depth() as u8,
        translucent,
    );
    CACHE.with_borrow_mut(|cache| {
        if cache.key.as_ref() != Some(&key) {
            cache.cells = cells(art, area.width, area.height, (mode, translucent), theme);
            cache.key = Some(key);
        }
        let w = usize::from(area.width);
        for (i, &(ch, fg, bg)) in cache.cells.iter().enumerate() {
            let (x, y) = ((i % w) as u16, (i / w) as u16);
            buf[(area.x + x, area.y + y)]
                .set_char(ch)
                .set_fg(fg)
                .set_bg(bg);
        }
    });
}

/// The cells of `art` at `cols × rows` in `mode` (pure).
pub fn cells(
    art: &Art,
    cols: u16,
    rows: u16,
    (mode, translucent): (TextMode, bool),
    theme: &Theme,
) -> Vec<PictureCell> {
    let (gw, gh) = mode.grid();
    let (w, h) = (usize::from(cols), usize::from(rows));
    let px = art.scaled(cols * gw as u16, rows * gh as u16);
    let color = |c: Rgb| theme.image(c).unwrap_or(Color::Reset);
    let mut out = Vec::with_capacity(w * h);
    let mut block = [Rgb::default(); 6];
    for row in 0..h {
        for col in 0..w {
            for dy in 0..gh {
                for dx in 0..gw {
                    let i = (row * gh + dy) * w * gw + col * gw + dx;
                    block[dy * gw + dx] = px[i];
                }
            }
            let pixels = &block[..gw * gh];
            let (ch, fg, bg, mask) = match mode {
                TextMode::HalfBlock => ('▀', pixels[0], pixels[1], 1),
                TextMode::Quadrant => {
                    let (mask, ink, paper) = split(pixels);
                    (quadrant(mask), ink, paper, mask)
                }
                TextMode::Sextant => {
                    let (mask, ink, paper) = split(pixels);
                    (sextant(mask), ink, paper, mask)
                }
            };
            let alike = |a: Rgb, b: Rgb| {
                let rgb = |c: Rgb| Color::Rgb(c.0, c.1, c.2);
                theme.merge(rgb(a), rgb(b), NEAR).is_some()
            };
            if translucent || mask == 0 || alike(fg, bg) {
                let c = color(mean(pixels));
                out.push(('█', c, c));
            } else {
                out.push((ch, color(fg), color(bg)));
            }
        }
    }
    out
}

/// The mean colour of `pixels`.
fn mean(pixels: &[Rgb]) -> Rgb {
    let n = pixels.len() as u32;
    let sum = |f: fn(&Rgb) -> u8| {
        let total: u32 = pixels.iter().map(|p| u32::from(f(p))).sum();
        ((total + n / 2) / n) as u8
    };
    Rgb(sum(|p| p.0), sum(|p| p.1), sum(|p| p.2))
}

/// The best two-colour split of a cell's pixels: the mask of the ink
/// pixels (bit `i` for pixel `i`, left to right, top to bottom), the ink
/// and the paper. The last pixel is always paper (each split once).
fn split(pixels: &[Rgb]) -> (u8, Rgb, Rgb) {
    let n = pixels.len();
    let mut best = (u32::MAX, 0u8, Rgb::default(), Rgb::default());
    for mask in 0..(1u8 << (n - 1)) {
        let mut sums = [[0u32; 4]; 2];
        for (i, p) in pixels.iter().enumerate() {
            let s = &mut sums[usize::from(mask >> i & 1)];
            s[0] += u32::from(p.0);
            s[1] += u32::from(p.1);
            s[2] += u32::from(p.2);
            s[3] += 1;
        }
        let mean = |s: [u32; 4]| {
            let n = s[3].max(1);
            let avg = |v: u32| ((v + n / 2) / n) as u8;
            Rgb(avg(s[0]), avg(s[1]), avg(s[2]))
        };
        let (paper, ink) = (mean(sums[0]), mean(sums[1]));
        let err: u32 = pixels
            .iter()
            .enumerate()
            .map(|(i, p)| distance(*p, if mask >> i & 1 == 1 { ink } else { paper }))
            .sum();
        if err < best.0 {
            best = (err, mask, ink, paper);
        }
    }
    (best.1, best.2, best.3)
}

fn distance(a: Rgb, b: Rgb) -> u32 {
    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2) as u32;
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ColorDepth, Palette};

    fn theme(depth: ColorDepth) -> Theme {
        Theme::new(Palette::by_name("lava").unwrap(), depth)
    }

    #[test]
    fn sextant_glyphs_follow_unicode() {
        assert_eq!(sextant(1), '\u{1FB00}', "BLOCK SEXTANT-1");
        assert_eq!(sextant(3), '\u{1FB02}', "BLOCK SEXTANT-12");
        assert_eq!(sextant(20), '\u{1FB13}', "BLOCK SEXTANT-35");
        assert_eq!(sextant(22), '\u{1FB14}', "BLOCK SEXTANT-235");
        assert_eq!(sextant(62), '\u{1FB3B}', "BLOCK SEXTANT-23456");
        assert_eq!((sextant(21), sextant(42), sextant(63)), ('▌', '▐', '█'));
        let all: std::collections::HashSet<char> = (0..64).map(sextant).collect();
        assert_eq!(all.len(), 64);
    }

    #[test]
    fn a_split_finds_the_two_colours() {
        let (r, b) = (Rgb(255, 0, 0), Rgb(0, 0, 255));
        // Left column red, right column blue (sextant ▌ shape).
        let (mask, ink, paper) = split(&[r, b, r, b, r, b]);
        assert_eq!((mask, ink, paper), (21, r, b));
        // All one colour: a blank cell on that colour.
        let (mask, _, paper) = split(&[r; 4]);
        assert_eq!((mask, paper), (0, r));
    }

    #[test]
    fn every_mode_fills_the_area_with_the_picture() {
        let art = Art::solid(Rgb(200, 120, 40));
        for mode in [TextMode::HalfBlock, TextMode::Quadrant, TextMode::Sextant] {
            for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
                let t = theme(depth);
                let cells = cells(&art, 7, 3, (mode, false), &t);
                assert_eq!(cells.len(), 21);
                let want = t.image(Rgb(200, 120, 40)).unwrap();
                assert!(cells.iter().all(|&(_, _, bg)| bg == want), "{mode:?}");
            }
        }
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        let t = theme(ColorDepth::Ansi16);
        let area = buf.area;
        draw(&mut buf, area, &art, "x", (TextMode::Sextant, false), &t);
        assert_eq!(buf, Buffer::empty(buf.area), "no pictures in 16 colours");
    }

    /// Opacity-safe: a smooth gradient is whole `█` cells (no colour in a
    /// see-through background), a hard edge still splits, and drawn for
    /// translucent backgrounds nothing does.
    /// Each mode samples the cover at its own grid, so the finer one shows
    /// detail the coarser can't: on a 32-col cover (the 128 px art is
    /// plenty: 64 × 48 samples in fine), a pattern of thin rings comes
    /// out as more distinct cells the finer the mode.
    #[test]
    fn finer_modes_show_more_detail() {
        let t = theme(ColorDepth::TrueColor);
        let art = Art::from_fn(|x, y| {
            let (dx, dy) = (x as i32 - 64, y as i32 - 64);
            let ring = ((dx * dx + dy * dy) as f64).sqrt() as i32 / 3 % 2 == 0;
            if ring {
                Rgb(240, 200, 60)
            } else {
                Rgb(30, 20, 80)
            }
        });
        let distinct = |mode| {
            let cells = cells(&art, 32, 16, (mode, false), &t);
            let mut glyphs: Vec<char> = cells.iter().map(|c| c.0).collect();
            glyphs.sort_unstable();
            glyphs.dedup();
            glyphs.len()
        };
        let (coarse, medium, fine) = (
            distinct(TextMode::HalfBlock),
            distinct(TextMode::Quadrant),
            distinct(TextMode::Sextant),
        );
        assert!(coarse < medium && medium < fine, "{coarse} {medium} {fine}");
    }

    #[test]
    fn alike_colours_and_translucent_cells_are_whole_blocks() {
        let t = theme(ColorDepth::TrueColor);
        let gradient = Art::from_fn(|_, y| Rgb(120 + (y / 16) as u8, 60, 40));
        let stripes = Art::from_fn(|x, _| match x % 2 == 0 {
            true => Rgb(250, 240, 230),
            false => Rgb(10, 20, 60),
        });
        let whole = |cells: &[PictureCell]| cells.iter().all(|&(ch, fg, bg)| ch == '█' && fg == bg);
        for mode in [TextMode::HalfBlock, TextMode::Quadrant, TextMode::Sextant] {
            assert!(
                whole(&cells(&gradient, 12, 6, (mode, false), &t)),
                "{mode:?}"
            );
            assert!(whole(&cells(&stripes, 12, 6, (mode, true), &t)), "{mode:?}");
        }
        // One-pixel stripes at full size: each quadrant cell splits ▌ / ▐.
        let split = cells(&stripes, 64, 4, (TextMode::Quadrant, false), &t);
        assert!(split.iter().any(|&(ch, fg, bg)| ch != '█' && fg != bg));
    }
}
