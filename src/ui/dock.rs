//! Drawing the dock (`crate::dock`): the side panel (§4.5), the widgets on
//! the lava (floating, or on a soft backing), and the one-line chip.

use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::app::Model;
use crate::dock::{Backdrop, Backing, Look, WIDGETS};
use crate::graphics;
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
        Backing::None => float(
            buf,
            &scratch,
            model.theme.role(Role::Dim),
            lamp,
            model.cell_opacity(),
        ),
        Backing::Soft => soft(buf, &scratch, lamp),
    }
}

/// Floating text reads at least this well (WCAG contrast) against what's
/// displayed behind each glyph: its own ink while that clears the bar,
/// else the better of the palette's light and dark inks. AA for text.
const READABLE: f32 = 4.5;
/// The bar for block glyphs (the big clock digits): WCAG's large text;
/// and the least any glyph's own ink may read.
const LARGE: f32 = 3.0;
/// A role ink that reads less than [`READABLE`] on the plain liquid (dim
/// lines: lava's `dim` 3.6 : 1) keeps itself while it reads at least this
/// share of that, never below [`LARGE`], so a quiet line stays quiet on
/// its own liquid and leaves for light / dark as soon as wax makes it
/// worse. (Paper's `dim`, 2.85 : 1 on its liquid, reads dark instead.)
const SLACK: f32 = 0.9;
/// A glyph that left its own ink takes it back once that reads this much
/// over the bar: the bar's hysteresis, above it, so the own ink never
/// reads below the bar. Under 1 / [`SLACK`], so a quiet ink still gets
/// back on its liquid.
const RETURN: f32 = 1.08;
/// How much a glyph's ink from the last frame is favoured (its contrast
/// counts this much more): it changes once as wax drifts under it, never
/// back and forth frame to frame. Light and dark cross at ≈ 3.8 : 1 on
/// lava, so the ink kept never reads below ≈ 3.5 : 1.
const STICKY: f32 = 1.15;

/// A glyph follows the ink of the one before it in its word (where light
/// and dark read about as well) only while that reads at least this.
const FOLLOW: f32 = 3.3;

/// A floating glyph's ink: its own (the widget's role colours), or the
/// palette's light or dark one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    Own,
    Light,
    Dark,
}

/// What floating glyphs had last frame, by cell: the one memory drawing
/// keeps, for [`STICKY`] and [`Memory::next`]. Each stack replaces its
/// own area's every frame, so it holds only what's on screen.
type Inks = HashMap<(u16, u16), Memory>;

/// One glyph's memory.
#[derive(Debug, Clone, Copy)]
struct Memory {
    /// The glyph (another glyph there, say a new lyric line, starts
    /// afresh).
    glyph: char,
    /// Its ink.
    ink: Ink,
    /// The ink it wanted instead, if any: a change waits one frame, so a
    /// line of the backdrop sweeping under a glyph (synthwave's grid)
    /// doesn't make it blink.
    next: Option<Ink>,
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

/// A floating glyph's ink this frame, before it's painted.
#[derive(Debug, Clone, Copy)]
struct Decided {
    ink: Ink,
    /// What it would have taken without the one frame's wait.
    want: Ink,
    reads: Option<[f32; 3]>,
    glyph: char,
}

/// One floating glyph's case for each ink.
struct Glyph {
    /// Contrast of own, light and dark against what's displayed behind
    /// it; `None` when that can't be told (no colour, the terminal's
    /// defaults): the glyph keeps its own ink.
    reads: Option<[f32; 3]>,
    /// The bar each ink must clear.
    bars: [f32; 3],
    /// Its ink last frame.
    was: Option<Ink>,
}

impl Glyph {
    /// How well `ink` reads, favoured if it's the glyph's last.
    fn score(&self, ink: Ink) -> f32 {
        let Some(reads) = self.reads else {
            return 0.0;
        };
        let sticky = if self.was == Some(ink) { STICKY } else { 1.0 };
        reads[ink as usize] * sticky
    }

    /// Whether `ink` clears its bar here: an ink it had last frame keeps
    /// it down to the bar, another needs [`RETURN`] more, so a glyph
    /// never reads below its bar in an ink it was left in.
    fn clears(&self, ink: Ink) -> bool {
        let lift = if self.was == Some(ink) { 1.0 } else { RETURN };
        self.reads
            .is_some_and(|r| r[ink as usize] >= self.bars[ink as usize] * lift)
    }

    /// The glyph's ink: its own while that clears the bar, else the
    /// better of light and dark (≳ 3.5 : 1, see [`STICKY`]). Where those
    /// two read about as well (within [`STICKY`], both ≥ [`FOLLOW`]) it
    /// follows `left`, the glyph before it in the word, so a word over
    /// wax in between never comes out speckled.
    fn ink(&self, left: Option<Ink>) -> Ink {
        if self.reads.is_none() || self.clears(Ink::Own) {
            return Ink::Own;
        }
        let best = if self.score(Ink::Dark) > self.score(Ink::Light) {
            Ink::Dark
        } else {
            Ink::Light
        };
        match left {
            Some(l @ (Ink::Light | Ink::Dark))
                if l != best
                    && self.reads.is_some_and(|r| r[l as usize] >= FOLLOW)
                    && self.score(l) * STICKY >= self.score(best) =>
            {
                l
            }
            _ => best,
        }
    }
}

/// No backing: the widgets' glyphs float on the lamp. Every cell keeps the
/// lamp's colours; only the cells a glyph takes change (spaces leave the
/// lamp showing, but for the gaps between words: [`Put::Gap`]). Block
/// glyphs (the clock faces) are composited pixel by pixel over half-block
/// lamps, so wax runs right up to each stroke. Text is bold (dim lines
/// aside).
///
/// Adaptive contrast, per *glyph*, against what the terminal displays
/// right behind it ([`behind`]; `opacity`: see-through cell backgrounds,
/// [`Theme::shown_luminance`]). A glyph keeps its own ink while that
/// reads ≥ [`READABLE`] ([`LARGE`] for block glyphs; a quiet ink: see
/// [`SLACK`]), else takes the better of the palette's light and dark
/// inks, with [`STICKY`] hysteresis kept per glyph. Nothing else decides
/// it, so a glyph changes only when what's behind it does: never a whole
/// word or line at once. Words stay one ink where that costs nothing: a
/// glyph that reads about as well in light as in dark follows the one
/// before it ([`Glyph::ink`]), and a big clock digit takes one ink
/// wherever that reads ≥ [`LARGE`]. A change waits a frame ([`Memory`])
/// unless the ink it has reads below [`LARGE`].
/// `dim` is the ink of secondary lines, drawn without bold.
fn float(buf: &mut Buffer, scratch: &Buffer, dim: Color, lamp: &Theme, opacity: Option<f32>) {
    let translucent = opacity.is_some();
    let area = scratch.area.intersection(buf.area);
    let (w, h) = (usize::from(area.width), usize::from(area.height));
    let at = |i: usize| Position::new(area.x + (i % w) as u16, area.y + (i / w) as u16);

    let mut puts = vec![Put::Nothing; w * h];
    for (i, put) in puts.iter_mut().enumerate() {
        let from = &scratch[at(i)];
        if from.bg != TERMINAL_DEFAULT || graphics::is_placeholder(from.symbol()) {
            // A picture (album art) brings its own background (and a kitty
            // placeholder's ink is its image id: never touched).
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

    let (light, dark) = lamp.floating_inks();
    // Each stack on the lava is drawn on its own: take only this one's
    // memory (and drop what's off the screen now).
    let last = INKS.with_borrow_mut(|m| {
        let mut mine = Inks::default();
        m.retain(|&(x, y), ink| {
            let p = Position::new(x, y);
            if area.contains(p) {
                mine.insert((x, y), *ink);
            }
            buf.area.contains(p) && !area.contains(p)
        });
        mine
    });
    let mut inks = Inks::default();
    // Widgets use a few role inks: remember the last one, with how well it
    // reads on the plain liquid.
    let liquid_l = lamp.shown_luminance(lamp.role(Role::Liquid), opacity);
    let mut own = (TERMINAL_DEFAULT, None, f32::MAX);
    let mut own_lum = |c: Color| {
        if own.0 != c {
            let l = lamp.luminance(c);
            let calm = match (l, liquid_l) {
                (Some(o), Some(q)) => theme::contrast(o, q),
                _ => f32::MAX,
            };
            own = (c, l, calm);
        }
        (own.1, own.2)
    };
    let (light_l, dark_l) = (lamp.luminance(light), lamp.luminance(dark));
    // Each glyph's ink, painted once the digits are settled.
    let mut decided: Vec<Option<Decided>> = vec![None; puts.len()];
    // The ink of the glyph before, while in the same word.
    let mut left = None;
    for (i, &put) in puts.iter().enumerate() {
        let pos = at(i);
        let to = &mut buf[pos];
        match put {
            Put::Nothing => {
                left = None;
                continue;
            }
            Put::Gap => {
                left = None;
                match halves(to.symbol(), to.fg, to.bg) {
                    None => {
                        to.set_char(' ');
                    }
                    // A half block between two letters: the letters' own
                    // backing (what they're painted on), so a line reads
                    // as one strip, never `thu▄1`.
                    Some(_) if !translucent && lamp.has_color() => {
                        let bg = under(to, lamp);
                        to.set_char(' ').set_fg(bg).set_bg(bg);
                    }
                    Some(_) => {}
                }
                continue;
            }
            Put::Glyph { block } => {
                let from = &scratch[pos];
                let (own_l, calm) = own_lum(from.fg);
                let reads = match (own_l, light_l, dark_l, behind(to, block, lamp, opacity)) {
                    (Some(o), Some(l), Some(d), Some(b)) => {
                        Some([o, l, d].map(|x| theme::contrast(x, b)))
                    }
                    _ => None,
                };
                let bar = if block { LARGE } else { READABLE };
                let ch = from.symbol().chars().next().unwrap_or(' ');
                let key = (pos.x, pos.y);
                let memory = last.get(&key).filter(|m| m.glyph == ch).copied();
                let glyph = Glyph {
                    reads,
                    bars: [bar.min(calm * SLACK).max(LARGE), bar, bar],
                    was: memory.map(|m| m.ink),
                };
                let want = glyph.ink(if i % w == 0 { None } else { left });
                // A change shows once it's wanted two frames running, or
                // at once if the ink it has reads below the least.
                let ink = match (memory, reads) {
                    (Some(m), Some(r))
                        if want != m.ink && m.next != Some(want) && r[m.ink as usize] >= LARGE =>
                    {
                        m.ink
                    }
                    _ => want,
                };
                left = Some(ink);
                decided[i] = Some(Decided {
                    ink,
                    want,
                    reads,
                    glyph: ch,
                });
            }
        }
    }
    // A big digit (block glyphs joined up, down and across) takes the ink
    // most of its cells chose wherever that still reads ≥ LARGE there: one
    // ink, unless a stroke straddles pale wax and dark liquid.
    let mut seen = vec![false; puts.len()];
    for start in 0..puts.len() {
        if seen[start] || puts[start] != (Put::Glyph { block: true }) {
            continue;
        }
        let mut digit = vec![start];
        seen[start] = true;
        let mut k = 0;
        while let Some(&i) = digit.get(k) {
            k += 1;
            let (x, y) = (i % w, i / w);
            let near = [
                (x > 0).then(|| i - 1),
                (x + 1 < w).then(|| i + 1),
                (y > 0).then(|| i - w),
                (y + 1 < h).then(|| i + w),
            ];
            for j in near.into_iter().flatten() {
                if !seen[j] && puts[j] == (Put::Glyph { block: true }) {
                    seen[j] = true;
                    digit.push(j);
                }
            }
        }
        let count = |ink| {
            digit
                .iter()
                .filter(|&&i| decided[i].is_some_and(|d| d.reads.is_some() && d.ink == ink))
                .count()
        };
        let most = [Ink::Own, Ink::Light, Ink::Dark]
            .into_iter()
            .max_by_key(|&ink| (count(ink), ink == Ink::Own))
            .unwrap_or(Ink::Own);
        for &i in &digit {
            if let Some(d) = decided[i].as_mut()
                && d.reads.is_some_and(|r| r[most as usize] >= LARGE)
            {
                d.ink = most;
            }
        }
    }
    for (i, d) in decided.iter().enumerate() {
        let Some(d) = d else { continue };
        let pos = at(i);
        if d.reads.is_some() {
            let next = (d.want != d.ink).then_some(d.want);
            let memory = Memory {
                glyph: d.glyph,
                ink: d.ink,
                next,
            };
            inks.insert((pos.x, pos.y), memory);
        }
        let from = &scratch[pos];
        let fg = match d.ink {
            Ink::Own => from.fg,
            Ink::Light => light,
            Ink::Dark => dark,
        };
        let bold = from.fg != dim || from.modifier.contains(Modifier::BOLD);
        paint(&mut buf[pos], from.symbol(), fg, bold, lamp, translucent);
    }
    INKS.with_borrow_mut(|m| m.extend(inks));
    // (16 colours: a bright wax colour as a background isn't always
    // that colour.)
    if !translucent && matches!(lamp.depth(), ColorDepth::TrueColor | ColorDepth::Ansi256) {
        solid_around(buf, area, |p| {
            !area.contains(p) || puts[at_index(area, p)] == Put::Nothing
        });
    }
}

/// The luminance displayed behind a glyph put in lamp cell `cell`, for
/// contrast. Text replaces the lamp's glyph and sits on what the cell
/// showed behind it ([`under`]); a block glyph (a clock stroke) sits among
/// the lamp's pixels: both halves' mean. With see-through backgrounds
/// (`opacity`) a background shows darker, the lamp's opaque glyph doesn't.
fn behind(cell: &Cell, block: bool, lamp: &Theme, opacity: Option<f32>) -> Option<f32> {
    if !block {
        return lamp.shown_luminance(under(cell, lamp), opacity);
    }
    let (fg, bg) = (
        lamp.luminance(cell.fg),
        lamp.shown_luminance(cell.bg, opacity),
    );
    match cell.symbol() {
        "▀" | "▄" => Some((fg? + bg?) / 2.0),
        "█" => fg,
        _ => bg,
    }
}

/// Lamp cells (`lamp_cell`) in and one cell around floating widgets that are one colour
/// all over (`█`, or a half block whose halves match) become that colour's
/// plain background: they look the same, but no block glyph is left
/// against the text, where a terminal that draws blocks a hair short of
/// the cell shows the backdrop through as thin lines (lava-1xk.21). Not
/// with see-through cell backgrounds, where wax must stay a glyph.
fn solid_around(buf: &mut Buffer, area: Rect, lamp_cell: impl Fn(Position) -> bool) {
    let around = Rect::new(
        area.x.saturating_sub(1),
        area.y.saturating_sub(1),
        area.width + 2,
        area.height + 2,
    )
    .intersection(buf.area);
    for pos in around.positions().filter(|&p| lamp_cell(p)) {
        let cell = &mut buf[pos];
        if let Some((top, bottom)) = halves(cell.symbol(), cell.fg, cell.bg)
            && top == bottom
            && top != TERMINAL_DEFAULT
            && cell.symbol() != " "
        {
            cell.set_char(' ').set_fg(top).set_bg(top);
        }
    }
}

/// `p`'s index in a row-major vector over `area`.
fn at_index(area: Rect, p: Position) -> usize {
    usize::from(p.y - area.y) * usize::from(area.width) + usize::from(p.x - area.x)
}

/// What a lamp cell shows behind a glyph put in it: its background, or for
/// a half block, both halves' mean (truecolor; else its background).
fn under(cell: &Cell, lamp: &Theme) -> Color {
    match halves(cell.symbol(), cell.fg, cell.bg) {
        Some((t, b)) => lamp.mean(t, b),
        None => cell.bg,
    }
}

/// Put a widget's `symbol` in `ink` into lamp cell `to`.
///
/// With `translucent` (see-through cell backgrounds, opaque glyphs) a
/// block glyph's other half is never the lamp's foreground pixel (wax),
/// which would show darker than the wax around it: it's the lamp cell's
/// background, what's behind the wax, as in [`crate::render::cell::half_block`].
/// The wax gives up half a cell beside the stroke; the stroke stays whole.
fn paint(to: &mut Cell, symbol: &str, ink: Color, bold: bool, lamp: &Theme, translucent: bool) {
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
            let (lt, lb) = match lamp_pixels {
                Some(pixels) if !translucent => pixels,
                _ => (to.bg, to.bg),
            };
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
        let own_bg = from.bg != TERMINAL_DEFAULT || graphics::is_placeholder(from.symbol());
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
        float_row_on(lamp, cells, text, ink, false)
    }

    /// As [`float_row`], on a terminal with see-through cell backgrounds
    /// when `translucent`.
    fn float_row_on(
        lamp: &Theme,
        cells: &[(&str, Color, Color)],
        text: &str,
        ink: Color,
        translucent: bool,
    ) -> Buffer {
        let area = Rect::new(0, 0, cells.len() as u16, 1);
        let mut buf = Buffer::empty(area);
        for (x, &(symbol, fg, bg)) in cells.iter().enumerate() {
            buf[(x as u16, 0)].set_symbol(symbol).set_fg(fg).set_bg(bg);
        }
        let mut scratch = Buffer::empty(area);
        scratch.set_string(0, 0, text, Style::new().fg(ink));
        float(
            &mut buf,
            &scratch,
            lamp.role(Role::Dim),
            lamp,
            translucent.then_some(0.75),
        );
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

    /// A grey lamp cell `g` (all three channels).
    fn grey(g: u8) -> Color {
        Color::Rgb(g, g, g)
    }

    #[test]
    fn a_glyph_flips_once_not_back_and_forth() {
        let t = theme("lava");
        let text = t.role(Role::Text);
        let (_, dark) = t.floating_inks();
        let ink_over = |g: u8| float_row(&t, &[(" ", grey(g), grey(g)); 3], "abc", text)[(0, 0)].fg;
        // Own ink reads 6 : 1 over grey 80, 2.2 over 150 (dark: 6.6);
        // light and dark cross near 110 (≈ 3.8 : 1 each).
        assert_eq!(ink_over(80), text);
        assert_eq!(ink_over(150), dark, "at once: the own ink read below 3 : 1");
        assert_eq!(ink_over(112), dark, "inside the band: no flip back");
        assert_eq!(ink_over(112), dark);
        // Over grey 95 dark still reads 3 : 1 and light 4.8: the change
        // waits a frame, then shows.
        assert_eq!(ink_over(95), dark, "one frame's wait");
        assert_eq!(ink_over(95), text, "then light");
        assert_eq!(ink_over(112), text, "inside the band: no flip back");
    }

    #[test]
    fn a_quiet_ink_stays_quiet_on_its_own_liquid() {
        // Lava's `dim` reads 3.6 : 1 on its liquid, under READABLE.
        let t = theme("lava");
        let (liquid, dim) = (t.role(Role::Liquid), t.role(Role::Dim));
        let buf = float_row(&t, &[(" ", liquid, liquid); 3], "thu", dim);
        assert_eq!(buf[(0, 0)].fg, dim);
        assert!(!buf[(0, 0)].modifier.contains(Modifier::BOLD));
        // Over wax it leaves (and stays unbolded: still a quiet line).
        let wax = t.role(Role::WaxCool);
        let buf = float_row(&t, &[(" ", wax, wax); 3], "thu", dim);
        assert_ne!(buf[(0, 0)].fg, dim);
        assert!(!buf[(0, 0)].modifier.contains(Modifier::BOLD));
        // Paper's `dim` reads 2.85 : 1 on its liquid: under the least any
        // glyph may read, so it takes the dark ink.
        let t = theme("paper");
        let (liquid, dim) = (t.role(Role::Liquid), t.role(Role::Dim));
        let buf = float_row(&t, &[(" ", liquid, liquid); 3], "thu", dim);
        assert_eq!(buf[(0, 0)].fg, t.floating_inks().1);
    }

    /// lava-1xk.31: with see-through cell backgrounds the wax behind text
    /// shows darker than it is (the user's Ghostty: 0.75), so mid wax
    /// takes the light ink there and the dark one on an opaque terminal.
    #[test]
    fn see_through_backgrounds_are_measured_as_shown() {
        let t = theme("lava");
        let (mid, text) = (t.role(Role::WaxMid), t.role(Role::Text));
        let (light, dark) = t.floating_inks();
        let row = [(" ", mid, mid); 3];
        assert_eq!(float_row_on(&t, &row, "abc", text, false)[(0, 0)].fg, dark);
        assert_eq!(float_row_on(&t, &row, "xyz", text, true)[(0, 0)].fg, light);
    }

    /// Every glyph reads at least 3 : 1 against what's behind it, over
    /// any grey, in any palette with colours, from any ink before.
    #[test]
    fn every_glyph_clears_the_least_contrast() {
        for name in [
            "lava",
            "ultraviolet",
            "abyss",
            "toxic",
            "synthwave",
            "mono",
            "paper",
        ] {
            let t = theme(name);
            for role in [Role::Text, Role::Dim, Role::Accent] {
                for translucent in [false, true] {
                    let ink = t.role(role);
                    let opacity = translucent.then_some(0.75);
                    let walk = (0..=255u8).step_by(5).chain((0..=255u8).rev().step_by(7));
                    for g in walk {
                        let row = [(" ", grey(g), grey(g)); 2];
                        let buf = float_row_on(&t, &row, "ab", ink, translucent);
                        let fg = t.luminance(buf[(0, 0)].fg).unwrap();
                        let bg = t.shown_luminance(buf[(0, 0)].bg, opacity).unwrap();
                        let c = theme::contrast(fg, bg);
                        assert!(
                            c >= LARGE - 0.01,
                            "{name} {role:?} {translucent} grey {g}: {c:.2}"
                        );
                    }
                }
            }
        }
    }

    /// Each stack on the lava keeps its own memory: drawing another
    /// stack between two frames doesn't wipe it (it used to: hysteresis
    /// then only held for the last stack drawn).
    #[test]
    fn stacks_keep_their_own_memory() {
        let t = theme("lava");
        let text = t.role(Role::Text);
        let (_, dark) = t.floating_inks();
        let draw = |x: u16, g: u8| {
            let area = Rect::new(0, 0, 20, 1);
            let mut buf = Buffer::empty(area);
            for p in area.positions() {
                buf[p].set_symbol(" ").set_fg(grey(g)).set_bg(grey(g));
            }
            let mut scratch = Buffer::empty(Rect::new(x, 0, 3, 1));
            scratch.set_string(x, 0, "abc", Style::new().fg(text));
            float(&mut buf, &scratch, t.role(Role::Dim), &t, None);
            buf[(x, 0)].fg
        };
        assert_eq!(draw(0, 150), dark);
        assert_eq!(draw(10, 150), dark);
        // Both in the band now (afresh, light would win there): both
        // keep dark.
        assert_eq!(draw(0, 108), dark);
        assert_eq!(draw(10, 108), dark);
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

    /// lava-98n: on see-through cell backgrounds a block glyph never puts
    /// the lamp's wax in the background (it would show darker than the
    /// wax around it): the other half is what's behind the wax.
    #[test]
    fn translucent_cells_keep_wax_out_of_block_glyph_backgrounds() {
        let t = theme("lava");
        let (liquid, wax, text) = (
            t.role(Role::Liquid),
            t.role(Role::WaxMid),
            t.role(Role::Text),
        );
        let lamp = [("▄", wax, liquid), ("▀", wax, liquid), ("█", wax, liquid)];
        let buf = float_row_on(&t, &lamp, "▀▄▀", text, true);
        for x in 0..3 {
            let c = &buf[(x, 0)];
            assert_eq!(c.bg, liquid, "{x}: {c:?}");
            assert!(c.fg != liquid && c.fg != wax, "{x}: the stroke is ink");
        }
        assert_eq!(buf[(0, 0)].symbol(), "▀");
        assert_eq!(buf[(1, 0)].symbol(), "▄");
        assert_eq!(buf[(2, 0)].symbol(), "▀");
        // A whole-cell stroke covers the lamp either way.
        let buf = float_row_on(&t, &[("█", wax, liquid)], "█", text, true);
        assert_eq!(buf[(0, 0)].symbol(), "█");
    }
}
