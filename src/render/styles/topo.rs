//! **topo**: a topographic map of the wax field. Fine braille contour
//! lines trace the density at fixed levels: faint "sea floor" lines in the
//! liquid where blobs are about to touch, a bright index line at the wax
//! surface, and tightening rings climbing to each core. Between the lines
//! the bands are tinted by elevation (hypsometric colour), warming with the
//! wax temperature. Without blending it's the lines alone, which is all a
//! map needs.

use ratatui::buffer::Buffer;

use super::stepped_heat;
use crate::render::cell::{braille, braille_dots, mark};
use crate::render::{Canvas, Grid, LampStyle, wax_heat};
use crate::theme::{Ink, Role};

pub struct Topo;

/// Contour levels, in density. `SURFACE_BAND` is the wax surface.
const LEVELS: [f32; 6] = [0.25, 0.5, 0.68, 0.86, 1.06, 1.4];
const SURFACE_BAND: u8 = 2;

impl LampStyle for Topo {
    const NAME: &'static str = "topo";
    const GRID: Grid = Grid::BRAILLE;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        c.for_each_cell(buf, |at, cell| {
            let (mut top_line, mut heat, mut density) = (0u8, 0.0, 0.0);
            let bits = braille_dots(at.cx, at.cy, |x, y| {
                let b = band(c, x, y);
                let s = c.at(x, y);
                heat += wax_heat(s.temp);
                density += s.density;
                // A line pixel sits on the high side of a level crossing.
                let low = band(c, x + 1, y).min(band(c, x, y + 1));
                let low = low.min(band(c, x.saturating_sub(1), y));
                let low = low.min(band(c, x, y.saturating_sub(1)));
                if low < b {
                    top_line = top_line.max(b);
                }
                low < b
            });
            let heat = stepped_heat(heat / 8.0);
            let bg = if c.theme.blends() {
                tint(c, at.backdrop, level(density / 8.0), heat)
            } else {
                at.base
            };
            let lines = (bits != 0).then(|| (braille(bits), line(c, at.backdrop, top_line, heat)));
            mark(cell, lines, bg);
        });
    }
}

/// Which contour band a pixel is in: the number of levels at or below its
/// density. Off-canvas repeats the edge (callers step left / up with
/// `saturating_sub`, right / down past the end), so the frame isn't
/// contoured.
fn band(c: &Canvas, x: usize, y: usize) -> u8 {
    level(c.at(x.min(c.width - 1), y.min(c.height - 1)).density)
}

fn level(density: f32) -> u8 {
    LEVELS.iter().take_while(|&&l| density >= l).count() as u8
}

/// Band fill: liquid below the surface, then wax colour deepening with
/// elevation.
fn tint(c: &Canvas, backdrop: Ink, band: u8, heat: f32) -> ratatui::style::Color {
    let paint = c.theme.paint(backdrop);
    if band < SURFACE_BAND {
        return paint.color();
    }
    let up = f32::from(band - SURFACE_BAND) / (LEVELS.len() as f32 - f32::from(SURFACE_BAND));
    paint.mix(Ink::Wax(heat), 0.3 + 0.45 * up).color()
}

/// Line colour: dim below the surface, the wax colour at and above it,
/// brightest on the index line at the surface.
fn line(c: &Canvas, backdrop: Ink, band: u8, heat: f32) -> ratatui::style::Color {
    match band {
        b if b < SURFACE_BAND => {
            if c.theme.blends() {
                c.theme.paint(backdrop).mix(Ink::Wax(0.0), 0.5).color()
            } else {
                c.theme.color(Ink::Role(Role::Dim))
            }
        }
        SURFACE_BAND => c
            .theme
            .paint(Ink::Wax(0.25 + 0.75 * heat))
            .scale(1.15)
            .color(),
        _ => c.theme.paint(Ink::Wax(heat)).scale(1.25).color(),
    }
}
