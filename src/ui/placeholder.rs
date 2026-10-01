//! Temporary wax view so the sim can be watched live: the field sampled at
//! two square-ish pixels per cell, drawn with half blocks. Replaced by the
//! real render styles (lava-bdj).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

use crate::sim::{Field, SURFACE, Sample};

pub struct WaxView<'a> {
    pub field: &'a Field,
    /// Reused sample buffer (no per-frame allocation once warmed up).
    pub samples: &'a mut Vec<Sample>,
}

impl Widget for WaxView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (cols, rows) = (usize::from(area.width), usize::from(area.height) * 2);
        self.samples.resize(cols * rows, Sample::default());
        self.field.fill(self.samples, cols, rows);
        for (cy, y) in (area.top()..area.bottom()).enumerate() {
            for (cx, x) in (area.left()..area.right()).enumerate() {
                let top = self.samples[2 * cy * cols + cx];
                let bottom = self.samples[(2 * cy + 1) * cols + cx];
                buf[(x, y)]
                    .set_char('▀')
                    .set_fg(color(top))
                    .set_bg(color(bottom));
            }
        }
    }
}

/// Liquid (dark plum, faintly warmer near the base) blended into wax
/// (coloured by temperature) across a soft edge around the surface.
fn color(s: Sample) -> Color {
    let edge = ((s.density - (SURFACE - 0.12)) / 0.24).clamp(0.0, 1.0);
    let cover = edge * edge * (3.0 - 2.0 * edge);
    let liquid = [18.0 + 30.0 * s.temp, 6.0 + 6.0 * s.temp, 22.0];
    let wax = lava(0.15 + 0.85 * s.temp);
    let mix = |i: usize| (liquid[i] + (wax[i] - liquid[i]) * cover).round() as u8;
    Color::Rgb(mix(0), mix(1), mix(2))
}

/// Deep red → orange → pale yellow.
fn lava(v: f32) -> [f32; 3] {
    const STOPS: [[f32; 3]; 4] = [
        [120.0, 16.0, 40.0],
        [200.0, 40.0, 30.0],
        [245.0, 120.0, 35.0],
        [255.0, 220.0, 140.0],
    ];
    let scaled = v.clamp(0.0, 1.0) * (STOPS.len() - 1) as f32;
    let i = (scaled as usize).min(STOPS.len() - 2);
    let f = scaled - i as f32;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * f)
}
