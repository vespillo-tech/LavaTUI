//! **crt**: an old phosphor monitor. Each cell row is one scanline: the top
//! pixel is the lit beam, the bottom a dark gap. The wax glows with a soft
//! phosphor bloom into the liquid, the picture falls off toward the
//! corners, and a faint hum bar rolls slowly down the screen. The tube
//! is the glass: outside it the app background stays plain. Without
//! blending the wax is drawn with glyphs instead: `▀` scanlines for cool
//! wax, solid `█` for hot.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use super::quantise;
use crate::render::cell::{blank, glyph, mark};
use crate::render::{Canvas, Grid, LampStyle, coverage, smoothstep, wax_heat};
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

impl LampStyle for Crt {
    const NAME: &'static str = "crt";
    const GRID: Grid = Grid::HALF_BLOCK;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        let rows = usize::from(c.area.height);
        c.for_each_cell(buf, |at, cell| {
            let (x, y) = (at.x, at.y);
            if !c.inside(x, y) {
                blank(cell, at.base);
            } else if !c.theme.blends() {
                mark(cell, block(c, x, y), at.base);
            } else {
                let beam = vignette(c, x, y) * hum_bar(c.time, at.cy, rows);
                let top = pixel(c, x, y, beam);
                let bottom = pixel(c, x, y + 1, beam * GAP);
                glyph(cell, '▀', top, bottom);
            }
        });
    }
}

/// Without blending: the wax as flat half blocks in the wax steps.
fn block(c: &Canvas, x: usize, y: usize) -> Option<(char, Color)> {
    let (t, b) = (c.at(x, y), c.at(x, y + 1));
    let wax = [t, b].map(|s| s.density >= SURFACE);
    if wax == [false, false] {
        return None;
    }
    let s = if wax[0] { t } else { b };
    let heat = wax_heat(s.temp);
    let ch = match wax {
        [true, true] if heat >= 0.5 => '█',
        [false, true] => '▄',
        _ => '▀',
    };
    Some((ch, c.theme.color(Ink::Wax(heat))))
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
