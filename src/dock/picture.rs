//! A cover in text cells: sharp, each cell showing a few of the picture's
//! pixels with two colours and a block glyph, or pixel art, flat square
//! blocks of whole cells and half cells.
//!
//! | mode | pixels a cell | glyphs |
//! |---|---|---|
//! | quadrant | 2 × 2 | `▘▝▀▖▌▞▛▗▚▐▜▄▙▟█` |
//! | sextant | 2 × 3 | U+1FB00..U+1FB3B (Unicode 13), `▌▐█` |
//! | pixels (small, medium, big) | blocks of k columns × k half rows | `▀` (exact: top ink, bottom paper), `█` |
//!
//! Pixel-art blocks are about [`PIXEL_ART`] across (32 small, 16 medium,
//! 10 big), a whole number of columns wide, at least two
//! ([`block_side`]), so every
//! block is the same size give or take one; each size's blocks are always
//! bigger than the one before's, however small the cover. A picture in
//! pixels (kitty, iTerm2, sixel) has the same blocks ([`pixel_grid`]), so
//! nothing moves when it takes over from the text cells.
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
//! streaks. There pixel-art blocks are whole cells (an even side), so
//! they keep their two colours a cell apart.

use std::cell::RefCell;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::media::art::Art;
use crate::render::{quadrant, sextant};
use crate::theme::{NEAR, Rgb, Theme};

/// Pixel art's blocks across, about (the cover quality's small, medium
/// and big pixels); fewer where the cover is too small for blocks of two
/// columns.
pub const PIXEL_ART: [u16; 3] = [32, 16, 10];

/// How a cover is drawn in text cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextMode {
    /// Sharp, 2 × 2 pixels a cell.
    Quadrant,
    /// Sharp, 2 × 3 pixels a cell (terminals that draw sextants).
    Sextant,
    /// Pixel art, about `PIXEL_ART[level]` blocks across.
    Pixels(usize),
}

/// The side of a pixel-art block in `mode` on a cover `cols` wide: in
/// columns across and half rows down (about square on screen); at least
/// two (one column is as fine as text gets: it would look sharp); even,
/// so a block is whole cells, where cell backgrounds are see-through;
/// always bigger than the level before's. `None` for the sharp modes.
pub fn block_side(mode: TextMode, cols: u16, translucent: bool) -> Option<usize> {
    let TextMode::Pixels(level) = mode else {
        return None;
    };
    let unit = if translucent { 2 } else { 1 };
    let side = |across: u16| {
        let k = (f64::from(cols) / f64::from(across)).round() as usize;
        k.max(2).next_multiple_of(unit)
    };
    let sides = PIXEL_ART[..=level.min(PIXEL_ART.len() - 1)].iter();
    sides.fold(None, |before: Option<usize>, &across| {
        Some(before.map_or(side(across), |b| side(across).max(b + unit)))
    })
}

/// Pixel art in `mode` on a cover `cols × rows`: its blocks across and
/// down. The text cells and the picture in pixels both use these. `None`
/// for the sharp modes.
pub fn pixel_grid(mode: TextMode, cols: u16, rows: u16, translucent: bool) -> Option<(u16, u16)> {
    let side = block_side(mode, cols, translucent)?;
    let (nx, ny) = grid((cols, rows), (side, translucent));
    Some((nx as u16, ny as u16))
}

/// Blocks across and down on `cols × rows` cells for blocks `side`
/// columns × `side` half rows (whole rows where `translucent`).
fn grid((cols, rows): (u16, u16), (side, translucent): (usize, bool)) -> (usize, usize) {
    let unit = if translucent { 2 } else { 1 };
    let down = 2 * usize::from(rows) / unit;
    let count = |len: usize, side: usize| ((len + side / 2) / side).max(1);
    (count(usize::from(cols), side), count(down, side / unit))
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
    let rest = (
        area.width,
        area.height,
        mode,
        theme.depth() as u8,
        translucent,
    );
    CACHE.with_borrow_mut(|cache| {
        // Compared before anything is copied: most frames it's the same.
        let same = cache
            .key
            .as_ref()
            .is_some_and(|(s, w, h, m, d, t)| s == source && (*w, *h, *m, *d, *t) == rest);
        if !same {
            cache.cells = cells(art, area.width, area.height, (mode, translucent), theme);
            let (w, h, m, d, t) = rest;
            cache.key = Some((source.to_owned(), w, h, m, d, t));
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
    let (gw, gh) = match mode {
        TextMode::Quadrant => (2, 2),
        TextMode::Sextant => (2, 3),
        TextMode::Pixels(_) => {
            let side = block_side(mode, cols, translucent).unwrap_or(1);
            return blocks(art, (cols, rows), (side, translucent), theme);
        }
    };
    let (w, h) = (usize::from(cols), usize::from(rows));
    let px = art.scaled(cols * gw as u16, rows * gh as u16);
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
            let (mask, ink, paper) = split(pixels);
            let ch = match mode {
                TextMode::Sextant => sextant(mask),
                _ => quadrant(mask),
            };
            out.push(cell(pixels, (ch, mask, ink, paper), translucent, theme));
        }
    }
    out
}

/// One cell: `ch` in `ink` on `paper` (`mask` its ink pixels), or a
/// whole `█` in the mean of `pixels` where the two look alike or
/// backgrounds are see-through.
fn cell(
    pixels: &[Rgb],
    (ch, mask, ink, paper): (char, u8, Rgb, Rgb),
    translucent: bool,
    theme: &Theme,
) -> PictureCell {
    let color = |c: Rgb| theme.image(c).unwrap_or(Color::Reset);
    let rgb = |c: Rgb| Color::Rgb(c.0, c.1, c.2);
    if translucent || mask == 0 || theme.merge(rgb(ink), rgb(paper), NEAR).is_some() {
        let c = color(mean(pixels));
        ('█', c, c)
    } else {
        (ch, color(ink), color(paper))
    }
}

/// Pixel art: `art` in flat square blocks `side` columns × `side` half
/// rows (whole rows where `translucent`), as many as fit the area, each
/// the mean of what it covers; block edges spread evenly.
fn blocks(
    art: &Art,
    (cols, rows): (u16, u16),
    (side, translucent): (usize, bool),
    theme: &Theme,
) -> Vec<PictureCell> {
    let (w, h) = (usize::from(cols), usize::from(rows));
    // Down, in half rows, or in rows where a cell can show one colour.
    let unit = if translucent { 2 } else { 1 };
    let down = 2 * h / unit;
    let (nx, ny) = grid((cols, rows), (side, translucent));
    let px = art.scaled(nx as u16, ny as u16);
    let at = |x: usize, y: usize| px[(y * ny / down) * nx + x * nx / w];
    let mut out = Vec::with_capacity(w * h);
    for row in 0..h {
        for col in 0..w {
            let pixels = if translucent {
                [at(col, row); 2]
            } else {
                [at(col, 2 * row), at(col, 2 * row + 1)]
            };
            let mask = u8::from(pixels[0] != pixels[1]);
            out.push(cell(
                &pixels,
                ('▀', mask, pixels[0], pixels[1]),
                false,
                theme,
            ));
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

    const MODES: [TextMode; 5] = [
        TextMode::Sextant,
        TextMode::Quadrant,
        TextMode::Pixels(0),
        TextMode::Pixels(1),
        TextMode::Pixels(2),
    ];

    #[test]
    fn every_mode_fills_the_area_with_the_picture() {
        let art = Art::solid(Rgb(200, 120, 40));
        for mode in MODES {
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

    /// Rings: thin enough that every coarser mode loses some.
    fn rings() -> Art {
        Art::from_fn(|x, y| {
            let (dx, dy) = (x as i32 - 64, y as i32 - 64);
            let ring = ((dx * dx + dy * dy) as f64).sqrt() as i32 / 3 % 2 == 0;
            if ring {
                Rgb(240, 200, 60)
            } else {
                Rgb(30, 20, 80)
            }
        })
    }

    /// Each mode samples the cover at its own grid, so the finer one shows
    /// detail the coarser can't: on a 32-col cover (the 128 px art is
    /// plenty: 64 × 48 samples in sextants), a pattern of thin rings comes
    /// out as more distinct cells in sextants than quadrants, and as fewer
    /// edges between cells the bigger the pixel-art blocks.
    #[test]
    fn finer_modes_show_more_detail() {
        let t = theme(ColorDepth::TrueColor);
        let art = rings();
        let cells = |mode| cells(&art, 32, 16, (mode, false), &t);
        let distinct = |mode| {
            let mut cells = cells(mode);
            cells.sort_unstable_by_key(|c| format!("{c:?}"));
            cells.dedup();
            cells.len()
        };
        assert!(distinct(TextMode::Sextant) > distinct(TextMode::Quadrant));
        let edges = |mode| {
            let cells = cells(mode);
            let across = cells
                .chunks(32)
                .flat_map(|r| r.windows(2))
                .filter(|p| p[0] != p[1]);
            let down = (32..cells.len()).filter(|&i| cells[i] != cells[i - 32]);
            across.count() + down.count()
        };
        // Sharp ones: nearly every cell differs from the next.
        let [sextant, quadrant, small, medium, big] = MODES.map(edges);
        assert!(
            sextant.min(quadrant) > medium,
            "{sextant} {quadrant} {medium}"
        );
        assert!(small > medium && medium > big, "{small} {medium} {big}");
    }

    /// lava-jop: the text cells show exactly `pixel_grid`'s blocks, the
    /// ones a picture in pixels is made with.
    #[test]
    fn text_cells_show_the_pixel_grid() {
        let t = theme(ColorDepth::TrueColor);
        // A different colour in every block, across and down.
        let art = Art::from_fn(|x, y| Rgb((x * 2) as u8, (y * 2) as u8, 0));
        let runs = |colors: &[Color]| 1 + colors.windows(2).filter(|p| p[0] != p[1]).count();
        for translucent in [false, true] {
            for cols in [10, 16, 24, 34, 64] {
                let rows = cols / 2;
                for level in 0..3 {
                    let mode = TextMode::Pixels(level);
                    let (nx, ny) = pixel_grid(mode, cols, rows, translucent).unwrap();
                    let cells = cells(&art, cols, rows, (mode, translucent), &t);
                    let top: Vec<Color> = cells[..usize::from(cols)].iter().map(|c| c.1).collect();
                    // Down the first column, in half rows (whole rows where
                    // every cell is one colour).
                    let left: Vec<Color> = cells
                        .chunks(usize::from(cols))
                        .flat_map(|row| {
                            let (ch, fg, bg) = row[0];
                            let bottom = if ch == '▀' { bg } else { fg };
                            if translucent {
                                vec![fg]
                            } else {
                                vec![fg, bottom]
                            }
                        })
                        .collect();
                    let at = format!("{cols}x{rows} level {level} {translucent}");
                    assert_eq!(runs(&top), usize::from(nx), "{at}");
                    assert_eq!(runs(&left), usize::from(ny), "{at}");
                }
            }
        }
    }

    /// Pixel art: about 32, 16 and 10 blocks across, whole columns, each
    /// size always bigger than the one before; whole cells where
    /// backgrounds are see-through.
    #[test]
    fn pixel_art_blocks_are_whole_cells_and_get_bigger() {
        use TextMode::Pixels;
        assert_eq!(block_side(TextMode::Sextant, 24, false), None);
        for translucent in [false, true] {
            for cols in 4..=64 {
                let sides = [0, 1, 2].map(|l| block_side(Pixels(l), cols, translucent).unwrap());
                assert!(
                    sides[0] < sides[1] && sides[1] < sides[2],
                    "{cols} {translucent}: {sides:?}"
                );
                assert!(!translucent || sides.iter().all(|s| s.is_multiple_of(2)));
            }
        }
        let across = |cols| {
            [0, 1, 2].map(|l| usize::from(cols) / block_side(Pixels(l), cols, false).unwrap())
        };
        assert_eq!(across(64), [32, 16, 10]);
        assert_eq!(across(24), [12, 8, 6]);
        // Blocks are flat: a cell row's colours change only at block edges.
        let t = theme(ColorDepth::TrueColor);
        let art = rings();
        for (level, side) in [(0, 2), (1, 3), (2, 4)] {
            let cells = cells(&art, 24, 12, (Pixels(level), false), &t);
            assert_eq!(block_side(Pixels(level), 24, false), Some(side));
            for row in cells.chunks(24) {
                for (x, pair) in row.windows(2).enumerate() {
                    assert!(pair[0] == pair[1] || (x + 1) % side == 0, "{level} col {x}");
                }
            }
        }
        // See-through backgrounds: whole cells, one colour each, still in
        // blocks two, four and six columns wide.
        for (level, side) in [(0, 2), (1, 4), (2, 6)] {
            let cells = cells(&art, 24, 12, (Pixels(level), true), &t);
            assert!(cells.iter().all(|&(ch, fg, bg)| ch == '█' && fg == bg));
            for row in cells.chunks(24) {
                assert!(row.chunks(side).all(|b| b.iter().all(|c| *c == b[0])));
            }
        }
    }

    /// Opacity-safe: a smooth gradient is whole `█` cells (no colour in a
    /// see-through background), a hard edge still splits, and drawn for
    /// translucent backgrounds nothing does.
    #[test]
    fn alike_colours_and_translucent_cells_are_whole_blocks() {
        let t = theme(ColorDepth::TrueColor);
        let gradient = Art::from_fn(|_, y| Rgb(120 + (y / 16) as u8, 60, 40));
        let stripes = Art::from_fn(|x, _| match x % 2 == 0 {
            true => Rgb(250, 240, 230),
            false => Rgb(10, 20, 60),
        });
        let whole = |cells: &[PictureCell]| cells.iter().all(|&(ch, fg, bg)| ch == '█' && fg == bg);
        for mode in MODES {
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
