//! **solid**: smooth wax in half blocks, two square pixels per cell. Wax is
//! coloured by temperature and blended into the liquid across a soft edge,
//! so outlines are anti-aliased. Without blending the edge is a crisp
//! threshold and temperature shows as the three wax colours (or, in
//! NO_COLOR, just the silhouette).

use ratatui::buffer::Buffer;

use crate::render::{Canvas, Grid, LIQUID, LampStyle, Pixel, coverage, wax_heat};
use crate::theme::Ink;

pub struct Solid;

impl LampStyle for Solid {
    const NAME: &'static str = "solid";
    const GRID: Grid = Grid::HALF_BLOCK;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        c.draw_half_blocks(buf, |x, y| pixel(c, x, y));
    }
}

fn pixel(c: &Canvas, x: usize, y: usize) -> Pixel {
    let s = c.at(x, y);
    let cover = coverage(s.density);
    let wax = Ink::Wax(wax_heat(s.temp));
    if c.theme.blends() {
        let color = c.theme.paint(LIQUID).mix(wax, cover).color();
        match cover {
            0.0 => Pixel::Back(color),
            _ => Pixel::Ink(color),
        }
    } else if cover >= 0.5 {
        Pixel::Ink(c.theme.color(wax))
    } else {
        Pixel::Liquid
    }
}
