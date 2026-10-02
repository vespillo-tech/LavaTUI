use std::fmt::Write as _;
use std::path::PathBuf;

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::StatefulWidget;

use super::*;
use crate::sim::{World, ambient_temp};
use crate::theme::{ColorDepth, Palette};

const DEPTHS: [(ColorDepth, &str); 4] = [
    (ColorDepth::TrueColor, "truecolor"),
    (ColorDepth::Ansi256, "256"),
    (ColorDepth::Ansi16, "16"),
    (ColorDepth::None, "none"),
];

fn theme(depth: ColorDepth) -> Theme {
    Theme::new(&Palette::all()[0], depth)
}

/// A fixed, sim-independent field: three blobs (cool, warm, hot) and a hot
/// pool, built with the same kernel shape as `sim::Field`, so snapshots
/// don't change when the simulation is retuned.
fn synthetic(width: usize, height: usize, aspect: f32) -> Vec<Sample> {
    const SUPPORT: f32 = 1.8;
    let peak = SURFACE / (1.0 - 1.0 / (SUPPORT * SUPPORT)).powi(2);
    // (x, y, radius, temp) in world units: x centred, y up, height 1.
    let blobs = [
        (-0.25 * aspect, 0.75, 0.13, 0.3),
        (0.2 * aspect, 0.5, 0.17, 0.6),
        (0.05 * aspect, 0.22, 0.1, 0.85),
    ];
    let mut out = Vec::with_capacity(width * height);
    for j in 0..height {
        let y = 1.0 - (j as f32 + 0.5) / height as f32;
        for i in 0..width {
            let x = ((i as f32 + 0.5) / width as f32 - 0.5) * aspect;
            let (mut density, mut heat) = (0.0, 0.0);
            for (bx, by, r, t) in blobs {
                let q2 = ((x - bx).powi(2) + (y - by).powi(2)) / (SUPPORT * r).powi(2);
                if q2 < 1.0 {
                    let w = peak * (1.0 - q2).powi(2);
                    density += w;
                    heat += w * t;
                }
            }
            let pool = smoothstep((0.08 - y) / 0.07 + 0.5);
            density += pool;
            heat += pool * 0.92;
            let liquid = ambient_temp(f64::from(y)) as f32;
            out.push(Sample {
                density,
                temp: (heat + 0.02 * liquid) / (density + 0.02),
            });
        }
    }
    out
}

/// Draw `style` from the synthetic field into a fresh buffer of `area`.
fn draw_synthetic(style: &StyleEntry, theme: &Theme, area: Rect) -> Buffer {
    draw_synthetic_at(style, theme, area, 0.0)
}

fn draw_synthetic_at(style: &StyleEntry, theme: &Theme, area: Rect, time: f64) -> Buffer {
    let grid = style.grid();
    let width = usize::from(area.width * grid.x);
    let height = usize::from(area.height * grid.y);
    let aspect = f32::from(area.width) / (2.0 * f32::from(area.height));
    let samples = synthetic(width, height, aspect);
    let canvas = Canvas {
        area,
        samples: &samples,
        width,
        height,
        theme,
        time,
        translucent: false,
    };
    let mut buf = Buffer::empty(area);
    style.draw(&canvas, &mut buf);
    buf
}

fn glyphs(buf: &Buffer) -> String {
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

/// One letter per 16-colour foreground, so colour legibility is part of
/// the snapshot.
fn ansi_fg(buf: &Buffer) -> String {
    let mut s = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            s.push(match buf[(x, y)].fg {
                Color::Reset => '.',
                Color::Red => 'r',
                Color::LightRed => 'R',
                Color::Yellow => 'y',
                Color::LightYellow => 'Y',
                Color::DarkGray => 'd',
                other => panic!("unexpected 16-colour fg {other:?}"),
            });
        }
        s.push('\n');
    }
    s
}

/// Compare with `src/render/snapshots/<name>.txt`. Run with
/// `UPDATE_SNAPSHOTS=1 cargo test` to (re)write them, then review the diff.
fn assert_snapshot(name: &str, actual: &str) {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "src/render/snapshots"]
        .iter()
        .collect::<PathBuf>()
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing snapshot {name}; run with UPDATE_SNAPSHOTS=1"));
    assert!(
        expected == actual,
        "snapshot {name} changed (UPDATE_SNAPSHOTS=1 to accept):\n--- expected\n{expected}--- actual\n{actual}"
    );
}

#[test]
fn snapshots() {
    let area = Rect::new(0, 0, 36, 14);
    for id in StyleId::all() {
        let style = id.style();
        for (depth, depth_name) in DEPTHS {
            let buf = draw_synthetic(style, &theme(depth), area);
            let mut text = glyphs(&buf);
            if depth == ColorDepth::Ansi16 {
                text.push_str("-- fg\n");
                text.push_str(&ansi_fg(&buf));
            }
            assert_snapshot(&format!("{}_{depth_name}", style.name()), &text);
        }
    }
}

#[test]
fn every_style_shows_the_wax_at_every_depth() {
    let area = Rect::new(0, 0, 36, 14);
    for id in StyleId::all() {
        for (depth, depth_name) in DEPTHS {
            let buf = draw_synthetic(id.style(), &theme(depth), area);
            let inked = buf.content().iter().filter(|c| c.symbol() != " ").count();
            let name = id.style().name();
            assert!(
                inked > 20,
                "{name} @ {depth_name}: only {inked} inked cells"
            );
            let distinct: std::collections::HashSet<_> =
                buf.content().iter().map(|c| c.symbol()).collect();
            if depth == ColorDepth::None {
                assert!(distinct.len() >= 2, "{name}: no shape in NO_COLOR");
            }
            if depth == ColorDepth::TrueColor {
                // The liquid is painted, never left to the terminal.
                assert!(buf.content().iter().all(|c| c.bg != Color::Reset), "{name}");
            }
        }
    }
}

/// Styles may animate on `Canvas::time`, but only as a pure function of
/// it: the same time always draws the same frame.
#[test]
fn animation_is_a_pure_function_of_time() {
    let area = Rect::new(0, 0, 36, 14);
    let theme = theme(ColorDepth::TrueColor);
    let draw_at = |style: &StyleEntry, time: f64| draw_synthetic_at(style, &theme, area, time);
    for id in StyleId::all() {
        let style = id.style();
        assert_eq!(
            draw_at(style, 7.25),
            draw_at(style, 7.25),
            "{}",
            style.name()
        );
    }
    for name in ["synthwave", "matrix"] {
        let style = StyleId::by_name(name).unwrap().style();
        assert_ne!(
            draw_at(style, 0.0),
            draw_at(style, 2.5),
            "{name} should animate"
        );
    }
}

/// Matrix rain is only visible through the wax: liquid cells stay blank.
#[test]
fn matrix_rain_stays_inside_the_wax() {
    let area = Rect::new(0, 0, 36, 14);
    let matrix = StyleId::by_name("matrix").unwrap().style();
    let samples = synthetic(36, 14, 36.0 / 28.0);
    for time in [0.0, 1.0, 4.5] {
        let buf = draw_synthetic_at(matrix, &theme(ColorDepth::TrueColor), area, time);
        for (cell, s) in buf.content().iter().zip(&samples) {
            assert_eq!(cell.symbol() != " ", s.density >= SURFACE);
        }
    }
}

/// Render the live sim through `LampView` at many sizes. Every cell of the
/// area must be written and nothing outside it touched.
#[test]
fn lamp_view_fills_area_and_stays_inside_at_all_sizes() {
    let mut world = World::new(7, 1.6);
    world.prewarm(400, 1.0 / 120.0);
    let mut field = Field::default();
    field.prepare(&world, 0.5);

    let theme = theme(ColorDepth::TrueColor);
    let mut state = LampState::default();
    let sentinel = {
        let mut c = Cell::default();
        c.set_symbol("§");
        c
    };
    let widths = (1..=120).step_by(7).chain([2, 3, 120]);
    let sizes: Vec<_> = widths
        .flat_map(|w| (1..=40).step_by(3).chain([2, 40]).map(move |h| (w, h)))
        .collect();
    for id in StyleId::all() {
        for &(w, h) in &sizes {
            // Offset area inside a bigger buffer, with a sentinel border.
            let outer = Rect::new(0, 0, w + 4, h + 3);
            let area = Rect::new(2, 1, w, h);
            let mut buf = Buffer::filled(outer, sentinel.clone());
            let view = LampView {
                field: &field,
                style: id.style(),
                theme: &theme,
                time: 0.0,
                options: LampOptions::default(),
            };
            view.render(area, &mut buf, &mut state);
            for pos in outer.positions() {
                let written = buf[pos].symbol() != "§";
                assert_eq!(
                    written,
                    area.contains(pos),
                    "{} {w}x{h}: cell {pos:?}",
                    id.style().name()
                );
            }
        }
    }
}

#[test]
fn lamp_view_handles_empty_and_clipped_areas() {
    let mut world = World::new(3, 1.0);
    world.prewarm(10, 1.0 / 120.0);
    let mut field = Field::default();
    field.prepare(&world, 1.0);
    let theme = theme(ColorDepth::Ansi16);
    let mut state = LampState::default();
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 5));
    for area in [
        Rect::new(0, 0, 0, 0),
        Rect::new(3, 3, 0, 4),
        Rect::new(8, 3, 10, 10), // partly off the buffer
        Rect::new(20, 20, 5, 5), // entirely off it
    ] {
        for id in StyleId::all() {
            let view = LampView {
                field: &field,
                style: id.style(),
                theme: &theme,
                time: 0.0,
                options: LampOptions::default(),
            };
            view.render(area, &mut buf, &mut state);
        }
    }
}

/// A lamp inside a wider frame must use the frame's stride without
/// shifting either the sampled wax or the lamp-anchored dither pattern.
#[test]
fn lamp_view_matches_origin_in_offset_wider_buffers() {
    for (w, h) in [(160, 45), (200, 60)] {
        let mut world = World::new(2, f64::from(w) / (2.0 * f64::from(h)));
        world.prewarm(600, 1.0 / 120.0);
        let mut field = Field::default();
        field.prepare(&world, 0.5);
        for (depth, _) in DEPTHS {
            let theme = theme(depth);
            for id in StyleId::all() {
                for reduced in [false, true] {
                    let origin = Rect::new(0, 0, w, h);
                    let offset = Rect::new(7, 3, w, h);
                    let mut a = Buffer::empty(origin);
                    // Give the buffer itself an origin too, to catch code
                    // indexing content with absolute screen coordinates.
                    let mut b = Buffer::empty(Rect::new(2, 1, w + 20, h + 8));
                    for (area, buf) in [(origin, &mut a), (offset, &mut b)] {
                        LampView {
                            field: &field,
                            style: id.style(),
                            theme: &theme,
                            time: 7.25,
                            options: LampOptions {
                                reduced,
                                translucent: false,
                            },
                        }
                        .render(area, buf, &mut LampState::default());
                    }
                    for y in 0..h {
                        for x in 0..w {
                            assert_eq!(
                                a[(x, y)],
                                b[(x + offset.x, y + offset.y)],
                                "{} {depth:?} {w}x{h} reduced={reduced} at {x},{y}",
                                id.style().name(),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Over the sample budget the field is sampled coarser and upsampled.
#[test]
fn over_budget_upsamples() {
    let mut world = World::new(5, 2.0);
    world.prewarm(300, 1.0 / 120.0);
    let mut field = Field::default();
    field.prepare(&world, 0.0);
    let theme = theme(ColorDepth::TrueColor);
    let mut state = LampState::default();
    // Braille at 400×150 = 480k samples.
    let area = Rect::new(0, 0, 400, 150);
    let mut buf = Buffer::empty(area);
    let outline = StyleId::by_name("outline").unwrap().style();
    assert_eq!(outline.grid(), Grid::BRAILLE);
    LampView {
        field: &field,
        style: outline,
        theme: &theme,
        time: 0.0,
        options: LampOptions::default(),
    }
    .render(area, &mut buf, &mut state);
    assert!(state.coarse.len() <= SAMPLE_BUDGET);
    assert_eq!(state.samples.len(), 800 * 600);
    assert!(buf.content().iter().any(|c| c.symbol() != " "));
}

#[test]
fn upsample_is_bilinear_and_exact_at_same_size() {
    let src: Vec<Sample> = (0..6)
        .map(|i| Sample {
            density: i as f32,
            temp: 0.5,
        })
        .collect();
    let mut same = vec![Sample::default(); 6];
    upsample(&src, 3, 2, &mut same, 3, 2);
    assert_eq!(same, src);
    let mut big = vec![Sample::default(); 6 * 4];
    upsample(&src, 3, 2, &mut big, 6, 4);
    // Corners clamp to the corner samples; values stay within range.
    assert_eq!(big[0].density, 0.0);
    assert_eq!(big[23].density, 5.0);
    assert!(big.iter().all(|s| (0.0..=5.0).contains(&s.density)));
}

#[test]
fn registry_cycles_and_names_are_unique() {
    let names: Vec<_> = StyleId::all().map(|id| id.style().name()).collect();
    assert_eq!(
        names,
        [
            "solid",
            "outline",
            "ascii",
            "braille",
            "halftone",
            "synthwave",
            "matrix",
            "topo",
            "chrome",
        ]
    );
    assert_eq!(StyleId::default().style().name(), "solid");
    for (i, id) in StyleId::all().enumerate() {
        assert_eq!(id.index(), i);
    }
    for name in &names {
        assert_eq!(StyleId::by_name(name).unwrap().style().name(), *name);
        assert!(name.chars().all(|c| c.is_ascii_lowercase()));
    }
    assert!(StyleId::by_name("nope").is_none());
    // The 12th style was called glass before it was chrome (lava-ebq.23).
    assert_eq!(StyleId::by_name("glass"), StyleId::by_name("chrome"));
    for id in StyleId::all() {
        let g = id.style().grid();
        assert!((1..=4).contains(&g.x) && (1..=4).contains(&g.y));
    }
    let _ = Grid::CELL;
}

/// Frame time and output size per style, on the live sim.
/// `cargo test --release -- --ignored --nocapture bench_lamp`
#[test]
#[ignore = "benchmark"]
fn bench_lamp() {
    use ratatui::backend::{Backend, CrosstermBackend};
    use std::time::{Duration, Instant};

    const FRAMES: u32 = 300;
    for (cols, rows) in [(80u16, 24u16), (200, 60)] {
        for (depth, depth_name, translucent) in [
            (ColorDepth::TrueColor, "truecolor", false),
            (ColorDepth::Ansi256, "256", false),
            (ColorDepth::Ansi16, "16", false),
            (ColorDepth::TrueColor, "truecolor, translucent cells", true),
        ] {
            let theme = theme(depth);
            let mut report = format!("{cols}x{rows} {depth_name}:");
            for id in StyleId::all() {
                let aspect = f64::from(cols) / (2.0 * f64::from(rows));
                let mut world = World::new(7, aspect);
                world.prewarm(1200, 1.0 / 120.0);
                let mut field = Field::default();
                let mut state = LampState::default();
                let area = Rect::new(0, 0, cols, rows);
                let (mut prev, mut next) = (Buffer::empty(area), Buffer::empty(area));
                let (mut render_time, mut bytes, mut changed) = (Duration::ZERO, 0usize, 0usize);
                for frame in 0..FRAMES {
                    // 60 fps against a 120 Hz sim: two steps per frame.
                    world.step(1.0 / 120.0);
                    world.step(1.0 / 120.0);
                    let t0 = Instant::now();
                    field.prepare(&world, 1.0);
                    next.reset();
                    LampView {
                        field: &field,
                        style: id.style(),
                        theme: &theme,
                        time: f64::from(frame) / 60.0,
                        options: LampOptions {
                            reduced: false,
                            translucent,
                        },
                    }
                    .render(area, &mut next, &mut state);
                    render_time += t0.elapsed();
                    let diff = prev.diff(&next);
                    // Skip the first (full-screen) frame in the averages.
                    if frame > 0 {
                        changed += diff.len();
                        let mut out = Vec::<u8>::new();
                        let mut backend = CrosstermBackend::new(&mut out);
                        backend.draw(diff.into_iter()).unwrap();
                        Backend::flush(&mut backend).unwrap();
                        bytes += out.len();
                    }
                    std::mem::swap(&mut prev, &mut next);
                }
                let n = (FRAMES - 1) as usize;
                let _ = write!(
                    report,
                    "\n  {:9} {:>7.2?}/frame  {:>5} cells  {:>6} B/frame  {:>4} KB/s@60",
                    id.style().name(),
                    render_time / FRAMES,
                    changed / n,
                    bytes / n,
                    bytes / n * 60 / 1024,
                );
            }
            println!("{report}");
        }
    }
}

/// Wax against the left wall only: column 0 is compared with itself, not
/// wrapped to the far edge, so it gets no contour dots (lava-ebq.17).
#[test]
fn topo_left_wall_has_no_wrapped_contours() {
    let (cols, rows) = (8u16, 4u16);
    let (width, height) = (usize::from(cols) * 2, usize::from(rows) * 4);
    let samples: Vec<Sample> = (0..width * height)
        .map(|i| Sample {
            density: if i % width < 3 { 1.0 } else { 0.0 },
            temp: 0.5,
        })
        .collect();
    let theme = theme(ColorDepth::TrueColor);
    let area = Rect::new(0, 0, cols, rows);
    let canvas = Canvas {
        area,
        samples: &samples,
        width,
        height,
        theme: &theme,
        time: 0.0,
        translucent: false,
    };
    let mut buf = Buffer::empty(area);
    StyleId::by_name("topo")
        .unwrap()
        .style()
        .draw(&canvas, &mut buf);
    for y in 0..rows {
        assert_eq!(buf[(0, y)].symbol(), " ", "row {y}: column 0 has dots");
        // The real surface crossing (x = 2) is still traced.
        assert_ne!(buf[(1, y)].symbol(), " ", "row {y}: surface missing");
    }
}

/// In 256 colours every blend is resolved to an index (no RGB escapes
/// reach the terminal), the dither is a pure function of the frame (a still
/// picture stays still).
#[test]
fn ansi256_lamp_is_all_indexed_and_still() {
    let area = Rect::new(0, 0, 36, 14);
    for palette in Palette::all() {
        let theme = Theme::new(palette, ColorDepth::Ansi256);
        for id in StyleId::all() {
            let buf = draw_synthetic(id.style(), &theme, area);
            for cell in buf.content() {
                assert!(
                    !matches!(cell.fg, Color::Rgb(..)) && !matches!(cell.bg, Color::Rgb(..)),
                    "{} {}",
                    palette.name,
                    id.style().name()
                );
            }
            assert_eq!(buf, draw_synthetic(id.style(), &theme, area));
        }
    }
}
