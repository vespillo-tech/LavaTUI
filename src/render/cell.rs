//! Glyph helpers shared by styles.

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use crate::theme::{NEAR, Theme};

/// One pixel of a half-block canvas, as a style paints it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pixel {
    /// No wax: the liquid shows.
    Liquid,
    /// Behind the wax in a colour of its own (a style's backdrop: a sky,
    /// a glow, a floor; or the liquid as a style blends it). Drawn like
    /// the liquid, as a cell background where it can be.
    Back(Color),
    /// Wax, or anything else in front. May be `Color::Reset` (terminal
    /// foreground) when there's no colour.
    Ink(Color),
}

impl Pixel {
    /// The colour shown, and whether it's behind the wax.
    #[inline]
    fn shown(self, base: Color) -> (Color, bool) {
        match self {
            Pixel::Liquid => (base, true),
            Pixel::Back(c) => (c, true),
            Pixel::Ink(c) => (c, false),
        }
    }
}

/// A drawn cell: glyph, foreground, background.
pub type Drawn = (char, Color, Color);

/// With `translucent`, backdrop halves closer than this (OKLab ΔE) merge.
const BACK_STEP: f32 = 0.1;

/// Two vertically stacked pixels as one cell, on `base`, the liquid's
/// colour: its glyph, foreground and background. Picks `▀ ▄ █` or a
/// space; the liquid is only ever a background, so it never needs a
/// colour that can't be a foreground.
///
/// Drawn to survive terminals that make cell backgrounds translucent but
/// keep glyphs opaque (Ghostty's `background-opacity-cells`), where a
/// cell's background half shows darker than its foreground half:
///
/// - Halves that look the same ([`Theme::merge`] within [`NEAR`]) are one
///   colour: a `█`, or a space when it's behind the wax. On an opaque
///   terminal that looks no different (neither pixel moves more than half
///   of [`NEAR`]), and no seam can show inside smooth wax.
/// - What's behind the wax is the background: a space, or the background
///   half of a split cell, so it's as see-through as the window
///   everywhere and wax edges don't change its shade.
/// - Two visibly different halves split, the one behind (else the darker:
///   least changed over a dark window) as the background. With
///   `translucent` (the terminal is known to see through backgrounds) two
///   wax halves never split: they become their mean, trading vertical
///   colour detail inside the wax (the silhouette keeps it) for no seams.
///   Two backdrop halves merge up to [`BACK_STEP`] apart (a gradient's
///   steps), so only lines drawn on the backdrop show opaque.
#[inline]
pub fn half_block(
    top: Pixel,
    bottom: Pixel,
    base: Color,
    theme: &Theme,
    translucent: bool,
) -> Drawn {
    let ((t, t_back), (b, b_back)) = (top.shown(base), bottom.shown(base));
    let within = match (translucent, t_back, b_back) {
        (true, false, false) => f32::INFINITY,
        (true, true, true) => BACK_STEP,
        _ => NEAR,
    };
    // Without colour, wax and liquid are both the terminal's default: only
    // the glyph tells them apart.
    let merged = match t_back != b_back && t == Color::Reset {
        true => None,
        false => theme.merge(t, b, within),
    };
    match merged {
        Some(c) if t_back || b_back => (' ', Color::Reset, c),
        Some(c) => ('█', c, base),
        None if bottom == Pixel::Liquid || (b_back && !t_back) => ('▀', t, b),
        None if top == Pixel::Liquid || (t_back && !b_back) || theme.darker(t, b) => ('▄', b, t),
        None => ('▀', t, b),
    }
}

/// Draws cells with [`half_block`], remembering the last pair of pixels:
/// neighbouring cells mostly repeat it (a sky row, a band of wax), and
/// telling whether two colours look the same isn't free.
pub struct HalfBlocks<'t> {
    theme: &'t Theme,
    base: Color,
    translucent: bool,
    last: Option<((Pixel, Pixel), Drawn)>,
}

impl<'t> HalfBlocks<'t> {
    pub fn new(theme: &'t Theme, base: Color, translucent: bool) -> Self {
        Self {
            theme,
            base,
            translucent,
            last: None,
        }
    }

    /// Draw `top` over `bottom` into `cell`.
    #[inline]
    pub fn draw(&mut self, cell: &mut Cell, top: Pixel, bottom: Pixel) {
        let (ch, fg, bg) = match self.last {
            Some((pair, drawn)) if pair == (top, bottom) => drawn,
            _ => {
                let drawn = half_block(top, bottom, self.base, self.theme, self.translucent);
                self.last = Some(((top, bottom), drawn));
                drawn
            }
        };
        cell.set_char(ch).set_fg(fg).set_bg(bg);
    }
}

/// Redraw the block glyphs in `area` for terminals whose block glyphs stop
/// short of the cell's top (macOS Terminal leaves the top sixth of every
/// cell to the background): a wax-coloured `█` there shows a dark line
/// along each row. Each block is turned, where it can be, so the colour
/// along its top edge is the cell's background, which fills the cell:
/// `█` becomes a space on its colour, `▀` a `▄` in swapped colours, and
/// quadrants and sextants their complement when ink holds more of the top
/// row (on a tie, more of the cell). `▓` becomes `░`. Every cell looks the
/// same on a terminal that does fill its blocks.
///
/// Left alone: cells whose colours can't swap (`Color::Reset`, the
/// terminal's own colours, means one thing as ink and another as
/// background) and reversed cells.
pub fn fill_from_background(buf: &mut Buffer, area: Rect) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        let start = buf.index_of(area.x, y);
        for cell in &mut buf.content[start..start + usize::from(area.width)] {
            fill_cell(cell);
        }
    }
}

/// [`fill_from_background`] for one cell.
#[inline]
fn fill_cell(cell: &mut Cell) {
    let mut chars = cell.symbol().chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return;
    };
    // Most cells: spaces, text, braille.
    if !matches!(ch, '\u{2580}'..='\u{259F}' | '\u{1FB00}'..='\u{1FB3B}')
        || cell.modifier.contains(Modifier::REVERSED)
    {
        return;
    }
    let flipped = match ch {
        '▓' => Some('░'),
        _ => match block_mask(ch) {
            Some((mask, n)) => {
                let (ink, top) = (mask.count_ones(), (mask & 3).count_ones());
                let flip = top == 2 || (top == 1 && 2 * ink > n);
                flip.then(|| match n {
                    4 => quadrant(!mask & 15),
                    _ => sextant(!mask & 63),
                })
            }
            None => None,
        },
    };
    let Some(to) = flipped else {
        return;
    };
    // The ink becomes the background; the old background the ink, unless
    // nothing is left to ink.
    if cell.fg == Color::Reset || (to != ' ' && cell.bg == Color::Reset) {
        return;
    }
    let (fg, bg) = (cell.bg, cell.fg);
    cell.set_char(to).set_fg(fg).set_bg(bg);
}

/// The quadrant glyphs by ink mask (1 top left, 2 top right, 4 bottom
/// left, 8 bottom right).
const QUADRANTS: [char; 16] = [
    ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
];

/// The quadrant glyph whose ink is `mask` (see [`QUADRANTS`]).
pub fn quadrant(mask: u8) -> char {
    QUADRANTS[usize::from(mask & 15)]
}

/// The sextant glyph whose ink is `mask` (1, 2 the top row, 4, 8 the
/// middle, 16, 32 the bottom; left then right). Unicode 13 has all but
/// the empty, full and half ones, which Block Elements already had.
pub fn sextant(mask: u8) -> char {
    match mask & 63 {
        0 => ' ',
        21 => '▌',
        42 => '▐',
        63 => '█',
        m => {
            let skipped = u32::from(m > 21) + u32::from(m > 42);
            char::from_u32(0x1FB00 + u32::from(m) - 1 - skipped).unwrap_or('█')
        }
    }
}

/// A block glyph's ink mask and how many parts it has: a quadrant (or
/// half, or full) block in 4, a sextant in 6. Both have two columns, so
/// the top row is bits 1 and 2.
#[inline]
pub(super) fn block_mask(ch: char) -> Option<(u8, u32)> {
    if let Some(i) = QUADRANTS.iter().position(|&q| q == ch) {
        return Some((i as u8, 4));
    }
    let i = u32::from(ch).checked_sub(0x1FB00).filter(|&i| i < 60)?;
    // Undo `sextant`'s skips of 21 and 42.
    let mut m = i + 1;
    m += u32::from(m >= 21);
    m += u32::from(m >= 42);
    Some((m as u8, 6))
}

/// An empty cell: a space on `bg`. The foreground is left as it was.
#[inline]
pub fn blank(cell: &mut Cell, bg: Color) {
    cell.set_char(' ').set_bg(bg);
}

/// `ch` in `fg` on `bg`.
#[inline]
pub fn glyph(cell: &mut Cell, ch: char, fg: Color, bg: Color) {
    cell.set_char(ch).set_fg(fg).set_bg(bg);
}

/// `Some((ch, fg))` as a [`glyph`] on `bg`, `None` as a [`blank`].
#[inline]
pub fn mark(cell: &mut Cell, mark: Option<(char, Color)>, bg: Color) {
    match mark {
        Some((ch, fg)) => glyph(cell, ch, fg, bg),
        None => blank(cell, bg),
    }
}

/// The braille dots of cell (`cx`, `cy`) on a 2×4 grid: one per sample
/// pixel for which `dot(x, y)` holds. Pixels are visited row by row, left
/// to right, so anything `dot` accumulates adds up in a fixed order.
#[inline]
pub fn braille_dots(cx: usize, cy: usize, mut dot: impl FnMut(usize, usize) -> bool) -> u8 {
    /// Dot bit per sub-pixel, by row then column.
    const BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let mut bits = 0;
    for (dy, row) in BITS.iter().enumerate() {
        for (dx, bit) in row.iter().enumerate() {
            if dot(2 * cx + dx, 4 * cy + dy) {
                bits |= bit;
            }
        }
    }
    bits
}

/// The braille glyph with dots `bits` set (blank braille for 0).
#[inline]
pub fn braille(bits: u8) -> char {
    char::from_u32(0x2800 + u32::from(bits)).unwrap_or(' ')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{ColorDepth, Palette, Rgb};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn theme(depth: ColorDepth) -> Theme {
        Theme::new(&Palette::all()[0], depth)
    }

    fn draw(top: Pixel, bottom: Pixel, base: Color, theme: &Theme, translucent: bool) -> Cell {
        let mut cell = Cell::default();
        HalfBlocks::new(theme, base, translucent).draw(&mut cell, top, bottom);
        cell
    }

    /// (top, bottom) colours shown, and whether each is the glyph.
    fn shown(cell: &Cell) -> [(Color, bool); 2] {
        let (fg, bg) = ((cell.fg, true), (cell.bg, false));
        match cell.symbol() {
            "▀" => [fg, bg],
            "▄" => [bg, fg],
            "█" => [fg, fg],
            " " => [bg, bg],
            other => panic!("not a half block: {other:?}"),
        }
    }

    /// Pseudo-random colours near one another and far apart.
    fn pairs() -> impl Iterator<Item = (Color, Color)> {
        (0..4000u32).map(|i| {
            // splitmix64
            let mut h = u64::from(i).wrapping_add(0x9e37_79b9_7f4a_7c15);
            h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            h ^= h >> 31;
            let a = Rgb(h as u8, (h >> 8) as u8, (h >> 16) as u8);
            let spread = [2u8, 6, 12, 40, 255][i as usize % 5];
            let nudge = |v: u8, k: u32| {
                let d = ((h >> k) as u8 % spread) as i16 - i16::from(spread / 2);
                (i16::from(v) + d).clamp(0, 255) as u8
            };
            let b = Rgb(nudge(a.0, 24), nudge(a.1, 32), nudge(a.2, 40));
            (Color::Rgb(a.0, a.1, a.2), Color::Rgb(b.0, b.1, b.2))
        })
    }

    #[test]
    fn opaque_terminals_see_each_pixel_within_half_of_near() {
        let theme = theme(ColorDepth::TrueColor);
        let base = theme.color(crate::render::LIQUID);
        let gap = |a: Color, b: Color| match (a, b) {
            (Color::Rgb(r, g, b), Color::Rgb(r2, g2, b2)) => {
                crate::theme::delta_e(Rgb(r, g, b), Rgb(r2, g2, b2))
            }
            _ => unreachable!(),
        };
        let mut merged = 0;
        for (a, b) in pairs() {
            for (top, bottom) in [
                (Pixel::Ink(a), Pixel::Ink(b)),
                (Pixel::Back(a), Pixel::Back(b)),
                (Pixel::Ink(a), Pixel::Back(b)),
            ] {
                let cell = draw(top, bottom, base, &theme, false);
                let [(t, t_glyph), (b2, b_glyph)] = shown(&cell);
                assert!(gap(t, a) <= NEAR / 2.0 + 0.002, "{a:?} {b:?}: {cell:?}");
                assert!(gap(b2, b) <= NEAR / 2.0 + 0.002, "{a:?} {b:?}: {cell:?}");
                // Halves that look the same are one colour, in one layer.
                if theme.near(a, b) {
                    assert_eq!((t, t_glyph), (b2, b_glyph), "{a:?} {b:?}: {cell:?}");
                    merged += 1;
                }
            }
        }
        assert!(merged > 1000, "{merged}");
    }

    #[test]
    fn the_liquid_and_the_backdrop_go_behind() {
        let theme = theme(ColorDepth::TrueColor);
        let base = Color::Rgb(20, 10, 30);
        let (wax, sky) = (Color::Rgb(250, 160, 60), Color::Rgb(90, 40, 120));
        for translucent in [false, true] {
            let at = |top, bottom| draw(top, bottom, base, &theme, translucent);
            let cell = at(Pixel::Ink(wax), Pixel::Liquid);
            assert_eq!((cell.symbol(), cell.fg, cell.bg), ("▀", wax, base));
            let cell = at(Pixel::Liquid, Pixel::Ink(wax));
            assert_eq!((cell.symbol(), cell.fg, cell.bg), ("▄", wax, base));
            let cell = at(Pixel::Back(sky), Pixel::Ink(wax));
            assert_eq!((cell.symbol(), cell.fg, cell.bg), ("▄", wax, sky));
            let cell = at(Pixel::Liquid, Pixel::Liquid);
            assert_eq!((cell.symbol(), cell.bg), (" ", base));
            let cell = at(Pixel::Back(sky), Pixel::Back(sky));
            assert_eq!((cell.symbol(), cell.bg), (" ", sky));
        }
        // Two different wax colours: the darker behind, or, drawn for
        // translucent backgrounds, their mean as one glyph.
        let dark = Color::Rgb(120, 30, 20);
        let cell = draw(Pixel::Ink(dark), Pixel::Ink(wax), base, &theme, false);
        assert_eq!((cell.symbol(), cell.fg, cell.bg), ("▄", wax, dark));
        let cell = draw(Pixel::Ink(dark), Pixel::Ink(wax), base, &theme, true);
        assert_eq!((cell.symbol(), cell.fg), ("█", Color::Rgb(185, 95, 40)));
    }

    #[test]
    fn without_colour_only_the_glyph_shows_wax() {
        let theme = theme(ColorDepth::None);
        let none = Color::Reset;
        for translucent in [false, true] {
            let at = |top, bottom| {
                draw(top, bottom, none, &theme, translucent)
                    .symbol()
                    .to_string()
            };
            assert_eq!(at(Pixel::Ink(none), Pixel::Liquid), "▀");
            assert_eq!(at(Pixel::Liquid, Pixel::Ink(none)), "▄");
            assert_eq!(at(Pixel::Ink(none), Pixel::Ink(none)), "█");
            assert_eq!(at(Pixel::Liquid, Pixel::Liquid), " ");
        }
    }

    #[test]
    fn blocks_turn_to_show_their_top_edge_as_background() {
        let (a, b) = (Color::Rgb(250, 160, 60), Color::Rgb(20, 10, 30));
        let filled = |ch: char, fg: Color, bg: Color| {
            let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
            buf[(0, 0)].set_char(ch).set_fg(fg).set_bg(bg);
            let area = buf.area;
            fill_from_background(&mut buf, area);
            let cell = &buf[(0, 0)];
            (cell.symbol().chars().next().unwrap(), cell.fg, cell.bg)
        };
        assert_eq!(filled('█', a, b), (' ', b, a));
        assert_eq!(filled('▀', a, b), ('▄', b, a));
        assert_eq!(filled('▛', a, b), ('▗', b, a));
        assert_eq!(filled('▓', a, b), ('░', b, a));
        // Already background on top, or a tie of a top row and a cell.
        for ch in ['▄', ' ', '▌', '▘', '▚', '░', 'x', '⣿'] {
            assert_eq!(filled(ch, a, b), (ch, a, b), "{ch}");
        }
        // Sextants: top row, else most of the cell.
        assert_eq!(filled(sextant(3), a, b), (sextant(60), b, a));
        assert_eq!(
            filled(sextant(1 | 4 | 8 | 16), a, b),
            (sextant(2 | 32), b, a)
        );
        assert_eq!(filled(sextant(1 | 4 | 8), a, b), (sextant(1 | 4 | 8), a, b));
        // The terminal's own colours can't swap.
        assert_eq!(filled('█', Color::Reset, b), ('█', Color::Reset, b));
        assert_eq!(filled('▀', a, Color::Reset), ('▀', a, Color::Reset));
        assert_eq!(filled('█', a, Color::Reset), (' ', Color::Reset, a));
    }

    #[test]
    fn every_block_glyph_round_trips_its_mask() {
        for m in 0..16 {
            assert_eq!(block_mask(quadrant(m)), Some((m, 4)));
        }
        for m in 1..63 {
            if ![21, 42].contains(&m) {
                assert_eq!(block_mask(sextant(m)), Some((m, 6)));
            }
        }
    }
}
