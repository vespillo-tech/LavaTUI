//! **crt**: an old phosphor monitor. Each cell row is one scanline: the top
//! pixel is the lit beam, the bottom a dark gap. The wax glows with a soft
//! phosphor bloom into the liquid, the picture falls off toward the
//! corners, and a faint hum bar rolls slowly down the screen. Without
//! blending the scanlines are drawn with glyphs instead: `▀` for hot wax,
//! a thin `▔` for cool.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::quantise;
use crate::render::{Canvas, Grid, Style, coverage, smoothstep, wax_heat};
use crate::sim::SURFACE;
use crate::theme::Ink;

pub struct Crt;

/// Brightness of the gap line under each scanline.
const GAP: f32 = 0.42;
/// How far the bloom reaches into the liquid, and how strong it is.
const BLOOM: f32 = 0.45;
/// Seconds for the hum bar to roll over the screen once.
const ROLL_SECS: f64 = 9.0;
/// Hum bar height in rows, and its extra brightness.
const BAR_ROWS: f64 = 4.0;
const BAR_GAIN: f32 = 0.07;

impl Style for Crt {
    fn name(&self) -> &'static str {
        "crt"
    }

    fn grid(&self) -> Grid {
        Grid::HALF_BLOCK
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        let rows = usize::from(area.height);
        for cy in 0..rows {
            let bar = hum_bar(c.time, cy, rows);
            for cx in 0..usize::from(area.width) {
                let (yt, yb) = (2 * cy, 2 * cy + 1);
                let base = c.theme.color(c.backdrop(cx, yt));
                let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                if !c.theme.blends() {
                    let s = c.at(cx, yt);
                    if s.density < SURFACE {
                        cell.set_char(' ').set_bg(base);
                    } else {
                        let heat = wax_heat(s.temp);
                        let ch = if heat < 0.5 { '▔' } else { '▀' };
                        let fg = c.theme.color(Ink::Wax(heat));
                        cell.set_char(ch).set_fg(fg).set_bg(base);
                    }
                    continue;
                }
                let beam = vignette(c, cx, yt) * bar;
                let top = pixel(c, cx, yt, beam);
                let bottom = pixel(c, cx, yb, beam * GAP);
                cell.set_char('▀').set_fg(top).set_bg(bottom);
            }
        }
    }
}

fn pixel(c: &Canvas, x: usize, y: usize, gain: f32) -> Color {
    let s = c.at(x, y);
    let cover = coverage(s.density);
    // Phosphor bloom: the soft field just outside the surface glows.
    let glow = BLOOM * smoothstep(s.density / SURFACE).powi(2);
    let wax = Ink::Wax(wax_heat(s.temp));
    let lit = if cover > 0.0 { 1.12 } else { 1.0 };
    c.theme
        .paint(c.backdrop(x, y))
        .mix(wax, quantise(cover.max(glow), 12.0))
        .scale(gain * lit)
        .shade(c.light(x, y))
        .color()
}

/// Darkening toward the corners of the tube; 1 in the middle.
fn vignette(c: &Canvas, x: usize, y: usize) -> f32 {
    let dx = 2.0 * (x as f32 + 0.5) / c.width as f32 - 1.0;
    let dy = 2.0 * (y as f32 + 0.5) / c.height as f32 - 1.0;
    quantise(1.0 - 0.22 * (dx * dx + dy * dy).powi(2), 32.0)
}

/// Extra brightness of row `cy` from the slowly rolling hum bar.
fn hum_bar(time: f64, cy: usize, rows: usize) -> f32 {
    let span = rows as f64 + 2.0 * BAR_ROWS;
    let centre = (time / ROLL_SECS).fract() * span - BAR_ROWS;
    let d = ((cy as f64 + 0.5 - centre) / BAR_ROWS).abs();
    if d >= 1.0 {
        return 1.0;
    }
    let bump = 1.0 - smoothstep(d as f32);
    1.0 + BAR_GAIN * quantise(bump, 3.0)
}
