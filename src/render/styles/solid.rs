//! **solid**: smooth wax in half blocks, two square pixels per cell. Wax is
//! coloured by temperature and blended into the liquid across a soft edge,
//! so outlines are anti-aliased. Without blending the edge is a crisp
//! threshold and temperature shows as the three wax colours (or, in
//! NO_COLOR, just the silhouette).

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use crate::render::{Canvas, Grid, LampStyle, coverage, wax_heat};
use crate::theme::Ink;

pub struct Solid;

impl LampStyle for Solid {
    const NAME: &'static str = "solid";
    const GRID: Grid = Grid::HALF_BLOCK;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        c.draw_half_blocks(buf, |x, y| pixel(c, x, y));
    }
}

fn pixel(c: &Canvas, x: usize, y: usize) -> Option<Color> {
    let s = c.at(x, y);
    let cover = coverage(s.density);
    let wax = Ink::Wax(wax_heat(s.temp));
    if c.theme.blends() {
        Some(c.theme.paint(c.backdrop(x, y)).mix(wax, cover).color())
    } else {
        (cover >= 0.5).then(|| c.theme.color(wax))
    }
}
