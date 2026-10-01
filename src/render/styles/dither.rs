//! **dither**: ordered Bayer 8×8 dithering over four flat inks (liquid,
//! cool, mid, hot wax) on the half-block grid. Wax edges and temperature
//! steps dissolve into a fixed screen-anchored pattern, so it looks the
//! same (and great) in truecolor, 256 and 16 colours, and static areas
//! never flicker. In NO_COLOR it dithers ink on/off by heat instead.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::render::cell::half_block;
use crate::render::{Canvas, Grid, Style, bayer, lit, smoothstep, wax_heat};
use crate::sim::SURFACE;
use crate::theme::{Ink, Role};

pub struct Dither;

const INKS: [Role; 3] = [Role::WaxCool, Role::WaxMid, Role::WaxHot];
/// Edge softness: density range dithered between liquid and wax.
const EDGE: f32 = 0.3;

impl Style for Dither {
    fn name(&self) -> &'static str {
        "dither"
    }

    fn grid(&self) -> Grid {
        Grid::HALF_BLOCK
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        for cy in 0..usize::from(area.height) {
            for cx in 0..usize::from(area.width) {
                let base = c.theme.color(c.backdrop(cx, 2 * cy));
                let (top, bottom) = (pixel(c, cx, 2 * cy), pixel(c, cx, 2 * cy + 1));
                half_block(
                    &mut buf[(area.x + cx as u16, area.y + cy as u16)],
                    top,
                    bottom,
                    base,
                );
            }
        }
    }
}

fn pixel(c: &Canvas, x: usize, y: usize) -> Option<Color> {
    let s = c.at(x, y);
    let cover = smoothstep((s.density - (SURFACE - EDGE / 2.0)) / EDGE);
    // Light shifts the dithered ink level: shadows cooler, highlights hotter.
    let light = c.light(x, y);
    let heat = lit(wax_heat(s.temp), light);
    let threshold = bayer(x, y);

    if !c.theme.has_color() {
        // Ink density = heat: cool wax is a light stipple, hot wax solid.
        let v = cover * (0.3 + 0.7 * heat);
        return (v > threshold).then_some(Color::Reset);
    }
    // 0 = liquid, 1..=3 = cool/mid/hot wax; dither between neighbours.
    let v = cover * (1.0 + 2.0 * heat);
    let level = (v + threshold).floor() as usize;
    match level {
        0 if c.theme.blends() => Some(c.theme.paint(c.backdrop(x, y)).scale(light).color()),
        0 => None,
        n => Some(c.theme.color(Ink::Role(INKS[n.min(3) - 1]))),
    }
}
