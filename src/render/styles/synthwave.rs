//! **synthwave**: a 1986 sunset. The liquid is a dusk sky glowing toward a
//! horizon, over a perspective grid floor in the accent colour. Wax blobs
//! are retro suns: one smooth gradient, gold at the top shading to pink
//! below, with a bright neon rim and a soft halo. The wax is opaque and
//! unbanded: no stripes (dark ones read as the sky showing through), and
//! the backdrop and halo only ever paint the liquid around it.
//! Without blending the grid and rims keep the look in flat colours, and
//! NO_COLOR keeps solid wax over the grid.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

use super::{is_edge, quantise};
use crate::render::{Canvas, Grid, LIQUID, LampStyle, Pixel, smoothstep, wax_heat};
use crate::sim::SURFACE;
use crate::theme::{Ink, Role};

pub struct Synthwave;

/// Horizon height, as a fraction of the canvas from the top.
const HORIZON: f32 = 0.6;
/// Floor grid: horizontal line density, and vertical lines across the
/// bottom row.
const DEPTH_LINES: f32 = 1.6;
const FAN_LINES: f32 = 7.0;
/// Seconds for the floor to scroll one line toward the viewer.
const SCROLL_SECS: f64 = 6.0;

impl LampStyle for Synthwave {
    const NAME: &'static str = "synthwave";
    const GRID: Grid = Grid::HALF_BLOCK;

    fn draw(c: &Canvas, buf: &mut Buffer) {
        let scroll = (c.time / SCROLL_SECS).fract() as f32;
        c.draw_half_blocks(buf, |x, y| pixel(c, x, y, scroll));
    }
}

fn pixel(c: &Canvas, x: usize, y: usize, scroll: f32) -> Pixel {
    let s = c.at(x, y);
    let v = (y as f32 + 0.5) / c.height as f32;
    let heat = wax_heat(s.temp);
    let wax = s.density >= SURFACE;
    let rim = wax && is_edge(c, x, y);
    // Neon: pink when cool, gold when hot.
    let neon = Ink::Wax(0.1 + 0.9 * heat);

    if wax {
        return Pixel::Ink(sun_pixel(c, v, heat, rim, neon));
    }
    if !c.theme.blends() {
        return match floor_line(c, x, y, scroll) >= 0.5 {
            true => Pixel::Ink(c.theme.color(Ink::Role(Role::Accent))),
            false => Pixel::Liquid,
        };
    }
    // Halo: the liquid just outside the wax glows in neon.
    let glow = 0.55 * smoothstep(s.density / SURFACE).powi(3);
    Pixel::Back(
        backdrop(c, x, y, v, scroll)
            .mix(neon, quantise(glow, 10.0))
            .color(),
    )
}

/// A wax pixel: neon on the rim, else the sun's gradient. Only wax inks,
/// so nothing behind the wax shows through.
fn sun_pixel(c: &Canvas, v: f32, heat: f32, rim: bool, neon: Ink) -> Color {
    let theme = c.theme;
    match rim {
        true if theme.blends() => theme.paint(neon).scale(1.35).color(),
        true => theme.color(neon),
        false => theme.color(Ink::Wax(sun(v, heat))),
    }
}

/// The sky and floor behind the wax.
fn backdrop<'t>(c: &'t Canvas, x: usize, y: usize, v: f32, scroll: f32) -> crate::theme::Paint<'t> {
    let paint = c.theme.paint(LIQUID);
    if v < HORIZON {
        // Dusk: deepening liquid, warming to pink just above the horizon.
        let t = v / HORIZON;
        return paint
            .mix(Ink::Role(Role::Bg), quantise(0.6 * (1.0 - t), 16.0))
            .mix(Ink::Wax(0.0), quantise(0.55 * t.powi(4), 24.0));
    }
    // The floor: dark, with a pink haze along the horizon.
    let near = (v - HORIZON) / (1.0 - HORIZON);
    let floor = paint
        .mix(Ink::Role(Role::Bg), 0.6)
        .mix(Ink::Wax(0.0), quantise(0.4 * (1.0 - near).powi(6), 24.0));
    let line = floor_line(c, x, y, scroll);
    floor.mix(
        Ink::Role(Role::Accent),
        quantise(line * (0.45 + 0.5 * near), 8.0),
    )
}

/// Retro-sun gradient: gold at the top of the lamp, pink toward the floor,
/// nudged by temperature. Fine steps, so a big blob shows no flat bands.
fn sun(v: f32, heat: f32) -> f32 {
    quantise(0.65 * (1.0 - v) + 0.35 * heat, 64.0)
}

/// How strongly pixel (`x`, `y`) lies on a floor grid line, 0..1. Lines
/// are one pixel wide, found where the line index changes from the pixel
/// above / left, and fade out where perspective packs them tighter than a
/// few pixels apart, so the far floor dissolves into haze instead of moiré.
fn floor_line(c: &Canvas, x: usize, y: usize, scroll: f32) -> f32 {
    let h = c.height as f32;
    let horizon = HORIZON * h;
    let fy = y as f32 + 0.5;
    let d = fy - horizon;
    if d <= 1.0 {
        return 0.0;
    }
    let visible = |spacing: f32| smoothstep((spacing - 1.5) / 2.0);
    // Depth lines: evenly spaced in 1/distance, so they bunch up toward
    // the horizon. Pixels between consecutive lines: d² / (k h).
    let depth = |fy: f32| (DEPTH_LINES * h / (fy - horizon) + scroll).floor();
    if depth(fy) != depth(fy - 1.0) {
        return visible(d * d / (DEPTH_LINES * h));
    }
    // Fan lines converge on the vanishing point at the centre of the horizon.
    let k = (h - horizon) / c.width as f32 * FAN_LINES / d;
    let fan = |fx: f32| ((fx - c.width as f32 / 2.0) * k).floor();
    if x > 0 && fan(x as f32 + 0.5) != fan(x as f32 - 0.5) {
        return visible(1.0 / k);
    }
    0.0
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;
    use crate::render::dither256;
    use crate::sim::{Field, Sample, World};
    use crate::theme::{ColorDepth, Palette, Theme};

    const DEPTHS: [ColorDepth; 4] = [
        ColorDepth::TrueColor,
        ColorDepth::Ansi256,
        ColorDepth::Ansi16,
        ColorDepth::None,
    ];

    /// Discs of wax over every backdrop band (sky, horizon, floor) and a
    /// pool along the bottom, with soft edges.
    fn field(width: usize, height: usize) -> Vec<Sample> {
        let blobs = [(0.3, 0.25, 0.2), (0.7, 0.55, 0.25), (0.45, 0.8, 0.18)];
        let mut out = Vec::with_capacity(width * height);
        for y in 0..height {
            let v = (y as f32 + 0.5) / height as f32;
            for x in 0..width {
                let u = (x as f32 + 0.5) / width as f32;
                let mut density = 2.0 * SURFACE * ((v - 0.92) / 0.08).max(0.0);
                for (bu, bv, r) in blobs {
                    let d = ((u - bu).powi(2) + (v - bv).powi(2)).sqrt() / r;
                    density += 2.0 * SURFACE * (1.0 - d).max(0.0);
                }
                out.push(Sample {
                    density,
                    temp: u * 0.8 + 0.1,
                });
            }
        }
        out
    }

    /// The cells drawn from `samples` in `theme`, dithered as `LampView`
    /// does in 256 colours.
    fn draw(theme: &Theme, area: Rect, samples: &[Sample], time: f64) -> Buffer {
        let dithering = theme.dithering();
        let theme = dithering.as_ref().unwrap_or(theme);
        let canvas = Canvas {
            area,
            samples,
            width: usize::from(area.width),
            height: 2 * usize::from(area.height),
            theme,
            time,
            translucent: false,
        };
        let mut buf = Buffer::empty(area);
        Synthwave::draw(&canvas, &mut buf);
        if dithering.is_some() {
            dither256::resolve(theme, area, &mut buf, false);
        }
        buf
    }

    /// Largest step, per RGB channel, between vertically adjacent body
    /// pixels: the gradient takes ≤ 8 a pixel, the sim's temperature ≤ 16
    /// more; a stripe or a backdrop pixel is a jump of 60+.
    const SMOOTH: i32 = 20;

    /// Wax bodies (inside the rim) in truecolour are one smooth gradient:
    /// down each column, neighbouring pixels differ by at most [`SMOOTH`]
    /// per channel (a stripe, a band or a backdrop pixel is a jump), and
    /// with `monotone` (temperature constant down a column) a colour, once
    /// left, never comes back. Returns how many pixel pairs it compared.
    fn assert_smooth(c: &Canvas, monotone: bool, what: &str) -> usize {
        let rgb = |x, y| match pixel(c, x, y, 0.0) {
            Pixel::Ink(Color::Rgb(r, g, b)) => [r, g, b].map(i32::from),
            other => panic!("{x},{y}: {other:?} ({what})"),
        };
        let body = |x, y| c.at(x, y).density >= SURFACE && !is_edge(c, x, y);
        let mut pairs = 0;
        for x in 0..c.width {
            let mut seen: Vec<[i32; 3]> = Vec::new();
            for y in 0..c.height {
                if !body(x, y) {
                    seen.clear();
                    continue;
                }
                let here = rgb(x, y);
                if let Some(&above) = seen.last() {
                    let step = (0..3).map(|i| (here[i] - above[i]).abs()).max().unwrap();
                    assert!(
                        step <= SMOOTH,
                        "band at {x},{y}: {above:?} -> {here:?} ({what})"
                    );
                    pairs += 1;
                    if monotone && here != above {
                        assert!(
                            !seen.contains(&here),
                            "stripe at {x},{y}: {here:?} ({what})"
                        );
                    }
                }
                seen.push(here);
            }
        }
        pairs
    }

    /// Wax is opaque: repainting everything the backdrop is made of (the
    /// liquid, bg and the accent grid) and scrolling the grid changes no
    /// wax pixel, at every depth and palette, and no wax pixel is a hole.
    /// In truecolour every body is a smooth unbanded gradient, here and in
    /// big real frames.
    #[test]
    fn backdrop_never_shows_inside_the_wax() {
        let area = Rect::new(0, 0, 48, 20);
        let (width, height) = (48, 40);
        let samples = field(width, height);
        let wax = |x: usize, y: usize| samples[y * width + x].density >= SURFACE;
        // Big frames, where bands are widest: the synthetic discs and the
        // real sim at 250×70 cells.
        let (big_w, big_h) = (250, 140);
        let discs = field(big_w, big_h);
        let mut world = World::new(2, big_w as f64 / big_h as f64);
        world.prewarm(1200, 1.0 / 120.0);
        let mut sim = Field::default();
        sim.prepare(&world, 1.0);
        let mut real = vec![Sample::default(); big_w * big_h];
        sim.fill(&mut real, big_w, big_h);
        for palette in Palette::all() {
            let theme = Theme::new(palette, ColorDepth::TrueColor);
            if !theme.blends() {
                continue; // `ansi`: the terminal's own 16 colours, no gradient
            }
            for (samples, monotone, what) in [(&discs, true, "discs"), (&real, false, "sim")] {
                let canvas = Canvas {
                    area: Rect::new(0, 0, big_w as u16, (big_h / 2) as u16),
                    samples,
                    width: big_w,
                    height: big_h,
                    theme: &theme,
                    time: 0.0,
                    translucent: false,
                };
                let what = format!("{what} {}", palette.name);
                assert!(assert_smooth(&canvas, monotone, &what) > 1000, "{what}");
            }
        }
        for palette in Palette::all() {
            for depth in DEPTHS {
                let plain = Theme::new(palette, depth);
                let garish = plain
                    .with_role(Role::Liquid, plain.paint(Ink::Role(Role::Accent)))
                    .with_role(Role::Bg, plain.paint(Ink::Role(Role::Text)))
                    .with_role(Role::Accent, plain.paint(Ink::Role(Role::Bg)));
                let canvas = |theme| Canvas {
                    area,
                    samples: &samples,
                    width,
                    height,
                    theme,
                    time: 0.0,
                    translucent: false,
                };
                let (a, b) = (canvas(&plain), canvas(&garish));
                for y in 0..height {
                    for x in (0..width).filter(|&x| wax(x, y)) {
                        let p = pixel(&a, x, y, 0.0);
                        assert!(
                            matches!(p, Pixel::Ink(_)),
                            "hole at {x},{y} ({} {depth:?})",
                            palette.name
                        );
                        assert_eq!(
                            p,
                            pixel(&b, x, y, 0.4),
                            "{x},{y} ({} {depth:?})",
                            palette.name
                        );
                    }
                }
                let (a, b) = (
                    draw(&plain, area, &samples, 0.0),
                    draw(&garish, area, &samples, 2.5),
                );
                for (cy, (ca, cb)) in a
                    .content()
                    .chunks(width)
                    .zip(b.content().chunks(width))
                    .enumerate()
                {
                    for cx in (0..width).filter(|&x| wax(x, 2 * cy) && wax(x, 2 * cy + 1)) {
                        let (ca, cb) = (&ca[cx], &cb[cx]);
                        assert_eq!(
                            ca.symbol(),
                            cb.symbol(),
                            "{cx},{cy} ({} {depth:?})",
                            palette.name
                        );
                        assert_eq!(ca.fg, cb.fg, "{cx},{cy} ({} {depth:?})", palette.name);
                        if ca.symbol() != "█" {
                            assert_eq!(ca.bg, cb.bg, "{cx},{cy} ({} {depth:?})", palette.name);
                        }
                    }
                }
            }
        }
    }
}
