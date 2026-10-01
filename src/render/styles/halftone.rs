//! **halftone**: newsprint. The wax is printed as a screen of round dots
//! on a 45° grid (every other cell, staggered row to row), and dot size is
//! ink: pinpricks at the soft skin of a blob, swelling toward the hot core,
//! where the gaps between fill in too until the dots almost merge. The
//! liquid stays bare paper. The screen is fixed to the terminal, so dots
//! grow and shrink in place as the wax drifts beneath them.

use ratatui::buffer::Buffer;

use super::stepped_heat;
use crate::render::cell::mark;
use crate::render::{Canvas, Grid, LampStyle, smoothstep, wax_heat};
use crate::sim::SURFACE;
use crate::theme::Ink;

pub struct Halftone;

/// Dot sizes, smallest first.
const DOTS: [char; 3] = ['·', '•', '●'];
/// Ink thresholds for each dot size on the screen cells, and on the gap
/// cells between them (which only print once the ink is heavy).
const SCREEN: [f32; 3] = [0.12, 0.34, 0.58];
const GAPS: [f32; 3] = [0.6, 0.78, 0.92];

impl LampStyle for Halftone {
    const NAME: &'static str = "halftone";
    const GRID: Grid = Grid::HALF_BLOCK;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        c.for_each_cell(buf, |at, cell| {
            let (a, b) = (c.at(at.x, at.y), c.at(at.x, at.y + 1));
            let heat = wax_heat(0.5 * (a.temp + b.temp));
            let ink = ink(0.5 * (a.density + b.density), heat);
            let steps = if (at.cx + at.cy) % 2 == 0 {
                SCREEN
            } else {
                GAPS
            };
            let size = steps.iter().take_while(|&&t| ink >= t).count();
            let dot = (size > 0).then(|| {
                let fg = c.theme.color(Ink::Wax(stepped_heat(heat)));
                (DOTS[size - 1], fg)
            });
            mark(cell, dot, at.base);
        });
    }
}

/// Ink coverage 0..1: fades in across a wide band around the surface so
/// dots shrink toward the skin, and hotter wax prints heavier.
fn ink(density: f32, heat: f32) -> f32 {
    let body = smoothstep((density - (SURFACE - 0.15)) / 0.6);
    body * (0.6 + 0.4 * heat)
}
