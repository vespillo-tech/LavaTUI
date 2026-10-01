//! **ascii**: a classic density ramp, ` .:-=+*#%@`. One glyph per cell,
//! supersampled from two pixels. The faint outer glyphs trace the soft rim
//! just outside the surface; inside, glyphs get denser toward the core and
//! with heat, so shape and temperature both read even with no colour.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::render::{Canvas, Grid, Style, coverage, lit, wax_heat};
use crate::sim::SURFACE;
use crate::theme::{Ink, Role};

pub struct Ascii;

const RIM: [char; 4] = [' ', '.', ':', '-'];
const BODY: [char; 6] = ['=', '+', '*', '#', '%', '@'];
/// Density where the rim glyphs start, below the surface.
const RIM_FROM: f32 = 0.3;

impl Style for Ascii {
    fn name(&self) -> &'static str {
        "ascii"
    }

    fn grid(&self) -> Grid {
        Grid::HALF_BLOCK
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        for cy in 0..usize::from(area.height) {
            for cx in 0..usize::from(area.width) {
                let (a, b) = (c.at(cx, 2 * cy), c.at(cx, 2 * cy + 1));
                let density = 0.5 * (a.density + b.density);
                let heat = wax_heat(0.5 * (a.temp + b.temp));
                let backdrop = c.backdrop(cx, 2 * cy);
                let base = c.theme.color(backdrop);

                let light = 0.5 * (c.light(cx, 2 * cy) + c.light(cx, 2 * cy + 1));
                let ch = glyph(density, heat, light);
                let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                if ch == ' ' {
                    cell.set_char(' ').set_bg(base);
                    continue;
                }
                let wax = Ink::Wax(heat);
                let fg = if c.theme.blends() {
                    // Rim glyphs fade from the liquid into the wax colour.
                    let paint = c.theme.paint(backdrop);
                    let paint = paint.mix(wax, 0.35 + 0.65 * coverage(density));
                    paint.scale(light).color()
                } else if density < SURFACE {
                    c.theme.role(Role::Dim)
                } else {
                    c.theme.color(wax)
                };
                cell.set_char(ch).set_fg(fg).set_bg(base);
            }
        }
    }
}

fn glyph(density: f32, heat: f32, light: f32) -> char {
    if density < SURFACE {
        let t = ((density - RIM_FROM) / (SURFACE - RIM_FROM)).max(0.0);
        return RIM[((t * RIM.len() as f32) as usize).min(RIM.len() - 1)];
    }
    // Half depth into the wax, half temperature.
    let depth = ((density - SURFACE) / 0.45).clamp(0.0, 1.0);
    // Lit sides read denser, shadowed sides sparser.
    let level = lit(0.45 * depth + 0.55 * heat, light);
    BODY[((level * BODY.len() as f32) as usize).min(BODY.len() - 1)]
}
