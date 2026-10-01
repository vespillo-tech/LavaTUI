//! **halftone**: newsprint. The wax is printed as a screen of round dots
//! on a 45° grid (every other cell, staggered row to row), and dot size is
//! ink: pinpricks at the soft skin of a blob, swelling toward the hot core,
//! where the gaps between fill in too until the dots almost merge. The
//! liquid stays bare paper. The screen is fixed to the terminal, so dots
//! grow and shrink in place as the wax drifts beneath them.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::quantise;
use crate::render::{Canvas, Grid, Style, smoothstep, wax_heat};
use crate::sim::SURFACE;
use crate::theme::Ink;

pub struct Halftone;

/// Dot sizes, smallest first.
const DOTS: [char; 3] = ['·', '•', '●'];
/// Ink thresholds for each dot size on the screen cells, and on the gap
/// cells between them (which only print once the ink is heavy).
const SCREEN: [f32; 3] = [0.12, 0.34, 0.58];
const GAPS: [f32; 3] = [0.6, 0.78, 0.92];

impl Style for Halftone {
    fn name(&self) -> &'static str {
        "halftone"
    }

    fn grid(&self) -> Grid {
        Grid::HALF_BLOCK
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        for cy in 0..usize::from(area.height) {
            for cx in 0..usize::from(area.width) {
                let (yt, yb) = (2 * cy, 2 * cy + 1);
                let (a, b) = (c.at(cx, yt), c.at(cx, yb));
                let heat = wax_heat(0.5 * (a.temp + b.temp));
                let light = 0.5 * (c.light(cx, yt) + c.light(cx, yb));
                let ink = ink(0.5 * (a.density + b.density), heat, light);
                let steps = if (cx + cy) % 2 == 0 { SCREEN } else { GAPS };
                let size = steps.iter().take_while(|&&t| ink >= t).count();

                let base = c.theme.color(c.backdrop(cx, yt));
                let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                if size == 0 {
                    cell.set_char(' ').set_bg(base);
                } else {
                    let fg = c.theme.color(Ink::Wax(quantise(heat, 16.0)));
                    cell.set_char(DOTS[size - 1]).set_fg(fg).set_bg(base);
                }
            }
        }
    }
}

/// Ink coverage 0..1: fades in across a wide band around the surface so
/// dots shrink toward the skin, and hotter wax prints heavier.
fn ink(density: f32, heat: f32, light: f32) -> f32 {
    let body = smoothstep((density - (SURFACE - 0.15)) / 0.6);
    body * (0.6 + 0.4 * heat) * light
}
