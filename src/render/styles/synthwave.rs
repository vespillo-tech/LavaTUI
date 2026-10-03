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
/// The floor is a plane seen from above: a row `d` pixels below the
/// horizon is `NEAR_DEPTH × floor height / d` grid squares away, so the
/// bottom row is `NEAR_DEPTH` squares out at any size.
const NEAR_DEPTH: f32 = 2.5;
/// Fan line spacing, as a fraction of the distance below the horizon
/// (along the bottom row: of the floor's height).
const FAN: f32 = 0.7;
/// Line width, in grid squares; never drawn thinner than a pixel.
const LINE: f32 = 0.05;
/// Lines fade out between these spacings (pixels): packed tighter, the
/// far floor is haze rather than dots and moiré.
const FADE: (f32, f32) = (1.5, 4.5);
/// Seconds for the floor to slide one square toward the viewer.
const SCROLL_SECS: f64 = 4.0;

impl LampStyle for Synthwave {
    const NAME: &'static str = "synthwave";
    const GRID: Grid = Grid::HALF_BLOCK;
    const TIMED: bool = true;

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
        return match floor_line(c, x, y, scroll, true) > 0.0 {
            true => Pixel::Ink(c.theme.color(Ink::Role(Role::Accent))),
            false => Pixel::Liquid,
        };
    }
    // Halo: the liquid just outside the wax glows in neon.
    let glow = 0.55 * smoothstep(s.density / SURFACE).powi(3);
    let back = backdrop(c, x, y, v, scroll)
        .mix(neon, quantise(glow, 10.0))
        .color();
    // In 256 colours the backdrop takes the dominant index of its dither
    // pair: the sky's dark tints, dithered, read as dots strewn over it.
    Pixel::Back(c.theme.dither(back, 0.5))
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
    let d = y as f32 + 0.5 - horizon(c);
    if d < 0.0 {
        // Dusk: deepening liquid, warming to pink toward the horizon.
        let t = v / HORIZON;
        return paint
            .mix(Ink::Role(Role::Bg), quantise(0.6 * (1.0 - t), 16.0))
            .mix(Ink::Wax(0.0), quantise(0.6 * t.powi(5), 24.0));
    }
    // The floor: dark, the row along the horizon glowing pink, then the
    // grid in the accent colour, brighter toward the viewer.
    let near = d / (c.height as f32 - horizon(c));
    let floor = paint
        .mix(Ink::Role(Role::Bg), 0.6)
        .mix(Ink::Wax(0.0), quantise(0.6 * (-d).exp(), 16.0));
    let line = floor_line(c, x, y, scroll, false);
    floor.mix(
        Ink::Role(Role::Accent),
        quantise(line * (0.5 + 0.5 * near), 16.0),
    )
}

/// The horizon, in pixels from the top: on a pixel boundary, so it's one
/// crisp edge.
fn horizon(c: &Canvas) -> f32 {
    (HORIZON * c.height as f32).round()
}

/// Retro-sun gradient: gold at the top of the lamp, pink toward the floor,
/// nudged by temperature. Fine steps, so a big blob shows no flat bands.
fn sun(v: f32, heat: f32) -> f32 {
    quantise(0.65 * (1.0 - v) + 0.35 * heat, 64.0)
}

/// How strongly pixel (`x`, `y`) lies on a floor grid line, 0..1: the
/// part of the pixel a line covers, so lines slide smoothly across pixels
/// and slanted ones stay unbroken. Lines are a pixel wide, or [`LINE`] of
/// a square where that's wider, and fade out where perspective packs them
/// tighter than [`FADE`], so the far floor dissolves into haze instead of
/// dots. Nothing is drawn above the horizon. With `binary` (no blending)
/// a line is on (1) where it covers half the pixel and has barely begun
/// to fade: lines end cleanly, short of where they'd crowd into stripes.
fn floor_line(c: &Canvas, x: usize, y: usize, scroll: f32, binary: bool) -> f32 {
    let horizon = horizon(c);
    let top = y as f32 - horizon;
    if top < 0.0 {
        return 0.0;
    }
    let d = top + 0.5;
    let reach = NEAR_DEPTH * (c.height as f32 - horizon);
    let fade = |spacing: f32| smoothstep((spacing - FADE.0) / (FADE.1 - FADE.0));
    let shade = |cover: f32, fade: f32| match binary {
        true => f32::from(u8::from(cover >= 0.5 && fade >= 0.9)),
        false => cover.min(1.0) * fade,
    };

    // Depth lines: at `reach / d + scroll` whole, i.e. `d = reach / (n -
    // scroll)` for whole `n`: they bunch up toward the horizon and speed
    // up toward the viewer. Exact coverage of the pixel's rows
    // `top..top + 1` by each line near it.
    let spacing = d * d / reach;
    let mut depth = 0.0;
    if fade(spacing) > 0.0 {
        let half = 0.5 * (LINE * spacing).max(1.0);
        let n0 = (reach / (top + 1.0 + half) + scroll).ceil();
        let n1 = (reach / (top - half).max(1e-3) + scroll).floor();
        let mut n = n0;
        while n <= n1 && n < n0 + 3.0 {
            if n - scroll > 0.0 {
                let at = reach / (n - scroll);
                depth += ((at + half).min(top + 1.0) - (at - half).max(top)).max(0.0);
            }
            n += 1.0;
        }
        depth = shade(depth, fade(spacing));
    }

    // Fan lines: whole values of the floor's sideways coordinate `q`,
    // converging on the vanishing point at the centre of the horizon.
    // Coverage from the distance across the line, in pixels; the spacing
    // is the nearest line's, so each row of a line fades as one. The
    // vanishing point sits on a pixel's centre: a crisp middle line.
    let off = x as f32 - (c.width / 2) as f32;
    let q = off / (FAN * d);
    let spacing = d / q.round().hypot(1.0 / FAN);
    let half = 0.5 * (LINE * spacing).max(1.0);
    let across = (q - q.round()).abs() * spacing;
    let fan = shade((half + 0.5 - across).clamp(0.0, 1.0), fade(spacing));

    depth.max(fan)
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

    /// The grid never breaks into dots: over a second of frames at small,
    /// big and huge sizes, blended and not, every pixel a line lights
    /// (half strength or more) has a lit neighbour, and nothing is drawn
    /// above the horizon.
    #[test]
    fn grid_has_no_stray_dots() {
        for (cols, rows) in [(80u16, 24u16), (160, 45), (250, 70), (40, 12)] {
            let theme = Theme::new(
                Palette::by_name("synthwave").unwrap(),
                ColorDepth::TrueColor,
            );
            let (width, height) = (usize::from(cols), 2 * usize::from(rows));
            let canvas = Canvas {
                area: Rect::new(0, 0, cols, rows),
                samples: &[],
                width,
                height,
                theme: &theme,
                time: 0.0,
                translucent: false,
            };
            for binary in [false, true] {
                for frame in 0..30 {
                    let scroll = frame as f32 / 30.0 / SCROLL_SECS as f32;
                    let line: Vec<f32> = (0..height)
                        .flat_map(|y| (0..width).map(move |x| (x, y)))
                        .map(|(x, y)| floor_line(&canvas, x, y, scroll, binary))
                        .collect();
                    let at = |x: usize, y: usize| line[y * width + x];
                    for y in 0..height {
                        for x in 0..width {
                            let what = format!("{x},{y} {cols}x{rows} frame {frame} {binary}");
                            if (y as f32) < horizon(&canvas) {
                                assert_eq!(at(x, y), 0.0, "above the horizon: {what}");
                            }
                            if at(x, y) < 0.5 {
                                continue;
                            }
                            let lit = (y.saturating_sub(1)..(y + 2).min(height))
                                .flat_map(|ny| {
                                    (x.saturating_sub(1)..(x + 2).min(width))
                                        .map(move |nx| (nx, ny))
                                })
                                .any(|(nx, ny)| (nx, ny) != (x, y) && at(nx, ny) >= 0.25);
                            assert!(lit, "stray dot at {what}");
                        }
                    }
                }
            }
        }
    }

    /// Writes consecutive frames as PPM images (one image pixel per
    /// half-block pixel, as the cells show them) for judging the backdrop
    /// and its motion by eye: `SYNTHWAVE_FRAMES=<dir> cargo test --release
    /// -- --ignored dump_frames`. Optional `SYNTHWAVE_DEPTH` (truecolor,
    /// 256, 16, none), `SYNTHWAVE_TRANSLUCENT=1` and `SYNTHWAVE_START`
    /// (seconds, default 10).
    #[test]
    #[ignore]
    fn dump_frames() {
        use crate::render::{LampOptions, LampState, LampView, StyleId};
        use ratatui::widgets::StatefulWidget;
        use std::io::Write;

        let Ok(dir) = std::env::var("SYNTHWAVE_FRAMES") else {
            return;
        };
        let depth = match std::env::var("SYNTHWAVE_DEPTH").as_deref() {
            Ok("256") => ColorDepth::Ansi256,
            Ok("16") => ColorDepth::Ansi16,
            Ok("none") => ColorDepth::None,
            _ => ColorDepth::TrueColor,
        };
        let translucent = std::env::var("SYNTHWAVE_TRANSLUCENT").is_ok();
        let start: f64 = std::env::var("SYNTHWAVE_START").map_or(10.0, |s| s.parse().unwrap());
        let rgb = |c: Color, fallback: [u8; 3]| -> [u8; 3] {
            const SYSTEM: [u32; 16] = [
                0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5,
                0x7f7f7f, 0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
            ];
            let hex = |h: u32| [(h >> 16) as u8, (h >> 8) as u8, h as u8];
            let index = |i: u8| match i {
                0..=15 => hex(SYSTEM[usize::from(i)]),
                16..=231 => {
                    let i = i - 16;
                    let v = |k: u8| if k == 0 { 0 } else { 55 + 40 * k };
                    [v(i / 36), v(i / 6 % 6), v(i % 6)]
                }
                _ => [8 + 10 * (i - 232); 3],
            };
            match c {
                Color::Rgb(r, g, b) => [r, g, b],
                Color::Indexed(i) => index(i),
                Color::Reset => fallback,
                Color::Black => index(0),
                Color::Red => index(1),
                Color::Green => index(2),
                Color::Yellow => index(3),
                Color::Blue => index(4),
                Color::Magenta => index(5),
                Color::Cyan => index(6),
                Color::Gray => index(7),
                Color::DarkGray => index(8),
                Color::LightRed => index(9),
                Color::LightGreen => index(10),
                Color::LightYellow => index(11),
                Color::LightBlue => index(12),
                Color::LightMagenta => index(13),
                Color::LightCyan => index(14),
                Color::White => index(15),
            }
        };
        let style = StyleId::by_name("synthwave").unwrap().style();
        for (cols, rows) in [(80u16, 24u16), (160, 45), (250, 70)] {
            for name in ["synthwave", "lava"] {
                let theme = Theme::new(Palette::by_name(name).unwrap(), depth);
                let mut world = World::new(7, f64::from(cols) / (2.0 * f64::from(rows)));
                world.prewarm(1200, 1.0 / 120.0);
                let mut field = Field::default();
                let mut state = LampState::default();
                let area = Rect::new(0, 0, cols, rows);
                for frame in 0..30 {
                    for _ in 0..4 {
                        world.step(1.0 / 120.0);
                    }
                    field.prepare(&world, 1.0);
                    let mut buf = Buffer::empty(area);
                    LampView {
                        field: &field,
                        style,
                        theme: &theme,
                        time: start + f64::from(frame) / 30.0,
                        options: LampOptions {
                            reduced: false,
                            translucent,
                            ..LampOptions::default()
                        },
                    }
                    .render(area, &mut buf, &mut state);
                    let (w, h) = (usize::from(cols), 2 * usize::from(rows));
                    let mut img = format!("P6 {w} {h} 255\n").into_bytes();
                    let mut px = vec![[0u8; 3]; w * h];
                    for cy in 0..usize::from(rows) {
                        for cx in 0..w {
                            let cell = &buf[(cx as u16, cy as u16)];
                            let fg = rgb(cell.fg, [220, 220, 220]);
                            let bg = rgb(cell.bg, [0, 0, 0]);
                            let (top, bottom) = match cell.symbol() {
                                "▀" => (fg, bg),
                                "▄" => (bg, fg),
                                "█" => (fg, fg),
                                _ => (bg, bg),
                            };
                            px[2 * cy * w + cx] = top;
                            px[(2 * cy + 1) * w + cx] = bottom;
                        }
                    }
                    img.extend(px.iter().flatten());
                    let path = format!("{dir}/{name}_{cols}x{rows}_{frame:02}.ppm");
                    std::fs::File::create(path)
                        .unwrap()
                        .write_all(&img)
                        .unwrap();
                }
            }
        }
    }
}
