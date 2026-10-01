//! **braille**: filled wax at the finest grid a terminal has, 2×4 dots per
//! cell. The surface is an unbroken line of dots; inside, a fixed ordered
//! stipple thins from the dense, hot core toward the cooler skin, like an
//! engraving, so volume and temperature read from the dots alone, in any
//! colour depth. Colour follows the wax temperature.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{is_edge, quantise};
use crate::render::cell::{braille, braille_bit};
use crate::render::{Canvas, Grid, Style, bayer, smoothstep, wax_heat};
use crate::sim::SURFACE;
use crate::theme::Ink;

pub struct Braille;

/// Distinct stipple densities: few enough that a drifting blob only
/// re-stipples now and then.
const TONES: f32 = 6.0;

impl Style for Braille {
    fn name(&self) -> &'static str {
        "braille"
    }

    fn grid(&self) -> Grid {
        Grid::BRAILLE
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        for cy in 0..usize::from(area.height) {
            for cx in 0..usize::from(area.width) {
                let (mut bits, mut heat, mut wax) = (0u8, 0.0, 0.0);
                for dy in 0..4 {
                    for dx in 0..2 {
                        let (x, y) = (2 * cx + dx, 4 * cy + dy);
                        let s = c.at(x, y);
                        if s.density < SURFACE {
                            continue;
                        }
                        let h = wax_heat(s.temp);
                        heat += h;
                        wax += 1.0;
                        if is_edge(c, x, y) || tone(s.density, h, c.light(x, y)) > bayer(x, y) {
                            bits |= braille_bit(dx, dy);
                        }
                    }
                }
                let base = c.theme.color(c.backdrop(2 * cx, 4 * cy));
                let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                if bits == 0 {
                    cell.set_char(' ').set_bg(base);
                } else {
                    let fg = c.theme.color(Ink::Wax(quantise(heat / wax, 16.0)));
                    cell.set_char(braille(bits)).set_fg(fg).set_bg(base);
                }
            }
        }
    }
}

/// Share of dots lit at a wax pixel: a light stipple under the skin,
/// filling in to solid toward the core, sooner the hotter the wax.
fn tone(density: f32, heat: f32, light: f32) -> f32 {
    let depth = smoothstep((density - SURFACE) / (0.5 - 0.25 * heat));
    quantise((0.2 + 0.85 * depth) * light, TONES)
}
