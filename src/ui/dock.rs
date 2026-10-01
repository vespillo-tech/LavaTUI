//! Drawing the dock (`crate::dock`): the side panel (§4.5), the widgets on
//! the lava (floating, or on a soft backing), and the one-line chip.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Position;
use ratatui::style::{Color, Modifier, Style};

use crate::app::Model;
use crate::dock::{Backdrop, Backing, Look, WIDGETS};
use crate::theme::{self, ColorDepth, Role, TERMINAL_DEFAULT, Theme};
use crate::ui::layout::{CHIP_SEP, ChipRow, Stack, halo};

/// The side panel: each widget in its slot, on the app background.
pub fn draw_panel(buf: &mut Buffer, panel: &Stack, model: &Model) {
    let look = Look {
        backdrop: Backdrop::Panel,
        align: panel.align,
    };
    for p in &panel.items {
        WIDGETS[p.widget].draw(model, p.form, p.rect, look, buf);
    }
}

/// The soft backing behind widgets on the lava (`dock.backing = "soft"`): full strength out to `CORE`
/// from the stack (in rows; a column counts half), fading out over
/// `FEATHER` more, so it ends inside
/// [`HALO`](crate::ui::layout::HALO). At full strength the
/// lamp still shows through at `1 − VEIL`: a frosted pool of liquid, not a
/// box (picked from captures of every style; see docs/design.md §4.6).
const CORE: f32 = 0.5;
const FEATHER: f32 = 1.5;
const VEIL: f32 = 0.82;

/// How strongly the backing covers a cell `dx` columns and `dy` rows off
/// the stack: 1 within [`CORE`], 0 past [`FEATHER`] more.
fn cover(dx: u16, dy: u16) -> f32 {
    let d = ((f32::from(dx) * 0.5).powi(2) + f32::from(dy).powi(2)).sqrt();
    (1.0 - (d - CORE) / FEATHER).clamp(0.0, 1.0)
}

/// The widgets on the lava. `lamp` is the theme the lamp was drawn with
/// (its liquid may be flashing).
pub fn draw_on_lava(buf: &mut Buffer, stack: &Stack, model: &Model, lamp: &Theme) {
    let look = Look {
        backdrop: Backdrop::Lava,
        align: stack.align,
    };
    // Widgets draw into a scratch buffer, then onto the lamp.
    let mut scratch = Buffer::empty(stack.rect);
    for p in &stack.items {
        WIDGETS[p.widget].draw(model, p.form, p.rect, look, &mut scratch);
    }
    match model.settings.dock.backing {
        Backing::None => float(buf, &scratch, model.theme.role(Role::Dim), lamp),
        Backing::Soft => soft(buf, &scratch, lamp),
    }
}

/// Below this contrast ratio (WCAG) a floating word leaves its own ink
/// for the palette's light or dark one.
const LEGIBLE: f32 = 3.0;
/// How much a word's ink from the last frame is favoured: another ink has
/// to read this much better before the word flips, so a word flips once
/// as wax drifts under it, never back and forth frame to frame.
const STICKY: f32 = 1.25;
/// A letter of a text word whose ink would read below this over what's
/// behind it (the word straddles pale wax and dark liquid) takes the
/// better of the light and dark inks on its own, with the same
/// hysteresis. Big block digits never split: they read at any contrast.
const FLOOR: f32 = 1.8;

/// A floating word's ink: its own (the widget's role colours), or the
/// palette's light or dark one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    Own,
    Light,
    Dark,
}

/// The inks floating text had last frame: the one memory drawing keeps,
/// for [`STICKY`]. Rebuilt every frame, so it holds only what's on screen.
#[derive(Default)]
struct Inks {
    /// Each word's, by its first cell.
    words: HashMap<(u16, u16), Ink>,
    /// The letters that left their word's ink ([`FLOOR`]), by cell.
    letters: HashMap<(u16, u16), Ink>,
}

thread_local! {
    /// Drawing runs on one thread.
    static INKS: RefCell<Inks> = RefCell::default();
}

/// What a widget puts in a cell on the lava.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Put {
    Nothing,
    /// A glyph; `block` for the half-block glyphs faces are drawn with.
    Glyph {
        block: bool,
    },
    /// A one-cell gap between two words of a text line: the lamp's glyph
    /// is cleared (its colours stay), so `thu 1 oct` never reads
    /// `thu#1#oct` over the glyph styles.
    Gap,
}

/// No backing: the widgets' glyphs float on the lamp. Every cell keeps the
/// lamp's colours; only the cells a glyph takes change (spaces leave the
/// lamp showing, but for the gaps between words: [`Put::Gap`]). Block
/// glyphs (the clock faces) are composited pixel by pixel over half-block
/// lamps, so wax runs right up to each stroke. Text is bold (dim lines
/// aside).
///
/// Adaptive contrast, per *word*: glyphs joined along a line, and block
/// glyphs up and down too, so a clock digit is one word and never two-tone.
/// A word keeps its own ink while that reads ≥ [`LEGIBLE`] against
/// everything behind it, else takes whichever of the palette's light and
/// dark inks reads better against its worst cell: dark over bright wax,
/// light over the liquid. With [`STICKY`] hysteresis.
/// `dim` is the ink of secondary lines, drawn without bold.
fn float(buf: &mut Buffer, scratch: &Buffer, dim: Color, lamp: &Theme) {
    let area = scratch.area.intersection(buf.area);
    let (w, h) = (usize::from(area.width), usize::from(area.height));
    let at = |i: usize| Position::new(area.x + (i % w) as u16, area.y + (i / w) as u16);

    let mut puts = vec![Put::Nothing; w * h];
    for (i, put) in puts.iter_mut().enumerate() {
        let from = &scratch[at(i)];
        if from.bg != TERMINAL_DEFAULT {
            // A picture (album art) brings its own background.
            buf[at(i)].set_symbol(from.symbol()).set_style(from.style());
        } else if from.symbol() != " " {
            let block = ink_halves(from.symbol()).is_some();
            *put = Put::Glyph { block };
        }
    }
    let text = |p: Put| p == Put::Glyph { block: false };
    for i in 1..puts.len().saturating_sub(1) {
        let inside = i % w != 0 && i % w != w - 1;
        if inside && puts[i] == Put::Nothing && text(puts[i - 1]) && text(puts[i + 1]) {
            puts[i] = Put::Gap;
        }
    }

    // Words: union-find over the cells.
    let mut parent: Vec<usize> = (0..puts.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..puts.len() {
        if puts[i] == Put::Nothing {
            continue;
        }
        let left = (i % w != 0 && puts[i - 1] != Put::Nothing).then(|| i - 1);
        let block = |p| matches!(p, Put::Glyph { block: true });
        let up = (i >= w && block(puts[i]) && block(puts[i - w])).then(|| i - w);
        for j in left.into_iter().chain(up) {
            let (a, b) = (root(&mut parent, i), root(&mut parent, j));
            parent[a.max(b)] = a.min(b);
        }
    }
    // In cell order, so drawing never depends on hashing.
    let mut words: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, put) in puts.iter().enumerate() {
        if *put != Put::Nothing {
            words.entry(root(&mut parent, i)).or_default().push(i);
        }
    }

    let (light, dark) = lamp.floating_inks();
    let last = INKS.with_borrow_mut(std::mem::take);
    let mut inks = Inks::default();
    let pick = |ink| match ink {
        Ink::Light => light,
        Ink::Dark => dark,
        Ink::Own => unreachable!("only light or dark is picked"),
    };
    // Luminances, each worked out once: what's behind every glyph, and
    // every ink. `None` where a colour can't be told (16 colours' defaults,
    // no colour): such words keep their own ink.
    let back: Vec<Option<f32>> = (0..puts.len())
        .map(|i| match puts[i] {
            Put::Glyph { .. } => lamp.luminance(seen_under(&buf[at(i)], lamp)),
            _ => None,
        })
        .collect();
    // Widgets use a few role inks: remember the last one.
    let mut own = (TERMINAL_DEFAULT, None);
    let mut own_lum = |c: Color| {
        if own.0 != c {
            own = (c, lamp.luminance(c));
        }
        own.1
    };
    let (light_l, dark_l) = (lamp.luminance(light), lamp.luminance(dark));
    let lum = |ink| match ink {
        Ink::Light => light_l,
        Ink::Dark => dark_l,
        Ink::Own => None,
    };
    for (first, cells) in words {
        let key = (at(first).x, at(first).y);
        let glyphs: Vec<usize> = cells
            .iter()
            .copied()
            .filter(|&i| puts[i] != Put::Gap)
            .collect();
        // The worst contrast an ink (by cell) has over the word.
        let worst = |ink: &mut dyn FnMut(usize) -> Option<f32>| {
            glyphs
                .iter()
                .map(|&i| Some(theme::contrast(ink(i)?, back[i]?)))
                .try_fold(f32::MAX, |m, c| Some(m.min(c?)))
        };
        let sticky = |which: Ink| {
            if last.words.get(&key) == Some(&which) {
                STICKY
            } else {
                1.0
            }
        };
        let scores = (
            worst(&mut |i| own_lum(scratch[at(i)].fg)).map(|s| s * sticky(Ink::Own)),
            worst(&mut |_| light_l).map(|s| s * sticky(Ink::Light)),
            worst(&mut |_| dark_l).map(|s| s * sticky(Ink::Dark)),
        );
        let ink = match scores {
            (Some(own), Some(l), Some(d)) if own < LEGIBLE => {
                if d > l {
                    Ink::Dark
                } else {
                    Ink::Light
                }
            }
            _ => Ink::Own,
        };
        inks.words.insert(key, ink);
        let text = cells.iter().all(|&i| puts[i] != Put::Glyph { block: true });
        for i in cells {
            let pos = at(i);
            let to = &mut buf[pos];
            if puts[i] == Put::Gap {
                if halves(to.symbol(), to.fg, to.bg).is_none() {
                    to.set_char(' ');
                }
                continue;
            }
            let from = &scratch[pos];
            let (mut fg, fg_l) = match ink {
                Ink::Own => (from.fg, own_lum(from.fg)),
                other => (pick(other), lum(other)),
            };
            // A letter on its own (text only, never a block digit).
            if text && let (Some(f), Some(b)) = (fg_l, back[i]) {
                let read = |l: Option<f32>| l.map_or(0.0, |l| theme::contrast(l, b));
                let key = (pos.x, pos.y);
                let reads = theme::contrast(f, b);
                // (The memory is only asked when it could matter.)
                let was = (reads < FLOOR * STICKY)
                    .then(|| last.letters.get(&key).copied())
                    .flatten();
                let floor = if was.is_some() { FLOOR * STICKY } else { FLOOR };
                if reads < floor {
                    let best = if read(dark_l) > read(light_l) {
                        Ink::Dark
                    } else {
                        Ink::Light
                    };
                    let best = match was {
                        Some(w) if read(lum(w)) * STICKY >= read(lum(best)) => w,
                        _ => best,
                    };
                    if read(lum(best)) > read(fg_l) {
                        fg = pick(best);
                        inks.letters.insert(key, best);
                    }
                }
            }
            paint(to, from.symbol(), fg, from.fg != dim, lamp);
        }
    }
    INKS.with_borrow_mut(|m| *m = inks);
}

/// What a lamp cell shows behind a glyph put in it: its background, or for
/// a half block, both halves' mean (truecolor; else its background).
fn under(cell: &Cell, lamp: &Theme) -> Color {
    match halves(cell.symbol(), cell.fg, cell.bg) {
        Some((t, b)) => lamp.mean(t, b),
        None => cell.bg,
    }
}

/// What the eye sees around a glyph put in a lamp cell, for contrast: as
/// [`under`], but a glyph style's own glyph counts half (its dots and
/// letters surround the text).
fn seen_under(cell: &Cell, lamp: &Theme) -> Color {
    match halves(cell.symbol(), cell.fg, cell.bg) {
        Some((t, b)) => lamp.mean(t, b),
        None => lamp.mean(cell.fg, cell.bg),
    }
}

/// Put a widget's `symbol` in `ink` into lamp cell `to`.
fn paint(to: &mut Cell, symbol: &str, ink: Color, bold: bool, lamp: &Theme) {
    let mut style = Style::new();
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    let lamp_pixels = halves(to.symbol(), to.fg, to.bg);
    match ink_halves(symbol) {
        // Block glyphs over block (or blank) lamp cells: per pixel. The
        // ink is always the glyph's foreground (it may be the terminal's
        // default, or `bg`, which a transparent theme never paints).
        Some((top, bottom)) if lamp.has_color() => {
            let (lt, lb) = lamp_pixels.unwrap_or((to.bg, to.bg));
            let (ch, bg) = match (top, bottom) {
                (true, true) => ('█', lb),
                (true, false) => ('▀', lb),
                _ => ('▄', lt),
            };
            to.set_char(ch).set_style(style.fg(ink).bg(bg));
        }
        // Text: on what the cell showed behind its glyph.
        _ => {
            let bg = under(to, lamp);
            to.set_symbol(symbol).set_style(style.fg(ink).bg(bg));
        }
    }
}

/// A lamp cell's top and bottom pixel colours, if it's drawn in half
/// blocks (or blank); `None` for any other glyph.
fn halves(symbol: &str, fg: Color, bg: Color) -> Option<(Color, Color)> {
    match symbol {
        " " => Some((bg, bg)),
        "▀" => Some((fg, bg)),
        "▄" => Some((bg, fg)),
        "█" => Some((fg, fg)),
        _ => None,
    }
}

/// Which halves of a widget's glyph are ink, if it's a block glyph.
fn ink_halves(symbol: &str) -> Option<(bool, bool)> {
    match symbol {
        "▀" => Some((true, false)),
        "▄" => Some((false, true)),
        "█" => Some((true, true)),
        _ => None,
    }
}

/// `dock.backing = "soft"`: a soft pool of liquid. Under the stack and just
/// around it the lamp is veiled most of the way to its liquid (glyphs
/// cleared), and the veil fades out over the next cells, so the readout
/// sits in a calm pool that the wax melts into rather than a box. Below
/// truecolor (256 colours would snap the tints to greys) the backing is
/// plain liquid where it's at least half strength.
fn soft(buf: &mut Buffer, scratch: &Buffer, lamp: &Theme) {
    let r = scratch.area;
    let liquid = lamp.role(Role::Liquid);
    // 256 colours would snap the veiled tints to cube greys: a grey box.
    let soft = lamp.depth() == ColorDepth::TrueColor;
    for pos in halo(r).intersection(buf.area).positions() {
        let (x, y) = (pos.x, pos.y);
        let dx = r
            .left()
            .saturating_sub(x)
            .max(x.saturating_sub(r.right() - 1));
        let dy = r
            .top()
            .saturating_sub(y)
            .max(y.saturating_sub(r.bottom() - 1));
        let a = cover(dx, dy);
        let cell = &mut buf[(x, y)];
        if !soft {
            if a >= 0.5 {
                cell.set_char(' ').set_fg(liquid).set_bg(liquid);
            }
        } else if a >= 1.0 {
            let bg = lamp.veil(cell.bg, Role::Liquid, VEIL);
            cell.set_char(' ').set_fg(bg).set_bg(bg);
        } else if a > 0.0 {
            let (fg, bg) = (cell.fg, cell.bg);
            cell.set_fg(lamp.veil(fg, Role::Liquid, a * VEIL));
            cell.set_bg(lamp.veil(bg, Role::Liquid, a * VEIL));
        }
    }
    for pos in r.positions() {
        let from = &scratch[pos];
        // Pictures (album art) bring their own background; text keeps the
        // backing's.
        let own_bg = from.bg != TERMINAL_DEFAULT;
        if from.symbol() != " " || own_bg {
            let to = &mut buf[pos];
            let bg = if own_bg { from.bg } else { to.bg };
            to.set_symbol(from.symbol())
                .set_style(from.style())
                .set_bg(bg);
        }
    }
}

/// The chip row over the lamp: ` 14:32 · ▸ 18:24 `, on `bg` with a
/// 1-cell pad, the dots `dim`.
pub fn draw_chips(buf: &mut Buffer, row: &ChipRow, model: &Model) {
    let bg = Style::new().bg(super::background(model));
    let r = row.rect;
    buf.set_string(r.x, r.y, " ".repeat(usize::from(r.width)), bg);
    let dim = model.theme.text(Role::Dim).patch(bg);
    for (i, chip) in row.items.iter().enumerate() {
        let Some(c) = WIDGETS[chip.widget].chip(model) else {
            continue;
        };
        let at = chip.rect;
        buf.set_stringn(
            at.x,
            at.y,
            &c.text,
            usize::from(at.width),
            model.theme.text(c.ink).patch(bg),
        );
        if i > 0 {
            let x = at.x - CHIP_SEP + 1;
            buf.set_string(x, at.y, "·", dim);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;
    use crate::theme::{ColorDepth, Palette};

    fn theme(palette: &str) -> Theme {
        Theme::new(Palette::by_name(palette).unwrap(), ColorDepth::TrueColor)
    }

    /// A one-row lamp of `cells` (`(symbol, fg, bg)`) and `text` on it.
    fn float_row(lamp: &Theme, cells: &[(&str, Color, Color)], text: &str, ink: Color) -> Buffer {
        let area = Rect::new(0, 0, cells.len() as u16, 1);
        let mut buf = Buffer::empty(area);
        for (x, &(symbol, fg, bg)) in cells.iter().enumerate() {
            buf[(x as u16, 0)].set_symbol(symbol).set_fg(fg).set_bg(bg);
        }
        let mut scratch = Buffer::empty(area);
        scratch.set_string(0, 0, text, Style::new().fg(ink));
        float(&mut buf, &scratch, lamp.role(Role::Dim), lamp);
        buf
    }

    #[test]
    fn text_floats_keeping_the_lamp_and_its_glyphs_in_spaces() {
        let t = theme("lava");
        let (liquid, wax, text) = (
            t.role(Role::Liquid),
            t.role(Role::WaxMid),
            t.role(Role::Text),
        );
        let buf = float_row(&t, &[("#", wax, liquid); 9], "ab cd  e", text);
        let row: String = (0..9).map(|x| buf[(x, 0)].symbol()).collect();
        // The gap between two words is cleared of its glyph; wider spaces
        // and everything past the text keep the lamp's.
        assert_eq!(row, "ab cd##e#");
        for x in 0..9 {
            assert_eq!(buf[(x, 0)].bg, liquid, "{x}: the lamp's colour stays");
        }
        assert_eq!(buf[(0, 0)].fg, text);
        assert!(buf[(0, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(5, 0)].fg, wax);
    }

    #[test]
    fn text_goes_dark_over_bright_wax_and_stays_itself_on_liquid() {
        let t = theme("lava");
        let (liquid, hot, text) = (
            t.role(Role::Liquid),
            t.role(Role::WaxHot),
            t.role(Role::Text),
        );
        let (_, dark) = t.floating_inks();
        let mut cells = vec![(" ", hot, hot); 3];
        cells.extend([(" ", liquid, liquid); 4]);
        let buf = float_row(&t, &cells, "abc def", text);
        assert_eq!(buf[(0, 0)].fg, dark, "over wax");
        assert_eq!(buf[(2, 0)].fg, dark, "one ink for the word");
        assert_eq!(buf[(4, 0)].fg, text, "on liquid");
        assert_eq!(buf[(0, 0)].bg, hot, "no backing");
    }

    #[test]
    fn a_word_flips_once_not_back_and_forth() {
        let t = theme("lava");
        let text = t.role(Role::Text);
        let (_, dark) = t.floating_inks();
        let ink_over = |grey: u8| {
            let g = Color::Rgb(grey, grey, grey);
            float_row(&t, &[(" ", g, g); 3], "abc", text)[(0, 0)].fg
        };
        // Own ink reads ~4.3 : 1 over grey 100, ~2.7 over 133, ~1.9 over 160.
        assert_eq!(ink_over(100), text);
        assert_eq!(ink_over(133), text, "inside the band: no flip yet");
        assert_eq!(ink_over(160), dark);
        assert_eq!(ink_over(133), dark, "inside the band: no flip back");
        assert_eq!(ink_over(100), text);
    }

    #[test]
    fn a_straddling_word_keeps_every_letter_readable() {
        let t = theme("mono");
        let (liquid, text) = (t.role(Role::Liquid), t.role(Role::Text));
        let pale = Color::Rgb(230, 230, 230);
        let (_, dark) = t.floating_inks();
        let mut cells = vec![(" ", pale, pale)];
        cells.extend([(" ", liquid, liquid); 4]);
        let buf = float_row(&t, &cells, "focus", text);
        assert_eq!(buf[(0, 0)].fg, dark, "the letter over pale wax");
        assert_eq!(buf[(1, 0)].fg, text, "the rest of the word");
    }

    #[test]
    fn block_glyphs_composite_per_pixel_with_the_ink_in_front() {
        let t = theme("lava");
        let (liquid, wax, text) = (
            t.role(Role::Liquid),
            t.role(Role::WaxMid),
            t.role(Role::Text),
        );
        // Face ▀ over a lamp ▄ (wax below): ink on top, wax below.
        let buf = float_row(&t, &[("▄", wax, liquid), ("▀", wax, liquid)], "▀▄", text);
        let (a, b) = (&buf[(0, 0)], &buf[(1, 0)]);
        assert_eq!((a.symbol(), a.bg), ("▀", wax));
        assert_eq!((b.symbol(), b.bg), ("▄", wax));
        assert!(a.fg != liquid && b.fg != liquid);
        // The terminal's default ink (the ansi palette) is still drawn.
        let ansi = theme("ansi");
        let d = TERMINAL_DEFAULT;
        let buf = float_row(&ansi, &[(" ", d, d)], "█", d);
        assert_eq!(buf[(0, 0)].symbol(), "█");
    }
}
