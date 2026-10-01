//! **dither**: ordered Bayer 8×8 dithering over four flat inks (liquid,
//! cool, mid, hot wax) on the half-block grid. Wax edges and temperature
//! steps dissolve into a fixed screen-anchored pattern, so it looks the
//! same (and great) in truecolor, 256 and 16 colours, and static areas
//! never flicker. In NO_COLOR it dithers ink on/off by heat instead.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use crate::render::{Canvas, Grid, LampStyle, bayer, lit, soft_edge, wax_heat};
use crate::theme::{Ink, Role};

pub struct Dither;

const INKS: [Role; 3] = [Role::WaxCool, Role::WaxMid, Role::WaxHot];
/// Edge softness: density range dithered between liquid and wax.
const EDGE: f32 = 0.3;

impl LampStyle for Dither {
    const NAME: &'static str = "dither";
    const GRID: Grid = Grid::HALF_BLOCK;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        c.draw_half_blocks(buf, |x, y| pixel(c, x, y));
    }
}

fn pixel(c: &Canvas, x: usize, y: usize) -> Option<Color> {
    let s = c.at(x, y);
    let cover = soft_edge(s.density, EDGE);
    // Light shifts the dithered ink level: shadows cooler, highlights hotter.
    let light = c.light(x, y);
    let heat = lit(wax_heat(s.temp), light);
    let threshold = bayer(x, y);

    if !c.theme.has_color() {
        // Ink density = heat: cool wax is a light stipple, hot wax solid.
        let v = cover * (0.3 + 0.7 * heat);
        return (v > threshold).then(|| c.theme.color(Ink::Wax(heat)));
    }
    // 0 = liquid, 1..=3 = cool/mid/hot wax; dither between neighbours.
    let v = cover * (1.0 + 2.0 * heat);
    let level = (v + threshold).floor() as usize;
    match level {
        0 if c.theme.blends() => Some(c.theme.paint(c.backdrop(x, y)).shade(light).color()),
        0 => None,
        n => Some(c.theme.color(Ink::Role(INKS[n.min(3) - 1]))),
    }
}
