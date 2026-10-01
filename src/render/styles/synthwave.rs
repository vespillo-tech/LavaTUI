//! **synthwave**: a 1986 sunset. The liquid is a dusk sky glowing toward a
//! horizon, over a perspective grid floor in the accent colour. Wax blobs
//! are retro suns: gold at the top shading to pink below, cut by the
//! classic widening stripes, with a bright neon rim and a soft halo.
//! Without blending the grid and rims keep the look in flat colours, and
//! NO_COLOR keeps the stripes and the grid.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::{is_edge, quantise};
use crate::render::cell::half_block;
use crate::render::{Canvas, Grid, Style, coverage, smoothstep, wax_heat};
use crate::sim::SURFACE;
use crate::theme::{Ink, Role};

pub struct Synthwave;

/// Horizon height, as a fraction of the canvas from the top.
const HORIZON: f32 = 0.6;
/// Floor grid: horizontal line density, and vertical lines across the
/// bottom row.
const DEPTH_LINES: f32 = 1.6;
const FAN_LINES: f32 = 7.0;
/// Seconds for the floor to scroll one line toward the viewer.
const SCROLL_SECS: f64 = 6.0;

impl Style for Synthwave {
    fn name(&self) -> &'static str {
        "synthwave"
    }

    fn grid(&self) -> Grid {
        Grid::HALF_BLOCK
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        let scroll = (c.time / SCROLL_SECS).fract() as f32;
        for cy in 0..usize::from(area.height) {
            for cx in 0..usize::from(area.width) {
                let base = c.theme.color(c.backdrop(cx, 2 * cy));
                let top = pixel(c, cx, 2 * cy, scroll);
                let bottom = pixel(c, cx, 2 * cy + 1, scroll);
                let pos = (area.x + cx as u16, area.y + cy as u16);
                half_block(&mut buf[pos], top, bottom, base);
            }
        }
    }
}

fn pixel(c: &Canvas, x: usize, y: usize, scroll: f32) -> Option<Color> {
    let s = c.at(x, y);
    let v = (y as f32 + 0.5) / c.height as f32;
    let heat = wax_heat(s.temp);
    let wax = s.density >= SURFACE;
    let rim = wax && is_edge(c, x, y);
    // Neon: pink when cool, gold when hot.
    let neon = Ink::Wax(0.1 + 0.9 * heat);

    if !c.theme.blends() {
        if rim {
            return Some(c.theme.color(neon));
        }
        if wax {
            let lit = !stripe(y, v) && (c.theme.has_color() || y.is_multiple_of(2));
            return lit.then(|| c.theme.color(Ink::Wax(sun(v, heat))));
        }
        return (c.inside(x, y) && floor_line(c, x, y, scroll) >= 0.5)
            .then(|| c.theme.color(Ink::Role(Role::Accent)));
    }

    let backdrop = backdrop(c, x, y, v, scroll);
    if rim {
        return Some(
            backdrop
                .mix(neon, 1.0)
                .scale(1.35)
                .shade(c.light(x, y))
                .color(),
        );
    }
    if wax && !stripe(y, v) {
        let body = Ink::Wax(sun(v, heat));
        let cover = coverage(s.density);
        return Some(backdrop.mix(body, cover).shade(c.light(x, y)).color());
    }
    // Halo: the soft field around the wax glows in neon (dimly through the
    // stripes).
    let glow = if wax {
        0.2
    } else {
        0.55 * smoothstep(s.density / SURFACE).powi(3)
    };
    Some(backdrop.mix(neon, quantise(glow, 10.0)).color())
}

/// The sky and floor behind the wax.
fn backdrop<'t>(c: &'t Canvas, x: usize, y: usize, v: f32, scroll: f32) -> crate::theme::Paint<'t> {
    let paint = c.theme.paint(c.backdrop(x, y));
    if !c.inside(x, y) {
        return paint;
    }
    if v < HORIZON {
        // Dusk: deepening liquid, warming to pink just above the horizon.
        let t = v / HORIZON;
        return paint
            .mix(Ink::Role(Role::Bg), quantise(0.6 * (1.0 - t), 16.0))
            .mix(Ink::Wax(0.0), quantise(0.55 * t.powi(4), 24.0));
    }
    // The floor: dark, with a pink haze along the horizon.
    let near = (v - HORIZON) / (1.0 - HORIZON);
    let floor = paint
        .mix(Ink::Role(Role::Bg), 0.6)
        .mix(Ink::Wax(0.0), quantise(0.4 * (1.0 - near).powi(6), 24.0));
    let line = floor_line(c, x, y, scroll);
    floor.mix(
        Ink::Role(Role::Accent),
        quantise(line * (0.45 + 0.5 * near), 8.0),
    )
}

/// Retro-sun gradient: gold at the top of the lamp, pink toward the floor,
/// nudged by temperature.
fn sun(v: f32, heat: f32) -> f32 {
    quantise(0.65 * (1.0 - v) + 0.35 * heat, 24.0)
}

/// The sun's cut-out stripes, widening toward the bottom of the lamp but
/// stopping short of the pool.
fn stripe(y: usize, v: f32) -> bool {
    const PERIOD: usize = 6;
    let gap = ((v - 0.35) / 0.55 * 4.0).floor();
    v < 0.88 && gap >= 1.0 && (y % PERIOD) < gap as usize
}

/// How strongly pixel (`x`, `y`) lies on a floor grid line, 0..1. Lines
/// are one pixel wide, found where the line index changes from the pixel
/// above / left, and fade out where perspective packs them tighter than a
/// few pixels apart, so the far floor dissolves into haze instead of moiré.
fn floor_line(c: &Canvas, x: usize, y: usize, scroll: f32) -> f32 {
    let h = c.height as f32;
    let horizon = HORIZON * h;
    let fy = y as f32 + 0.5;
    let d = fy - horizon;
    if d <= 1.0 {
        return 0.0;
    }
    let visible = |spacing: f32| smoothstep((spacing - 1.5) / 2.0);
    // Depth lines: evenly spaced in 1/distance, so they bunch up toward
    // the horizon. Pixels between consecutive lines: d² / (k h).
    let depth = |fy: f32| (DEPTH_LINES * h / (fy - horizon) + scroll).floor();
    if depth(fy) != depth(fy - 1.0) {
        return visible(d * d / (DEPTH_LINES * h));
    }
    // Fan lines converge on the vanishing point at the centre of the horizon.
    let k = (h - horizon) / c.width as f32 * FAN_LINES / d;
    let fan = |fx: f32| ((fx - c.width as f32 / 2.0) * k).floor();
    if x > 0 && fan(x as f32 + 0.5) != fan(x as f32 - 0.5) {
        return visible(1.0 / k);
    }
    0.0
}
