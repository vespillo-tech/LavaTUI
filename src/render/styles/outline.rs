//! **outline**: just the wax surface, as a one-dot braille contour (2×4
//! dots per cell, so the line is thin and smooth). Dots are the wax pixels
//! that touch liquid; the contour is coloured by the wax temperature along
//! it. Reads the same in every colour depth, since it's all shape.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::cell::{braille, braille_bit};
use crate::render::{Canvas, Grid, Style, wax_heat};
use crate::sim::SURFACE;
use crate::theme::Ink;

pub struct Outline;

impl Style for Outline {
    fn name(&self) -> &'static str {
        "outline"
    }

    fn grid(&self) -> Grid {
        Grid::BRAILLE
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        for cy in 0..usize::from(area.height) {
            for cx in 0..usize::from(area.width) {
                let (mut bits, mut heat, mut light, mut dots) = (0u8, 0.0, 0.0, 0.0);
                for dy in 0..4 {
                    for dx in 0..2 {
                        let (x, y) = (2 * cx + dx, 4 * cy + dy);
                        if is_edge(c, x, y) {
                            bits |= braille_bit(dx, dy);
                            heat += wax_heat(c.at(x, y).temp);
                            light += c.light(x, y);
                            dots += 1.0;
                        }
                    }
                }
                let base = c.theme.color(c.backdrop(2 * cx, 4 * cy));
                let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                if bits == 0 {
                    cell.set_char(' ').set_bg(base);
                } else {
                    // Lifted toward hot so a thin line holds its own against the liquid.
                    let wax = Ink::Wax(0.3 + 0.7 * heat / dots);
                    let fg = c.theme.paint(wax).scale(light / dots).color();
                    cell.set_char(braille(bits)).set_fg(fg).set_bg(base);
                }
            }
        }
    }
}

/// A wax pixel with liquid on at least one side. Off-canvas counts as wax,
/// so wax touching the walls or floor (the pool) isn't outlined there.
fn is_edge(c: &Canvas, x: usize, y: usize) -> bool {
    let wax = |x: usize, y: usize| x >= c.width || y >= c.height || c.at(x, y).density >= SURFACE;
    wax(x, y)
        && !(wax(x.wrapping_sub(1), y)
            && wax(x + 1, y)
            && wax(x, y.wrapping_sub(1))
            && wax(x, y + 1))
}
