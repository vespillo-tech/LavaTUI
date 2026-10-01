//! **matrix**: digital rain, seen only through the wax. Every column runs
//! its own streams of falling glyphs at its own pace, but they only show
//! where they cross a blob, so the wax reads as a window onto the code.
//! Idle wax holds faint, slowly mutating glyphs; each stream has a bright
//! head and a trail fading behind it. Colour is the wax temperature. In
//! NO_COLOR idle wax becomes quiet dots and the streams stay legible.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{hash, quantise};
use crate::render::{Canvas, Grid, Style, wax_heat};
use crate::sim::SURFACE;
use crate::theme::{Ink, Role};

pub struct Matrix;

const GLYPHS: &[u8] = b"0123456789ABCDEFHKMNTXZ:=+*<>";
/// Idle glyphs in NO_COLOR, so streams stand out without colour.
const QUIET: [char; 2] = ['.', ':'];
/// Stream speed range, rows per second.
const SPEED: (f64, f64) = (5.0, 13.0);
/// Trail length range, in rows.
const TRAIL: (u32, u32) = (5, 14);
/// How often an idle glyph changes, per second.
const MUTATE_HZ: f64 = 0.6;
/// Streams per column.
const STREAMS: u32 = 3;

impl Style for Matrix {
    fn name(&self) -> &'static str {
        "matrix"
    }

    fn grid(&self) -> Grid {
        Grid::CELL
    }

    fn draw(&self, c: &Canvas, area: Rect, buf: &mut Buffer) {
        let rows = usize::from(area.height);
        for cx in 0..usize::from(area.width) {
            for cy in 0..rows {
                let s = c.at(cx, cy);
                let base = c.theme.color(c.backdrop(cx, cy));
                let cell = &mut buf[(area.x + cx as u16, area.y + cy as u16)];
                if s.density < SURFACE {
                    cell.set_char(' ').set_bg(base);
                    continue;
                }
                let heat = wax_heat(s.temp);
                let (glyph, fg) = match stream(cx, cy, rows, c.time) {
                    // The head: brightest, freshly changing glyph.
                    Some(0.0) => {
                        let fg = if c.theme.blends() {
                            c.theme
                                .paint(Ink::Wax(heat))
                                .mix(Ink::Role(Role::Text), 0.7)
                                .color()
                        } else {
                            c.theme.color(Ink::Role(Role::Text))
                        };
                        (glyph(cx, cy, c.time * 12.0), fg)
                    }
                    Some(fade) => {
                        let fg = c
                            .theme
                            .paint(Ink::Wax(heat))
                            .mix(Ink::Role(Role::Liquid), 0.6 * fade)
                            .scale(1.2 * c.light(cx, cy))
                            .color();
                        (glyph(cx, cy, c.time * MUTATE_HZ), fg)
                    }
                    None if !c.theme.has_color() => (
                        QUIET[(hash(cell_seed(cx, cy)) & 1) as usize],
                        c.theme.color(Ink::Wax(heat)),
                    ),
                    None => {
                        let fg = if c.theme.blends() {
                            c.theme
                                .paint(Ink::Wax(heat))
                                .mix(Ink::Role(Role::Liquid), 0.7)
                                .color()
                        } else {
                            c.theme.color(Ink::Role(Role::Dim))
                        };
                        (glyph(cx, cy, c.time * MUTATE_HZ), fg)
                    }
                };
                cell.set_char(glyph).set_fg(fg).set_bg(base);
            }
        }
    }
}

/// Where cell (`cx`, `cy`) sits in a falling stream: `Some(0)` at a head,
/// `Some(f)` in a trail (`f` 0 → 1 fading toward its end), `None` if no
/// stream is passing.
fn stream(cx: usize, cy: usize, rows: usize, time: f64) -> Option<f32> {
    (0..STREAMS)
        .filter_map(|k| {
            let h = hash(cx as u32 * STREAMS + k + 0x9e37);
            let unit = |shift: u32| f64::from((h >> shift) & 0xff) / 255.0;
            let speed = SPEED.0 + (SPEED.1 - SPEED.0) * unit(0);
            let trail = TRAIL.0 + (h >> 8) % (TRAIL.1 - TRAIL.0 + 1);
            // A gap between passes, so columns don't all look busy.
            let period = rows as f64 + f64::from(trail) + 6.0 + 30.0 * unit(16);
            let head = ((time * speed + unit(24) * period) % period).floor() as i64;
            let behind = head - cy as i64;
            (0..i64::from(trail))
                .contains(&behind)
                .then(|| quantise(behind as f32 / trail as f32, 8.0))
        })
        .reduce(f32::min)
}

/// A glyph for the cell that changes `rate`-wise over time, each cell on
/// its own phase so they don't flip in unison.
fn glyph(cx: usize, cy: usize, rate: f64) -> char {
    let seed = cell_seed(cx, cy);
    let phase = f64::from(hash(seed) & 0xffff) / 65536.0;
    let epoch = (rate + phase).floor() as u32;
    let i = hash(seed ^ epoch.wrapping_mul(0x2c1b_3c6d)) as usize % GLYPHS.len();
    char::from(GLYPHS[i])
}

fn cell_seed(cx: usize, cy: usize) -> u32 {
    (cx as u32).wrapping_mul(73_856_093) ^ (cy as u32).wrapping_mul(19_349_663)
}
