//! **heatmap**: a thermal camera. Every pixel, wax or liquid, is coloured by
//! temperature on the liquid → cool → mid → hot ramp, so the warm base and
//! the cooling plume are visible as well as the wax. Wax gets a small boost
//! so its edges read against liquid of the same temperature. Without
//! blending it becomes a shade ramp (`░▒▓█`), one level per cell.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use crate::render::cell::glyph;
use crate::render::{Canvas, Grid, LampStyle, bayer, coverage, lit, smoothstep};
use crate::sim::{SURFACE, ambient_temp};
use crate::theme::{Ink, Role, TERMINAL_DEFAULT};

pub struct Heatmap;

/// Distinct thermal levels.
const HEAT_BANDS: f32 = 32.0;
/// Where [`shade`] reaches the wax colours.
const WAX_BAND: f32 = 0.36;

impl LampStyle for Heatmap {
    const NAME: &'static str = "heatmap";
    const GRID: Grid = Grid::HALF_BLOCK;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        if c.theme.blends() {
            c.draw_half_blocks(buf, |x, y| Some(pixel(c, x, y)));
            return;
        }
        c.for_each_cell(buf, |at, cell| {
            let (x, yt, yb) = (at.x, at.y, at.y + 1);
            let h = 0.5 * (heat(c, x, yt) + heat(c, x, yb));
            // Light shades the wax bands only: on the liquid it would draw
            // the base glow as hard `░` stripes.
            let h = if h >= WAX_BAND {
                lit(h, 0.5 * (c.light(x, yt) + c.light(x, yb))).max(WAX_BAND)
            } else {
                h
            };
            let (ch, ink) = shade(h);
            let fg = if ch == ' ' {
                TERMINAL_DEFAULT
            } else {
                c.theme.color(ink)
            };
            glyph(cell, ch, fg, at.base);
        });
    }
}

/// Position on the thermal ramp: temperature, plus a lift inside wax.
fn heat(c: &Canvas, x: usize, y: usize) -> f32 {
    let s = c.at(x, y);
    if !c.inside(x, y) {
        return 0.0;
    }
    // The sampled temperature snaps from liquid to wax near the edge of a
    // blob's reach; ease it in over the whole rim instead, as a soft glow.
    let liquid = ambient_temp(1.0 - (y as f64 + 0.5) / c.height as f64) as f32;
    let temp = liquid + (s.temp - liquid) * smoothstep(s.density / SURFACE);
    let t = ((temp - 0.12) / 0.85).clamp(0.0, 1.0);
    let h = (t * 0.8 + 0.2 * coverage(s.density)).min(1.0);
    // Banded, so slow drifts don't repaint cells every frame, with the
    // band edges broken up by a static ordered dither so the bands don't
    // show as stripes.
    (h * HEAT_BANDS + bayer(x, y) - 0.5).round() / HEAT_BANDS
}

fn pixel(c: &Canvas, x: usize, y: usize) -> Color {
    if !c.inside(x, y) {
        return c.theme.color(c.backdrop(x, y));
    }
    let paint = c.theme.paint(Ink::Heat(heat(c, x, y)));
    paint.shade(c.light(x, y)).color()
}

/// Discrete thermal bands: cold liquid blank, warm liquid `░` in `dim`,
/// then wax temperatures as denser shades in the wax colours.
fn shade(h: f32) -> (char, Ink) {
    match h {
        h if h < 0.27 => (' ', Ink::Role(Role::Liquid)),
        h if h < WAX_BAND => ('░', Ink::Role(Role::Dim)),
        h if h < 0.52 => ('▒', Ink::Wax(0.0)),
        h if h < 0.72 => ('▓', Ink::Wax(0.5)),
        _ => ('█', Ink::Wax(1.0)),
    }
}
