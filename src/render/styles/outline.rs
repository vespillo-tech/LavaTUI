//! **outline**: just the wax surface, as a one-dot braille contour (2×4
//! dots per cell, so the line is thin and smooth). Dots are the wax pixels
//! that touch liquid; the contour is coloured by the wax temperature along
//! it. Reads the same in every colour depth, since it's all shape.

use ratatui::buffer::Buffer;

use super::is_edge;
use crate::render::cell::{braille, braille_dots, mark};
use crate::render::{Canvas, Grid, LampStyle, wax_heat};
use crate::theme::Ink;

pub struct Outline;

impl LampStyle for Outline {
    const NAME: &'static str = "outline";
    const GRID: Grid = Grid::BRAILLE;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        c.for_each_cell(buf, |at, cell| {
            let (mut heat, mut dots) = (0.0, 0.0);
            let bits = braille_dots(at.cx, at.cy, |x, y| {
                let edge = is_edge(c, x, y);
                if edge {
                    heat += wax_heat(c.at(x, y).temp);
                    dots += 1.0;
                }
                edge
            });
            let line = (bits != 0).then(|| {
                // Lifted toward hot so a thin line holds its own against the liquid.
                let wax = Ink::Wax(0.3 + 0.7 * heat / dots);
                (braille(bits), c.theme.color(wax))
            });
            mark(cell, line, at.base);
        });
    }
}
