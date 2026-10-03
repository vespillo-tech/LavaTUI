//! Drawing the dock (`crate::dock`): the side panel (§4.5), the widgets on
//! the lava (floating, or on a soft backing), and the one-line chip.

use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::app::Model;
use crate::dock::{Backdrop, Backing, Look, TextInk, WIDGETS};
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
    // Widgets draw into a scratch buffer (kept between frames), then onto
    // the lamp.
    SCRATCH.with_borrow_mut(|scratch| {
        scratch.resize(stack.rect);
        scratch.reset();
        for p in &stack.items {
            WIDGETS[p.widget].draw(model, p.form, p.rect, look, scratch);
        }
        let text = model.settings.dock.text;
        match model.settings.dock.backing {
            Backing::None => float(
                buf,
                scratch,
                model.theme.role(Role::Dim),
                lamp,
                model.cell_opacity(),
                text,
            ),
            Backing::Soft => soft(buf, scratch, lamp, text),
        }
    });
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

/// A word takes another ink once that's been wanted this many frames
/// running (≈ 0.1 s at 30 fps; meanwhile letters that fall short are
/// knocked back): a line of synthwave's grid sweeping under it isn't wax.
/// At once where nothing can be knocked back and a letter reads below
/// [`LARGE`].
const SETTLE: u8 = 4;

/// A word this long or longer whose letters all read in the other ink is
/// on wax; a shorter one may be on a line of the backdrop (synthwave's
/// grid) unless a cell beside it reads in that ink too.
const SLIVER: usize = 3;

/// A glyph follows the ink of the one before it in its word (where light
/// and dark read about as well) only while that reads at least this.
const FOLLOW: f32 = 3.3;

/// How far a quiet word (`dim`: the lines around the current lyric, the
/// words still to sing) in the other ink moves from it toward the
/// palette's own text ink, so it stays quieter than the words around it
/// over bright wax too; plain other ink where that would read too little.
const QUIET: f32 = 0.3;

/// The steps a letter's backdrop is knocked back by (of the way to the
/// ink's opposite), smallest first: the first that makes the letter read
/// at its bar. Few and fixed, so a knocked-back cell holds still while
/// the wax under it drifts a little.
const KNOCK: [f32; 5] = [0.35, 0.5, 0.65, 0.8, 1.0];

/// A floating glyph's ink: its own (the widget's role colours), or the
/// palette's light or dark one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ink {
    Own,
    Light,
    Dark,
}

/// The one ink `dock.text` puts on every glyph, if it asks for one (and
/// there are colours to choose: with none, glyphs keep their own).
fn fixed_ink(text: TextInk, lamp: &Theme) -> Option<Ink> {
    match text {
        _ if !lamp.has_color() => None,
        TextInk::Auto => None,
        TextInk::Light => Some(Ink::Light),
        TextInk::Dark => Some(Ink::Dark),
    }
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
    /// Its ink, and how far up its tone's ladder ([`Family::rung`]; text
    /// only: a step on that tone's).
    ink: Ink,
    tone: Tone,
    rung: u8,
    /// The ink (and rung) it wanted instead, if any, and for how many
    /// frames running (text): a change waits, so a line of the backdrop
    /// sweeping under a glyph (synthwave's grid) doesn't make it blink.
    next: Option<(Ink, u8)>,
    waited: u8,
}

/// What [`float`] keeps between frames: the glyphs' memory and its
/// scratch space, so drawing allocates nothing once warm.
#[derive(Default)]
struct Floating {
    /// Every stack's glyphs' memory, by cell.
    inks: Inks,
    /// The stack being drawn's, taken out of `inks` for the frame.
    last: Inks,
    puts: Vec<Put>,
    decided: Vec<Option<Decided>>,
    seen: Vec<bool>,
    digit: Vec<usize>,
    word: Vec<Letter>,
}

thread_local! {
    /// Drawing runs on one thread.
    static FLOATING: RefCell<Floating> = RefCell::default();
    /// The widgets on the lava draw into this first.
    static SCRATCH: RefCell<Buffer> = RefCell::new(Buffer::empty(Rect::ZERO));
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
    /// follows `left`, the glyph before it in the stroke.
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

/// What a text glyph is in its line, read off how the widget drew it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    /// `text`, or any bold ink: sung words, a title, the time.
    Strong,
    /// Bold `accent`: the lyrics' word being sung.
    Accent,
    /// `dim`, not bold: the lines around the current lyric, the words
    /// still to sing, an artist.
    Quiet,
}

/// One letter of a floating word, as [`float`] weighs it.
#[derive(Debug, Clone, Copy)]
struct Letter {
    /// Its cell, as an index into the stack's area.
    i: usize,
    tone: Tone,
    /// The luminance displayed behind it, if that can be told (with
    /// colours: then its own ink's luminance and bar, and for a quiet
    /// letter its own ink lifted halfway to `text`).
    behind: Option<f32>,
    own_l: f32,
    bar: f32,
    /// Whether its own ink reads at its bar on the plain liquid (paper's
    /// `dim` doesn't).
    steady: bool,
    lift: Option<(Color, f32)>,
    memory: Option<Memory>,
}

/// One way to ink a letter: one rung of [`Family::rung`]'s ladders.
#[derive(Debug, Clone, Copy)]
struct Rung {
    /// `None`: the widget's own ink.
    fg: Option<Color>,
    lum: f32,
    bar: f32,
    underline: bool,
}

/// The inks a floating word can take: the widget's own (`Ink::Own`), or
/// the palette's other one of light and dark, the one its `text` isn't
/// (`other`: dark over bright wax, on dark palettes). In each, a tone has
/// a short ladder of inks, the quietest first ([`Family::rung`]).
struct Family {
    /// The palette's text ink, and its luminance.
    text: (Color, f32),
    other: Ink,
    other_ink: (Color, f32),
    /// The other ink a little toward `text`, for quiet words.
    quiet: Option<(Color, f32)>,
    /// The lamp's liquid, what its own inks are made to read on.
    liquid: Color,
    /// Whether the other ink's letters can be knocked back toward `text`
    /// while a change back waits (colours blend, on opaque cells).
    brightens: bool,
    /// Whether its own inks' letters that fall short are knocked back
    /// toward the liquid rather than the word stepping up its ladder: but
    /// for a light lamp on see-through cells, which show a background no
    /// lighter than the window (brightened cells come out as boxes).
    /// Where colours don't blend, they sit on the plain liquid.
    knocks: bool,
}

/// The most rungs a ladder has.
const RUNGS: u8 = 3;

impl Family {
    /// Rung `r` of `letter`'s ladder in `ink` (`None` past its end, or
    /// where the theme has no such shade). In its own inks: its colour;
    /// for a quiet letter then that lifted halfway to `text`; then `text`
    /// (not for quiet letters where they're knocked back), underlined for
    /// the word being sung. In the other ink: for a quiet
    /// letter first [`QUIET`]'s shade; then the ink itself, underlined for
    /// the word being sung.
    fn rung(&self, letter: &Letter, ink: Ink, r: u8) -> Option<Rung> {
        let quiet = letter.tone == Tone::Quiet;
        let at = |(fg, lum): (Color, f32)| Rung {
            fg: Some(fg),
            lum,
            bar: if quiet { letter.bar } else { READABLE },
            underline: letter.tone == Tone::Accent,
        };
        match (ink, r, quiet) {
            (Ink::Own, 0, _) => Some(Rung {
                fg: None,
                lum: letter.own_l,
                bar: letter.bar,
                underline: false,
            }),
            (Ink::Own, 1, true) => letter.lift.map(at),
            (Ink::Own, 1, false) => Some(at(self.text)),
            // Where backdrops are knocked back, a quiet word never
            // takes the sung words' `text`: it stays a shade.
            (Ink::Own, 2, true) => (!self.knocks).then(|| at(self.text)),
            (_, 0, true) => self.quiet.map(at),
            (_, 0, false) | (_, 1, true) => Some(at(self.other_ink)),
            _ => None,
        }
    }
}

/// How far `contrast` falls short of `bar`, in log steps (0 when it
/// clears it): a word's ink is the one its letters fall least short in.
fn short(contrast: f32, bar: f32) -> f32 {
    (bar / contrast).ln().max(0.0)
}

/// No backing: the widgets' glyphs float on the lamp. Every cell keeps the
/// lamp's colours; only the cells a glyph takes change (spaces leave the
/// lamp showing, but for the gaps between words: [`Put::Gap`]). Block
/// glyphs (the clock faces) are composited pixel by pixel over half-block
/// lamps, so wax runs right up to each stroke. Text is bold (dim lines
/// aside).
///
/// Adaptive contrast against what the terminal displays right behind each
/// glyph ([`behind`]; `opacity`: see-through cell backgrounds,
/// [`Theme::shown_luminance`]). Text takes its ink per *word*
/// ([`float_word`]): the widget's own colours, or the palette's other ink
/// of light and dark, whichever its letters read better in, so a word
/// never splits and the lyrics' sung / being sung / still to come stay
/// apart. A letter that would still read under [`LARGE`] gets the cell
/// behind it knocked back toward the ink's opposite. Block glyphs choose
/// per glyph: their own ink while that reads ≥ [`LARGE`], else the better
/// of the light and dark inks, a big digit taking one ink wherever that
/// reads ≥ [`LARGE`]. Both keep [`STICKY`] hysteresis, and a change waits
/// a frame ([`Memory`]) unless the ink it has reads below [`LARGE`].
/// `dim` is the ink of secondary lines, drawn without bold.
///
/// `text` (`dock.text`) light or dark skips all that: every glyph takes
/// that ink, whatever is behind it (and nothing is remembered).
fn float(
    buf: &mut Buffer,
    scratch: &Buffer,
    dim: Color,
    lamp: &Theme,
    opacity: Option<f32>,
    text: TextInk,
) {
    FLOATING.with_borrow_mut(|f| f.float(buf, scratch, dim, lamp, opacity, text));
}

impl Floating {
    fn float(
        &mut self,
        buf: &mut Buffer,
        scratch: &Buffer,
        dim: Color,
        lamp: &Theme,
        opacity: Option<f32>,
        text: TextInk,
    ) {
        let fixed = fixed_ink(text, lamp);
        let translucent = opacity.is_some();
        let area = scratch.area.intersection(buf.area);
        let (w, h) = (usize::from(area.width), usize::from(area.height));
        let at = |i: usize| Position::new(area.x + (i % w) as u16, area.y + (i / w) as u16);

        let puts = &mut self.puts;
        puts.clear();
        puts.resize(w * h, Put::Nothing);
        for (i, put) in puts.iter_mut().enumerate() {
            let from = &scratch[at(i)];
            if from.bg != TERMINAL_DEFAULT || graphics::is_placeholder(from.symbol()) {
                // A picture (album art) brings its own background (and a
                // kitty placeholder's ink is its image id: never touched).
                buf[at(i)].set_symbol(from.symbol()).set_style(from.style());
            } else if from.symbol() != " " {
                let block = ink_halves(from.symbol()).is_some();
                *put = Put::Glyph { block };
            }
        }
        let is_text = |p: Put| p == Put::Glyph { block: false };
        for i in 1..puts.len().saturating_sub(1) {
            let inside = i % w != 0 && i % w != w - 1;
            if inside && puts[i] == Put::Nothing && is_text(puts[i - 1]) && is_text(puts[i + 1]) {
                puts[i] = Put::Gap;
            }
        }

        let (light, dark) = lamp.floating_inks();
        // Each stack on the lava is drawn on its own: take only this one's
        // memory (and drop what's off the screen now).
        let last = &mut self.last;
        last.clear();
        self.inks.retain(|&(x, y), ink| {
            let p = Position::new(x, y);
            if area.contains(p) {
                last.insert((x, y), *ink);
            }
            buf.area.contains(p) && !area.contains(p)
        });
        let (light_l, dark_l) = (lamp.luminance(light), lamp.luminance(dark));
        // Text's other ink: the one of light and dark the palette's text
        // isn't.
        let text_ink = match lamp.role(Role::Text) {
            TERMINAL_DEFAULT => Color::White,
            c => c,
        };
        let ((text, text_l), (other_c, other_l), other) = if text_ink == light {
            ((light, light_l), (dark, dark_l), Ink::Dark)
        } else {
            ((dark, dark_l), (light, light_l), Ink::Light)
        };
        let shade = |c: Option<Color>| c.and_then(|c| Some((c, lamp.luminance(c)?)));
        let family = match (text_l, other_l) {
            (Some(text_l), Some(other_l)) => Some(Family {
                text: (text, text_l),
                other,
                other_ink: (other_c, other_l),
                quiet: shade(lamp.toward(other_c, text, QUIET)),
                knocks: !lamp.blends() || other_l < text_l || !translucent,
                liquid: lamp.role(Role::Liquid),
                brightens: lamp.blends() && !translucent,
            }),
            _ => None,
        };
        // Widgets use a few role inks: remember the last one, with how
        // well it reads on the plain liquid and its shade halfway to text.
        let liquid_l = lamp.shown_luminance(lamp.role(Role::Liquid), opacity);
        let mut own = (TERMINAL_DEFAULT, None, f32::MAX, None);
        let mut own_lum = |c: Color| {
            if own.0 != c {
                let l = lamp.luminance(c);
                let calm = match (l, liquid_l) {
                    (Some(o), Some(q)) => theme::contrast(o, q),
                    _ => f32::MAX,
                };
                own = (c, l, calm, shade(lamp.toward(c, text, 0.5)));
            }
            (own.1, own.2, own.3)
        };
        let accent = lamp.role(Role::Accent);

        // Block glyphs' inks, painted once the digits are settled.
        let decided = &mut self.decided;
        decided.clear();
        decided.resize(puts.len(), None);
        // The ink of the block glyph before, while in the same stroke.
        let mut left = None;
        for (i, &put) in puts.iter().enumerate() {
            let pos = at(i);
            let to = &mut buf[pos];
            match put {
                Put::Nothing | Put::Glyph { block: false } => {
                    left = None;
                }
                Put::Gap => {
                    left = None;
                    match halves(to.symbol(), to.fg, to.bg) {
                        None => {
                            to.set_char(' ');
                        }
                        // A half block between two letters: the letters'
                        // own backing (what they're painted on), so a line
                        // reads as one strip, never `thu▄1`.
                        Some(_) if !translucent && lamp.has_color() => {
                            let bg = under(to, lamp);
                            to.set_char(' ').set_fg(bg).set_bg(bg);
                        }
                        Some(_) => {}
                    }
                }
                Put::Glyph { block: true } => {
                    let from = &scratch[pos];
                    let (own_l, _, _) = own_lum(from.fg);
                    let reads = match (own_l, light_l, dark_l, behind(to, true, lamp, opacity)) {
                        (Some(o), Some(l), Some(d), Some(b)) => {
                            Some([o, l, d].map(|x| theme::contrast(x, b)))
                        }
                        _ => None,
                    };
                    let ch = from.symbol().chars().next().unwrap_or(' ');
                    let memory = last.get(&(pos.x, pos.y)).filter(|m| m.glyph == ch).copied();
                    let glyph = Glyph {
                        reads,
                        bars: [LARGE; 3],
                        was: memory.map(|m| m.ink),
                    };
                    let want =
                        fixed.unwrap_or_else(|| glyph.ink(if i % w == 0 { None } else { left }));
                    // A change shows once it's wanted two frames running,
                    // or at once if the ink it has reads below the least.
                    let ink = match (memory, reads) {
                        _ if fixed.is_some() => want,
                        (Some(m), Some(r))
                            if want != m.ink
                                && m.next != Some((want, 0))
                                && r[m.ink as usize] >= LARGE =>
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
        // A big digit (block glyphs joined up, down and across) takes the
        // ink most of its cells chose wherever that still reads ≥ LARGE
        // there: one ink, unless a stroke straddles pale wax and dark
        // liquid.
        let seen = &mut self.seen;
        seen.clear();
        seen.resize(puts.len(), false);
        let digit = &mut self.digit;
        for start in 0..puts.len() {
            if fixed.is_some() || seen[start] || puts[start] != (Put::Glyph { block: true }) {
                continue;
            }
            digit.clear();
            digit.push(start);
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
            for &i in digit.iter() {
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
            if d.reads.is_some() && fixed.is_none() {
                let next = (d.want != d.ink).then_some((d.want, 0));
                let memory = Memory {
                    glyph: d.glyph,
                    ink: d.ink,
                    tone: Tone::Strong,
                    rung: 0,
                    next,
                    waited: 0,
                };
                self.inks.insert((pos.x, pos.y), memory);
            }
            let from = &scratch[pos];
            let fg = match d.ink {
                Ink::Own => from.fg,
                Ink::Light => light,
                Ink::Dark => dark,
            };
            let mut marks = from.modifier & Modifier::UNDERLINED;
            if from.fg != dim || from.modifier.contains(Modifier::BOLD) {
                marks |= Modifier::BOLD;
            }
            paint(
                &mut buf[pos],
                from.symbol(),
                fg,
                marks,
                None,
                lamp,
                translucent,
            );
        }

        // Text, word by word.
        let word = &mut self.word;
        for y in 0..h {
            let mut x = 0;
            while x < w {
                if !is_text(puts[y * w + x]) {
                    x += 1;
                    continue;
                }
                word.clear();
                // What shows just left and right of the word, in the stack
                // (`Some(None)`: a cell whose colour can't be told).
                let flank =
                    |x: usize| (x < w).then(|| behind(&buf[at(y * w + x)], false, lamp, opacity));
                let left = x.checked_sub(1).and_then(flank);
                while x < w && is_text(puts[y * w + x]) {
                    let i = y * w + x;
                    let pos = at(i);
                    let from = &scratch[pos];
                    let bold = from.fg != dim || from.modifier.contains(Modifier::BOLD);
                    let tone = match from.fg {
                        _ if !bold => Tone::Quiet,
                        c if c == accent && from.modifier.contains(Modifier::BOLD) => Tone::Accent,
                        _ => Tone::Strong,
                    };
                    let (own_l, calm, lift) = own_lum(from.fg);
                    let shown = behind(&buf[pos], false, lamp, opacity);
                    let (behind, own_l) = match (own_l, shown) {
                        (Some(o), Some(b)) => (Some(b), o),
                        _ => (None, 0.0),
                    };
                    let ch = from.symbol().chars().next().unwrap_or(' ');
                    let memory = last.get(&(pos.x, pos.y)).filter(|m| m.glyph == ch).copied();
                    word.push(Letter {
                        i,
                        tone,
                        behind,
                        own_l,
                        bar: READABLE.min(calm * SLACK).max(LARGE),
                        steady: calm >= READABLE.min(calm * SLACK).max(LARGE),
                        lift,
                        memory,
                    });
                    x += 1;
                }
                let flanks = [left, flank(x)];
                let inks = &mut self.inks;
                float_word(word, flanks, family.as_ref(), fixed, lamp, |letter, put| {
                    let pos = at(letter.i);
                    let from = &scratch[pos];
                    if letter.behind.is_some() && fixed.is_none() {
                        let glyph = from.symbol().chars().next().unwrap_or(' ');
                        let memory = Memory {
                            glyph,
                            ink: put.ink,
                            tone: letter.tone,
                            rung: put.rung,
                            next: put.next,
                            waited: put.waited,
                        };
                        inks.insert((pos.x, pos.y), memory);
                    }
                    let to = &mut buf[pos];
                    let knocked = put
                        .knock
                        .map(|knock| knock_back(lamp, under(to, lamp), knock, opacity));
                    let mut fg = put.fg.unwrap_or(from.fg);
                    let bg = match knocked {
                        Some(Some(bg)) => Some(bg),
                        // It can't be knocked back far enough (a light
                        // backdrop on see-through cells shows no lighter
                        // than the window): this letter alone takes the
                        // better of light and dark.
                        Some(None) => {
                            fg = letter.behind.map_or(fg, |b| better(lamp, b));
                            None
                        }
                        None => None,
                    };
                    // An underline (the lyrics' word being sung, without
                    // colours) stays.
                    let marks = put.marks | (from.modifier & Modifier::UNDERLINED);
                    paint(to, from.symbol(), fg, marks, bg, lamp, translucent);
                });
            }
        }
        // (16 colours: a bright wax colour as a background isn't always
        // that colour.)
        if !translucent && matches!(lamp.depth(), ColorDepth::TrueColor | ColorDepth::Ansi256) {
            solid_around(buf, area, |p| {
                !area.contains(p) || puts[at_index(area, p)] == Put::Nothing
            });
        }
    }
}

/// How [`float_word`] puts one letter.
struct PutLetter {
    ink: Ink,
    rung: u8,
    /// The ink and rung it wanted instead, and for how many frames
    /// running.
    next: Option<(Ink, u8)>,
    waited: u8,
    /// Its ink; `None`: its own.
    fg: Option<Color>,
    marks: Modifier,
    /// How to knock back the cell behind it, if it has to be.
    knock: Option<Knock>,
}

/// How to knock a letter's backdrop back ([`knock_back`]).
#[derive(Debug, Clone, Copy)]
enum Knock {
    /// Toward `to` until its ink, of luminance `ink`, reads at `bar`.
    Toward { ink: f32, bar: f32, to: Color },
    /// To the plain liquid (colours that don't blend).
    Liquid(Color),
}

/// Contrasts within this of each other count as the same.
const EVEN: f32 = 1e-4;

/// One floating word's inks (see [`float`]). In each of its own inks and
/// the other ink ([`Family`]), each tone in the word takes the first rung
/// of its ladder that all its letters read at the bar in (a rung below the
/// one it had needs [`RETURN`] more; where backdrops are knocked back
/// ([`Family::knocks`]), its own ink stays while that reads on the plain
/// liquid), else at least [`LARGE`]
/// in, else its last. The word
/// takes whichever of the two its letters fall least short in ([`short`];
/// the one it had last frame favoured by [`STICKY`]), then the fewer rungs
/// up, then the one it had, then its own; a change waits [`SETTLE`]
/// frames. Bold but for quiet letters. A letter still reading below
/// [`LARGE`] gets its cell knocked back (own inks toward the liquid's
/// side, the other ink toward `text`'s), or where colours don't blend,
/// takes the better of light and dark on its own. Letters whose backdrop
/// can't be told keep their own ink. `put` gets each letter's ink.
fn float_word(
    word: &[Letter],
    flanks: [Option<Option<f32>>; 2],
    family: Option<&Family>,
    fixed: Option<Ink>,
    lamp: &Theme,
    mut put: impl FnMut(&Letter, PutLetter),
) {
    let (light, dark) = lamp.floating_inks();
    let bold = |l: &Letter| match l.tone {
        Tone::Quiet => Modifier::empty(),
        _ => Modifier::BOLD,
    };
    let plain = |l: &Letter, ink| PutLetter {
        ink,
        rung: 0,
        next: None,
        waited: 0,
        fg: match ink {
            Ink::Own => None,
            Ink::Light => Some(light),
            Ink::Dark => Some(dark),
        },
        marks: bold(l),
        knock: None,
    };
    let (Some(family), None) = (family, fixed) else {
        for l in word {
            put(l, plain(l, fixed.unwrap_or(Ink::Own)));
        }
        return;
    };
    let measured = || word.iter().filter_map(|l| Some((l, l.behind?)));
    let count = |ink| {
        word.iter()
            .filter(|l| l.memory.is_some_and(|m| m.ink == ink))
            .count()
    };
    let was = match (count(Ink::Own), count(family.other)) {
        (0, 0) => None,
        (own, other) if other > own => Some(family.other),
        _ => Some(Ink::Own),
    };
    let tones = [Tone::Strong, Tone::Accent, Tone::Quiet];
    // Each tone's rung in `ink`.
    let rungs = |ink: Ink| {
        tones.map(|tone| {
            let of = || measured().filter(move |(l, _)| l.tone == tone);
            let exists = |r| of().all(|(l, _)| family.rung(l, ink, r).is_some());
            // Whether all its letters read at the bar on rung `r` (or at
            // least `LARGE`, `floor`).
            let reads = |r, floor: bool| {
                of().all(|(l, b)| {
                    let back =
                        was == Some(ink) && l.memory.is_some_and(|m| m.tone == tone && m.rung > r);
                    let lift = if back { RETURN } else { 1.0 };
                    family.rung(l, ink, r).is_some_and(|g| {
                        let bar = if floor { LARGE } else { g.bar };
                        theme::contrast(g.lum, b) >= bar * lift
                    })
                })
            };
            // Its own ink, where that reads on the plain liquid, stays
            // where backdrops are knocked back: the letters a line of the backdrop
            // or a blob makes fall short are knocked back, rather than
            // the word brightening each time one passes.
            if ink == Ink::Own && family.knocks && of().all(|(l, _)| l.steady) {
                return 0;
            }
            let mut there = (0..RUNGS).filter(|&r| exists(r));
            let last = there.clone().next_back().unwrap_or(0);
            there
                .clone()
                .find(|&r| reads(r, false))
                .or_else(|| there.find(|&r| reads(r, true)))
                .unwrap_or(last)
        })
    };
    let tone_rung =
        |rungs: &[u8; 3], l: &Letter| rungs[tones.iter().position(|&t| t == l.tone).unwrap_or(0)];
    // (falls short, rungs up) of `ink`.
    let weigh = |ink: Ink, rungs: &[u8; 3]| {
        let sticky = if was == Some(ink) { STICKY } else { 1.0 };
        measured().fold((0.0, 0), |(short_by, up), (l, b)| {
            let r = tone_rung(rungs, l);
            let g = family.rung(l, ink, r);
            let c = g.map_or(1.0, |g| theme::contrast(g.lum, b));
            let bar = g.map_or(READABLE, |g| g.bar);
            (short_by + short(c * sticky, bar), up + u32::from(r))
        })
    };
    let (first, second) = match was {
        Some(ink) if ink != Ink::Own => (ink, Ink::Own),
        _ => (Ink::Own, family.other),
    };
    let (first_rungs, second_rungs) = (rungs(first), rungs(second));
    let (a, b) = (weigh(first, &first_rungs), weigh(second, &second_rungs));
    // The other ink only where it reads on its own on all the word, and
    // for a word of a letter or two on a cell beside it too: wax, not a
    // line of the backdrop under a letter or two (those are knocked back),
    // and never a lighter knock-back on a dark lamp. Not where a backdrop
    // can't be told.
    let fits = |ink: Ink, rungs: &[u8; 3]| {
        let other = family.other_ink.1;
        let reads = |l: &Letter| {
            l.behind.is_some_and(|b| {
                family
                    .rung(l, ink, tone_rung(rungs, l))
                    .is_some_and(|g| theme::contrast(g.lum, b) >= LARGE)
            })
        };
        let flank = |f: &Option<f32>| f.is_some_and(|f| theme::contrast(other, f) >= LARGE);
        let wide = word.len() >= SLIVER || flanks.iter().flatten().any(flank);
        ink == Ink::Own || (word.iter().all(reads) && wide)
    };
    let lighter = b.0 < a.0 - EVEN || (b.0 <= a.0 + EVEN && b.1 < a.1);
    let want = match (
        lighter && fits(second, &second_rungs),
        fits(first, &first_rungs),
    ) {
        (true, _) => second,
        (false, true) => first,
        (false, false) => Ink::Own,
    };
    let rungs_of = |ink| {
        if ink == first {
            first_rungs
        } else {
            second_rungs
        }
    };
    // What it had: its ink and each tone's rung (a tone none of its
    // letters remember takes the one it would now).
    let had = was.map(|was| {
        let mut rungs = rungs_of(was);
        for (k, &tone) in tones.iter().enumerate() {
            let of = word.iter().filter(|l| l.tone == tone);
            let mut had = of.filter_map(|l| l.memory);
            if let Some(m) = had.find(|m| m.ink == was && m.tone == tone) {
                rungs[k] = m.rung;
            }
        }
        (was, rungs)
    });
    let wanted = (want, rungs_of(want));
    // Whether a letter reads below LARGE as `(ink, rungs)` would put it.
    let low = |(ink, rungs): (Ink, [u8; 3])| {
        measured().any(|(l, b)| {
            family
                .rung(l, ink, tone_rung(&rungs, l))
                .is_none_or(|g| theme::contrast(g.lum, b) < LARGE)
        })
    };
    // What each letter would change to, and for how many frames running
    // the word has wanted that, this one included.
    let goal = |l: &Letter| (want, tone_rung(&wanted.1, l));
    let waited = word
        .iter()
        .filter_map(|l| l.memory.filter(|m| m.next == Some(goal(l))))
        .map(|m| m.waited)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    // It keeps what it had for a while, if that still reads, or its
    // letters that don't are knocked back ([`Family::knocks`], for own
    // inks that read on the liquid; [`Family::brightens`]).
    let keep = had.filter(|&h| {
        let knocked = match h.0 {
            Ink::Own => family.knocks && word.iter().all(|l| l.steady),
            _ => family.brightens,
        };
        let safe = !low(h) || knocked;
        h != wanted && waited < SETTLE && safe
    });
    let (ink, rungs) = keep.unwrap_or(wanted);
    let waited = if keep.is_some() { waited } else { 0 };
    // Its own inks are made to read on the liquid: they're knocked back
    // toward it (the other ink); the other ink toward `text`.
    let toward = if ink == Ink::Own {
        family.other_ink.0
    } else {
        family.text.0
    };
    for l in word {
        let r = tone_rung(&rungs, l);
        let (Some(b), Some(g)) = (l.behind, family.rung(l, ink, r)) else {
            put(l, plain(l, Ink::Own));
            continue;
        };
        let mut marks = bold(l);
        if g.underline {
            marks |= Modifier::UNDERLINED;
        }
        let mut letter = PutLetter {
            ink,
            rung: r,
            next: keep.map(|_| goal(l)),
            waited,
            fg: g.fg,
            marks,
            knock: None,
        };
        if theme::contrast(g.lum, b) < LARGE {
            letter.knock = match (lamp.blends(), ink) {
                (true, _) => Some(Knock::Toward {
                    ink: g.lum,
                    bar: g.bar,
                    to: toward,
                }),
                // Without blending, its own inks' letter sits on the plain
                // liquid they're made for.
                (false, Ink::Own) => Some(Knock::Liquid(family.liquid)),
                // Nothing to knock back to: this letter alone takes the
                // better of light and dark.
                (false, _) => {
                    letter.fg = Some(better(lamp, b));
                    None
                }
            };
        }
        put(l, letter);
    }
}

/// The better reading of the palette's light and dark inks on a backdrop
/// of luminance `behind`.
fn better(lamp: &Theme, behind: f32) -> Color {
    let (light, dark) = lamp.floating_inks();
    let reads = |c| {
        lamp.luminance(c)
            .map_or(0.0, |l| theme::contrast(l, behind))
    };
    if reads(dark) > reads(light) {
        dark
    } else {
        light
    }
}

/// The background a text cell's `bg` is knocked back to so its letter
/// reads at the knock's bar: the first of [`KNOCK`]'s steps toward the
/// knock's colour that does, else the last if that reads at least
/// [`LARGE`]; or the plain liquid. `None` where it can't be done (a light
/// backdrop on see-through cells shows no lighter than the window).
fn knock_back(lamp: &Theme, bg: Color, knock: Knock, opacity: Option<f32>) -> Option<Color> {
    let (ink, bar, to) = match knock {
        Knock::Toward { ink, bar, to } => (ink, bar, to),
        Knock::Liquid(liquid) => return Some(liquid),
    };
    let reads = |c| {
        lamp.shown_luminance(c, opacity)
            .map_or(0.0, |l| theme::contrast(ink, l))
    };
    let mut knocked = None;
    for t in KNOCK {
        let c = lamp.toward(bg, to, t)?;
        knocked = Some(c);
        if reads(c) >= bar {
            break;
        }
    }
    knocked.filter(|&c| reads(c) >= LARGE)
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

/// Put a widget's `symbol` in `ink` into lamp cell `to`, with `marks` (bold,
/// underline); text on `knocked` if its backdrop was knocked back.
///
/// With `translucent` (see-through cell backgrounds, opaque glyphs) a
/// block glyph's other half is never the lamp's foreground pixel (wax),
/// which would show darker than the wax around it: it's the lamp cell's
/// background, what's behind the wax, as in [`crate::render::cell::half_block`].
/// The wax gives up half a cell beside the stroke; the stroke stays whole.
fn paint(
    to: &mut Cell,
    symbol: &str,
    ink: Color,
    marks: Modifier,
    knocked: Option<Color>,
    lamp: &Theme,
    translucent: bool,
) {
    let style = Style::new().add_modifier(marks);
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
        // Text: on what the cell showed behind its glyph (or that,
        // `knocked` back).
        _ => {
            let bg = knocked.unwrap_or_else(|| under(to, lamp));
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
fn soft(buf: &mut Buffer, scratch: &Buffer, lamp: &Theme, text: TextInk) {
    let (light, dark) = lamp.floating_inks();
    let fixed = fixed_ink(text, lamp).map(|ink| if ink == Ink::Light { light } else { dark });
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
            if let Some(ink) = fixed.filter(|_| !own_bg) {
                to.set_fg(ink);
            }
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
    use std::ops::Range;

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
            TextInk::Auto,
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

    /// lava-4ba: a word changes ink once the other has been wanted
    /// [`SETTLE`] frames running (meanwhile its letters are knocked back,
    /// so they read), and never back and forth.
    #[test]
    fn a_word_changes_ink_once_it_settles_never_back_and_forth() {
        let t = theme("lava");
        let text = t.role(Role::Text);
        let (_, dark) = t.floating_inks();
        let over = |g: u8| {
            let buf = float_row(&t, &[(" ", grey(g), grey(g)); 3], "abc", text);
            (buf[(0, 0)].fg, reads(&t, &buf[(0, 0)]))
        };
        // Own ink reads 6 : 1 over grey 80, 2.2 over 150 (dark: 6.6);
        // light and dark cross near 110 (≈ 3.8 : 1 each).
        assert_eq!(over(80).0, text);
        for _ in 1..SETTLE {
            let (fg, c) = over(150);
            assert_eq!(fg, text, "waits, knocked back");
            assert!(c >= READABLE, "{c}");
        }
        assert_eq!(over(150).0, dark, "then dark");
        for _ in 0..2 * SETTLE {
            assert_eq!(over(112).0, dark, "inside the band: no flip back");
        }
        // Over grey 95 dark still reads 3 : 1 and light 4.8: the change
        // waits, then shows.
        for _ in 1..SETTLE {
            assert_eq!(over(95).0, dark);
        }
        assert_eq!(over(95).0, text, "then light");
        assert_eq!(over(112).0, text, "inside the band: no flip back");
    }

    /// How well a floating cell's glyph reads on its background.
    fn reads(t: &Theme, cell: &Cell) -> f32 {
        theme::contrast(t.luminance(cell.fg).unwrap(), t.luminance(cell.bg).unwrap())
    }

    #[test]
    fn a_quiet_ink_stays_quiet_on_its_own_liquid() {
        // Lava's `dim` reads 3.6 : 1 on its liquid, under READABLE.
        let t = theme("lava");
        let (liquid, dim) = (t.role(Role::Liquid), t.role(Role::Dim));
        let buf = float_row(&t, &[(" ", liquid, liquid); 3], "thu", dim);
        assert_eq!(buf[(0, 0)].fg, dim);
        assert_eq!(buf[(0, 0)].bg, liquid, "nothing knocked back");
        assert!(!buf[(0, 0)].modifier.contains(Modifier::BOLD));
        // Over wax it stays a quiet line, `dim` and unbolded, never the
        // `text` the lines around it are in: the wax behind it is knocked
        // back toward the liquid until it reads.
        let wax = t.role(Role::WaxCool);
        let buf = float_row(&t, &[(" ", wax, wax); 3], "thu", dim);
        assert_eq!(buf[(0, 0)].fg, dim);
        assert!(t.darker(buf[(0, 0)].bg, wax));
        assert!(!buf[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(reads(&t, &buf[(0, 0)]) >= LARGE);
        // Paper's `dim` reads 2.85 : 1 on its liquid: under the least any
        // glyph may read, so it takes its shade halfway to `text`, still
        // lighter than `text`.
        let t = theme("paper");
        let (liquid, dim, text) = (t.role(Role::Liquid), t.role(Role::Dim), t.role(Role::Text));
        let buf = float_row(&t, &[(" ", liquid, liquid); 3], "thu", dim);
        let c = &buf[(0, 0)];
        assert!(c.fg != dim && c.fg != text, "{c:?}");
        assert!(t.darker(text, c.fg) && t.darker(c.fg, dim), "{c:?}");
        assert_eq!(c.bg, liquid, "nothing knocked back");
        assert!(reads(&t, c) >= LARGE);
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
            float(
                &mut buf,
                &scratch,
                t.role(Role::Dim),
                &t,
                None,
                TextInk::Auto,
            );
            buf[(x, 0)].fg
        };
        assert_eq!(draw(0, 150), dark);
        assert_eq!(draw(10, 150), dark);
        // Both in the band now (afresh, light would win there): both
        // keep dark.
        assert_eq!(draw(0, 108), dark);
        assert_eq!(draw(10, 108), dark);
    }

    /// `dock.text` light / dark: one ink everywhere, over wax and liquid
    /// alike, text and big digits; with no colour, glyphs keep their own.
    #[test]
    fn light_and_dark_text_take_one_ink_whatever_is_behind() {
        let t = theme("lava");
        let (liquid, hot, text) = (
            t.role(Role::Liquid),
            t.role(Role::WaxHot),
            t.role(Role::Text),
        );
        let (light, dark) = t.floating_inks();
        let mut cells = vec![(" ", hot, hot); 3];
        cells.extend([(" ", liquid, liquid); 4]);
        let area = Rect::new(0, 0, 7, 1);
        let draw = |theme: &Theme, mode, s: &str| {
            let mut buf = Buffer::empty(area);
            for (x, &(symbol, fg, bg)) in cells.iter().enumerate() {
                buf[(x as u16, 0)].set_symbol(symbol).set_fg(fg).set_bg(bg);
            }
            let mut scratch = Buffer::empty(area);
            scratch.set_string(0, 0, s, Style::new().fg(text));
            float(&mut buf, &scratch, theme.role(Role::Dim), theme, None, mode);
            buf
        };
        for (mode, ink) in [(TextInk::Light, light), (TextInk::Dark, dark)] {
            for s in ["abc def", "███████"] {
                let buf = draw(&t, mode, s);
                for x in [0, 2, 4, 6] {
                    assert_eq!(buf[(x, 0)].fg, ink, "{mode:?} {s:?} {x}");
                }
            }
        }
        // Automatic: dark over the wax, its own on the liquid.
        let buf = draw(&t, TextInk::Auto, "abc def");
        assert_eq!((buf[(0, 0)].fg, buf[(4, 0)].fg), (dark, text));
        // No colour: the setting changes nothing.
        let none = Theme::new(Palette::by_name("lava").unwrap(), ColorDepth::None);
        let auto = draw(&none, TextInk::Auto, "abc def");
        for mode in [TextInk::Light, TextInk::Dark] {
            assert_eq!(draw(&none, mode, "abc def"), auto, "{mode:?}");
        }
    }

    /// Forget what floating glyphs had (each case starts afresh).
    fn forget() {
        FLOATING.with_borrow_mut(|f| f.inks.clear());
    }

    /// A lyric line, karaoke style: sung words bold `text`, the word being
    /// sung bold `accent`, the rest `dim`, floated on a lamp row of
    /// `cells` (see-through cell backgrounds when `translucent`).
    fn karaoke_row(
        t: &Theme,
        cells: &[Color],
        translucent: bool,
    ) -> (Buffer, Vec<(Range<u16>, Tone)>) {
        let area = Rect::new(0, 0, cells.len() as u16, 1);
        let mut buf = Buffer::empty(area);
        for (x, &c) in cells.iter().enumerate() {
            buf[(x as u16, 0)].set_symbol(" ").set_fg(c).set_bg(c);
        }
        let sung = t.text(Role::Text).add_modifier(Modifier::BOLD);
        let now = t.text(Role::Accent).add_modifier(Modifier::BOLD);
        let ahead = t.text(Role::Dim);
        let words = [
            ("cooling", sung, Tone::Strong),
            ("at", sung, Tone::Strong),
            ("the", now, Tone::Accent),
            ("top", ahead, Tone::Quiet),
            ("a", ahead, Tone::Quiet),
        ];
        let mut scratch = Buffer::empty(area);
        let mut x = 0;
        let mut spans = Vec::new();
        for (word, style, tone) in words {
            scratch.set_string(x, 0, word, style);
            spans.push((x..x + word.len() as u16, tone));
            x += word.len() as u16 + 1;
        }
        let opacity = translucent.then_some(0.75);
        float(
            &mut buf,
            &scratch,
            t.role(Role::Dim),
            t,
            opacity,
            TextInk::Auto,
        );
        (buf, spans)
    }

    /// lava-4ba: lyrics floating over a line of the backdrop (synthwave's
    /// grid, in the accent's colour) under a letter of each word keep
    /// their three parts apart, each word in one ink, every letter
    /// readable: the line is knocked back behind the letters it crosses.
    #[test]
    fn karaoke_keeps_its_parts_over_a_line_of_the_backdrop() {
        for name in [
            "lava",
            "abyss",
            "synthwave",
            "mono",
            "paper",
            "toxic",
            "ultraviolet",
        ] {
            for translucent in [false, true] {
                forget();
                let t = theme(name);
                let (liquid, line) = (t.role(Role::Liquid), t.role(Role::Accent));
                // `cooling at the top a`: the line under the 2nd letter of
                // `cooling`, `at`'s `t`, `the`'s `h`, `top`'s `o` and `a`.
                let mut cells = vec![liquid; 22];
                for x in [1, 9, 12, 16, 19] {
                    cells[x] = line;
                }
                let (buf, spans) = karaoke_row(&t, &cells, translucent);
                let opacity = translucent.then_some(0.75);
                let mut looks = Vec::new();
                for (span, tone) in spans {
                    let first = &buf[(span.start, 0)];
                    let look = (first.fg, first.modifier);
                    for x in span {
                        let c = &buf[(x, 0)];
                        assert_eq!(
                            (c.fg, c.modifier),
                            look,
                            "{name} {translucent} {x}: one ink a word"
                        );
                        let fg = t.luminance(c.fg).unwrap();
                        let bg = t.shown_luminance(c.bg, opacity).unwrap();
                        let reads = theme::contrast(fg, bg);
                        assert!(
                            reads >= LARGE - 0.01,
                            "{name} {translucent} {x}: {reads:.2}"
                        );
                    }
                    looks.push((tone, look));
                }
                // Sung, being sung and still to come look different (paper
                // on see-through cells, its liquid showing mid grey: by
                // weight and underline only).
                let of = |tone| looks.iter().find(|l| l.0 == tone).unwrap().1;
                let (sung, now, ahead) = (of(Tone::Strong), of(Tone::Accent), of(Tone::Quiet));
                let colours = !(name == "paper" && translucent);
                assert!(
                    sung != now && now != ahead,
                    "{name} {translucent} {looks:?}"
                );
                assert!(
                    sung.0 != ahead.0 || !colours,
                    "{name} {translucent} {looks:?}"
                );
                assert!(sung.1.contains(Modifier::BOLD) && !ahead.1.contains(Modifier::BOLD));
                if name != "paper" {
                    // On a dark lamp the words keep their own colours.
                    assert_eq!(sung.0, t.role(Role::Text), "{name} {translucent}");
                    assert_eq!(now.0, t.role(Role::Accent), "{name} {translucent}");
                    assert_eq!(ahead.0, t.role(Role::Dim), "{name} {translucent}");
                    // `at`'s `t`, `text` on the line: knocked back where
                    // it wouldn't read.
                    let text = t.luminance(t.role(Role::Text)).unwrap();
                    let shown = t.shown_luminance(line, opacity).unwrap();
                    if theme::contrast(text, shown) < LARGE {
                        assert!(t.darker(buf[(9, 0)].bg, line), "{name} {translucent}");
                    }
                }
            }
        }
    }

    /// lava-4ba: over bright wax the lyrics take the dark ink, keeping
    /// their parts: sung bold, the word being sung bold and underlined,
    /// the words still to come a quieter shade, not bold.
    #[test]
    fn karaoke_over_bright_wax_keeps_its_parts_in_the_dark_ink() {
        for name in ["lava", "abyss", "synthwave", "mono"] {
            forget();
            let t = theme(name);
            let (_, dark) = t.floating_inks();
            let (buf, spans) = karaoke_row(&t, &[t.role(Role::WaxHot); 22], false);
            let at = |tone| &buf[(spans.iter().find(|s| s.1 == tone).unwrap().0.start, 0)];
            let (sung, now, ahead) = (at(Tone::Strong), at(Tone::Accent), at(Tone::Quiet));
            assert_eq!((sung.fg, now.fg), (dark, dark), "{name}");
            assert!(
                sung.modifier.contains(Modifier::BOLD)
                    && !sung.modifier.contains(Modifier::UNDERLINED)
            );
            assert!(
                now.modifier.contains(Modifier::BOLD | Modifier::UNDERLINED),
                "{name}"
            );
            assert!(
                ahead.fg != dark && !ahead.modifier.contains(Modifier::BOLD),
                "{name}"
            );
            assert!(reads(&t, ahead) >= LARGE, "{name}");
            assert_eq!(
                ahead.bg,
                t.role(Role::WaxHot),
                "{name}: nothing knocked back"
            );
        }
    }

    #[test]
    fn the_soft_backing_takes_light_or_dark_text_too() {
        let t = theme("lava");
        let (liquid, text) = (t.role(Role::Liquid), t.role(Role::Text));
        let (light, dark) = t.floating_inks();
        let area = Rect::new(0, 0, 3, 1);
        for (mode, ink) in [
            (TextInk::Auto, text),
            (TextInk::Light, light),
            (TextInk::Dark, dark),
        ] {
            let mut buf = Buffer::empty(area);
            for p in area.positions() {
                buf[p].set_symbol(" ").set_fg(liquid).set_bg(liquid);
            }
            let mut scratch = Buffer::empty(area);
            scratch.set_string(0, 0, "abc", Style::new().fg(text));
            soft(&mut buf, &scratch, &t, mode);
            assert_eq!(buf[(1, 0)].fg, ink, "{mode:?}");
        }
    }

    /// lava-4ba: a word keeps one ink; a letter that wouldn't read in it
    /// gets the cell behind it knocked back instead of an ink of its own.
    #[test]
    fn a_straddling_word_keeps_one_ink_and_every_letter_readable() {
        let t = theme("mono");
        let (liquid, text) = (t.role(Role::Liquid), t.role(Role::Text));
        let pale = Color::Rgb(230, 230, 230);
        let mut cells = vec![(" ", pale, pale)];
        cells.extend([(" ", liquid, liquid); 4]);
        let buf = float_row(&t, &cells, "focus", text);
        for x in 0..5 {
            assert_eq!(buf[(x, 0)].fg, text, "{x}: one ink for the word");
            assert!(reads(&t, &buf[(x, 0)]) >= READABLE, "{x}");
        }
        assert!(t.darker(buf[(0, 0)].bg, pale), "the pale wax knocked back");
        assert_eq!(buf[(1, 0)].bg, liquid, "the rest untouched");
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
