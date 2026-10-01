//! **solid**: smooth wax in half blocks, two square pixels per cell. Wax is
//! coloured by temperature and blended into the liquid across a soft edge,
//! so outlines are anti-aliased. Without blending the edge is a crisp
//! threshold and temperature shows as the three wax colours (or, in
//! NO_COLOR, just the silhouette).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::render::cell::half_block;
use crate::render::{Canvas, Grid, Style, coverage, wax_heat};
use crate::theme::Ink;

pub struct Solid;

impl Style for Solid {
    fn name(&self) -> &'static str {
        "solid"
    }

    fn grid(&self) -> Grid {
        Grid::HALF_BLOCK
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        for cy in 0..usize::from(area.height) {
            for cx in 0..usize::from(area.width) {
                let (top, bottom) = (pixel(c, cx, 2 * cy), pixel(c, cx, 2 * cy + 1));
                let base = c.theme.color(c.backdrop(cx, 2 * cy));
                let pos = (area.x + cx as u16, area.y + cy as u16);
                half_block(&mut buf[pos], top, bottom, base);
            }
        }
    }
}

fn pixel(c: &Canvas, x: usize, y: usize) -> Option<Color> {
    let s = c.at(x, y);
    let cover = coverage(s.density);
    let wax = Ink::Wax(wax_heat(s.temp));
    if c.theme.blends() {
        let paint = c.theme.paint(c.backdrop(x, y)).mix(wax, cover);
        Some(paint.shade(c.light(x, y)).color())
    } else {
        (cover >= 0.5).then(|| c.theme.color(wax))
    }
}
