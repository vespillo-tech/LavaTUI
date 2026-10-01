//! Placeholder animation proving the loop is smooth: a slow, warm moving
//! gradient drawn with half blocks (two "pixels" per cell). Delete once the
//! real sim + render styles are wired in.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

pub struct Gradient {
    pub time: f64,
}

impl Widget for Gradient {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let px = f64::from(x - area.x);
                let py = f64::from(y - area.y) * 2.0;
                buf[(x, y)]
                    .set_char('▀')
                    .set_fg(self.color_at(px, py))
                    .set_bg(self.color_at(px, py + 1.0));
            }
        }
    }
}

impl Gradient {
    fn color_at(&self, x: f64, y: f64) -> Color {
        let t = self.time;
        let wave = (x * 0.06 + t * 0.6).sin() * (y * 0.09 - t * 0.4).cos()
            + (x * 0.02 - y * 0.03 + t * 0.25).sin();
        lava((wave * 0.25 + 0.5).clamp(0.0, 1.0))
    }
}

/// Deep plum → red → orange → pale yellow.
fn lava(v: f64) -> Color {
    const STOPS: [(f64, f64, f64); 4] = [
        (24.0, 8.0, 20.0),
        (150.0, 20.0, 40.0),
        (240.0, 100.0, 30.0),
        (255.0, 220.0, 140.0),
    ];
    let scaled = v * (STOPS.len() - 1) as f64;
    let i = (scaled as usize).min(STOPS.len() - 2);
    let f = scaled - i as f64;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    let mix = |p: f64, q: f64| (p + (q - p) * f).round() as u8;
    Color::Rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}
