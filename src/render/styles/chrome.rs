//! **chrome**: wax as glossy blown glass. The density field is treated as a
//! height map, so each blob is a dome with a surface normal; it's lit from
//! the upper left with a soft body shade, a sharp specular glint and a
//! warm rim light where the surface turns away (fresnel), and the body is
//! translucent over the liquid. Without blending the same shading becomes
//! shade glyphs (`░▒▓█`) with the glint in the text colour.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use super::{quantise, stepped_heat};
use crate::render::cell::mark as mark_cell;
use crate::render::{Canvas, Grid, LampStyle, coverage, wax_heat};
use crate::sim::SURFACE;
use crate::theme::{Ink, Role};

pub struct Chrome;

/// Direction to the light (x right, y down, z toward the viewer).
const LIGHT: [f32; 3] = [-0.48, -0.62, 0.62];
/// Slope exaggeration: higher = more curved-looking domes.
const RELIEF: f32 = 2.6;
const SHINE: i32 = 24;

impl LampStyle for Chrome {
    const NAME: &'static str = "chrome";
    const GRID: Grid = Grid::HALF_BLOCK;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        if c.theme.blends() {
            c.draw_half_blocks(buf, |x, y| Some(pixel(c, x, y)));
            return;
        }
        c.for_each_cell(buf, |at, cell| {
            let mark = glyph(c, at.x, at.y).map(|(ch, ink)| (ch, c.theme.color(ink)));
            mark_cell(cell, mark, at.base);
        });
    }
}

/// A shade glyph for the cell whose top pixel is (`x`, `y`), from the same
/// shading as the blended look; `None` where the cell is mostly liquid.
fn glyph(c: &Canvas, x: usize, y: usize) -> Option<(char, Ink)> {
    let (top, bottom) = (c.at(x, y), c.at(x, y + 1));
    if 0.5 * (coverage(top.density) + coverage(bottom.density)) < 0.5 {
        return None;
    }
    let (a, b) = (shade(c, x, y), shade(c, x, y + 1));
    if a.spec.max(b.spec) > 0.5 {
        return Some(('█', Ink::Role(Role::Text)));
    }
    let ch = match 0.5 * (a.diffuse + b.diffuse) {
        l if l < 0.25 => '░',
        l if l < 0.5 => '▒',
        l if l < 0.75 => '▓',
        _ => '█',
    };
    Some((ch, Ink::Wax(wax_heat(0.5 * (top.temp + bottom.temp)))))
}

struct Shade {
    diffuse: f32,
    spec: f32,
    rim: f32,
}

/// Dome height from density: 0 at the surface, rising steeply then
/// flattening toward the core, like a drop of liquid.
fn height(c: &Canvas, x: usize, y: usize) -> f32 {
    let d = c.at(x.min(c.width - 1), y.min(c.height - 1)).density;
    ((d - SURFACE) / 0.5).clamp(0.0, 1.0).sqrt()
}

fn shade(c: &Canvas, x: usize, y: usize) -> Shade {
    let gx = height(c, x + 1, y) - height(c, x.saturating_sub(1), y);
    let gy = height(c, x, y + 1) - height(c, x, y.saturating_sub(1));
    let (nx, ny, nz) = (-gx * RELIEF, -gy * RELIEF, 1.0);
    let len = (nx * nx + ny * ny + nz * nz).sqrt();
    let n = [nx / len, ny / len, nz / len];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let l = normalise(LIGHT);
    let half = normalise([l[0], l[1], l[2] + 1.0]);
    Shade {
        diffuse: quantise(dot(n, l).max(0.0), 8.0),
        spec: quantise(dot(n, half).max(0.0).powi(SHINE), 6.0),
        rim: quantise((1.0 - n[2]).powi(2), 8.0),
    }
}

fn normalise(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / len, v[1] / len, v[2] / len]
}

fn pixel(c: &Canvas, x: usize, y: usize) -> Color {
    let s = c.at(x, y);
    let cover = coverage(s.density);
    let backdrop = c.theme.paint(c.backdrop(x, y));
    if cover == 0.0 {
        return backdrop.color();
    }
    let sh = shade(c, x, y);
    let heat = stepped_heat(wax_heat(s.temp));
    // Translucent body: more wax where it's lit, the liquid through it in shadow.
    let body = cover * (0.45 + 0.4 * sh.diffuse);
    backdrop
        .mix(Ink::Wax(heat), body)
        .scale(0.7 + 0.5 * sh.diffuse)
        .mix(Ink::Wax(1.0), cover * 0.75 * sh.rim)
        .mix(Ink::Role(Role::Text), cover * sh.spec)
        .color()
}
