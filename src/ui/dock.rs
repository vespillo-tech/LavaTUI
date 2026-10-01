//! Drawing the dock (`crate::dock`): the side panel (§4.5), the widgets on
//! the lava with their soft backing, and the one-line chip.

use ratatui::buffer::Buffer;
use ratatui::style::Style;

use crate::app::Model;
use crate::dock::{Backdrop, Look, WIDGETS};
use crate::theme::{ColorDepth, Role, TERMINAL_DEFAULT, Theme};
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

/// The backing behind widgets on the lava: full strength out to `CORE`
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

/// The widgets on the lava, on a soft backing. Under the stack and just
/// around it the lamp is veiled most of the way to its liquid (glyphs
/// cleared), and the veil fades out over the next cells, so the readout
/// sits in a calm pool that the wax melts into rather than a box. Below
/// truecolor (256 colours would snap the tints to greys) the backing is
/// plain liquid where it's at least half strength. `lamp` is the theme the lamp was drawn with (its
/// liquid may be flashing).
pub fn draw_on_lava(buf: &mut Buffer, stack: &Stack, model: &Model, lamp: &Theme) {
    let look = Look {
        backdrop: Backdrop::Lava,
        align: stack.align,
    };
    // Widgets draw into a scratch buffer, then onto the backing.
    let mut scratch = Buffer::empty(stack.rect);
    for p in &stack.items {
        WIDGETS[p.widget].draw(model, p.form, p.rect, look, &mut scratch);
    }

    let r = stack.rect;
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
