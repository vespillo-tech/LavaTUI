//! **ascii**: a classic density ramp, ` .:-=+*#%@`. One glyph per cell,
//! supersampled from two pixels. The faint outer glyphs trace the soft rim
//! just outside the surface; inside, glyphs get denser toward the core and
//! with heat, so shape and temperature both read even with no colour.

use ratatui::buffer::Buffer;

use crate::render::cell::{blank, glyph as set_glyph};
use crate::render::{Canvas, Grid, LIQUID, LampStyle, coverage, wax_heat};
use crate::sim::SURFACE;
use crate::theme::{Ink, Role};

pub struct Ascii;

const RIM: [char; 4] = [' ', '.', ':', '-'];
const BODY: [char; 6] = ['=', '+', '*', '#', '%', '@'];
/// Density where the rim glyphs start, below the surface.
const RIM_FROM: f32 = 0.3;

impl LampStyle for Ascii {
    const NAME: &'static str = "ascii";
    const GRID: Grid = Grid::HALF_BLOCK;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        c.for_each_cell(buf, |at, cell| {
            let (a, b) = (c.at(at.x, at.y), c.at(at.x, at.y + 1));
            let density = 0.5 * (a.density + b.density);
            let heat = wax_heat(0.5 * (a.temp + b.temp));
            let ch = glyph(density, heat);
            if ch == ' ' {
                blank(cell, at.base);
                return;
            }
            let wax = Ink::Wax(heat);
            let fg = if c.theme.blends() {
                // Rim glyphs fade from the liquid into the wax colour.
                let paint = c.theme.paint(LIQUID);
                paint.mix(wax, 0.35 + 0.65 * coverage(density)).color()
            } else if density < SURFACE {
                c.theme.role(Role::Dim)
            } else {
                c.theme.color(wax)
            };
            set_glyph(cell, ch, fg, at.base);
        });
    }
}

fn glyph(density: f32, heat: f32) -> char {
    if density < SURFACE {
        let t = ((density - RIM_FROM) / (SURFACE - RIM_FROM)).max(0.0);
        return RIM[((t * RIM.len() as f32) as usize).min(RIM.len() - 1)];
    }
    // Half depth into the wax, half temperature.
    let depth = ((density - SURFACE) / 0.45).clamp(0.0, 1.0);
    let level = 0.45 * depth + 0.55 * heat;
    BODY[((level * BODY.len() as f32) as usize).min(BODY.len() - 1)]
}
