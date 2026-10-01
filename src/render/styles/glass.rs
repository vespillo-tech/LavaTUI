//! **glass**: wax as glossy blown glass. The density field is treated as a
//! height map, so each blob is a dome with a surface normal; it's lit from
//! the upper left with a soft body shade, a sharp specular glint and a
//! warm rim light where the surface turns away (fresnel), and the body is
//! translucent over the liquid. Without blending the same shading becomes
//! shade glyphs (`░▒▓█`) with the glint in the text colour.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::quantise;
use crate::render::cell::half_block;
use crate::render::{Canvas, Grid, Style, coverage, wax_heat};
use crate::sim::SURFACE;
use crate::theme::{Ink, Role};

pub struct Glass;

/// Direction to the light (x right, y down, z toward the viewer).
const LIGHT: [f32; 3] = [-0.48, -0.62, 0.62];
/// Slope exaggeration: higher = more curved-looking domes.
const RELIEF: f32 = 2.6;
const SHINE: i32 = 24;

impl Style for Glass {
    fn name(&self) -> &'static str {
        "glass"
    }

    fn grid(&self) -> Grid {
        Grid::HALF_BLOCK
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        for cy in 0..usize::from(area.height) {
            for cx in 0..usize::from(area.width) {
                let (yt, yb) = (2 * cy, 2 * cy + 1);
                let base = c.theme.color(c.backdrop(cx, yt));
                let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                if c.theme.blends() {
                    half_block(cell, Some(pixel(c, cx, yt)), Some(pixel(c, cx, yb)), base);
                    continue;
                }
                let (a, b) = (shade(c, cx, yt), shade(c, cx, yb));
                let cover = 0.5 * (coverage(c.at(cx, yt).density) + coverage(c.at(cx, yb).density));
                if cover < 0.5 {
                    cell.set_char(' ').set_bg(base);
                    continue;
                }
                let heat = wax_heat(0.5 * (c.at(cx, yt).temp + c.at(cx, yb).temp));
                let (ch, ink) = if a.spec.max(b.spec) > 0.5 {
                    ('█', Ink::Role(Role::Text))
                } else {
                    let lit =
                        0.5 * (a.diffuse + b.diffuse) * 0.5 * (c.light(cx, yt) + c.light(cx, yb));
                    let ch = match lit {
                        l if l < 0.25 => '░',
                        l if l < 0.5 => '▒',
                        l if l < 0.75 => '▓',
                        _ => '█',
                    };
                    (ch, Ink::Wax(heat))
                };
                cell.set_char(ch).set_fg(c.theme.color(ink)).set_bg(base);
            }
        }
    }
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
    let heat = quantise(wax_heat(s.temp), 16.0);
    // Translucent body: more wax where it's lit, the liquid through it in shadow.
    let body = cover * (0.45 + 0.4 * sh.diffuse);
    backdrop
        .mix(Ink::Wax(heat), body)
        .scale(0.7 + 0.5 * sh.diffuse)
        .mix(Ink::Wax(1.0), cover * 0.75 * sh.rim)
        .mix(Ink::Role(Role::Text), cover * sh.spec)
        .scale(c.light(x, y))
        .color()
}
