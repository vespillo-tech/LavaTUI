//! **braille**: filled wax at the finest grid a terminal has, 2×4 dots per
//! cell. The surface is an unbroken line of dots; inside, a fixed ordered
//! stipple thins from the dense, hot core toward the cooler skin, like an
//! engraving, so volume and temperature read from the dots alone, in any
//! colour depth. Colour follows the wax temperature.

use ratatui::buffer::Buffer;

use super::{is_edge, quantise, stepped_heat};
use crate::render::cell::{braille, braille_dots, mark};
use crate::render::{Canvas, Grid, LampStyle, bayer, smoothstep, wax_heat};
use crate::sim::SURFACE;
use crate::theme::Ink;

pub struct Braille;

/// Distinct stipple densities: few enough that a drifting blob only
/// re-stipples now and then.
const TONES: f32 = 6.0;

impl LampStyle for Braille {
    const NAME: &'static str = "braille";
    const GRID: Grid = Grid::BRAILLE;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        c.for_each_cell(buf, |at, cell| {
            let (mut heat, mut wax) = (0.0, 0.0);
            let bits = braille_dots(at.cx, at.cy, |x, y| {
                let s = c.at(x, y);
                if s.density < SURFACE {
                    return false;
                }
                let h = wax_heat(s.temp);
                heat += h;
                wax += 1.0;
                is_edge(c, x, y) || tone(s.density, h) > bayer(x, y)
            });
            let dots = (bits != 0).then(|| {
                let fg = c.theme.color(Ink::Wax(stepped_heat(heat / wax)));
                (braille(bits), fg)
            });
            mark(cell, dots, at.base);
        });
    }
}

/// Share of dots lit at a wax pixel: a light stipple under the skin,
/// filling in to solid toward the core, sooner the hotter the wax.
fn tone(density: f32, heat: f32) -> f32 {
    let depth = smoothstep((density - SURFACE) / (0.5 - 0.25 * heat));
    quantise(0.2 + 0.85 * depth, TONES)
}
