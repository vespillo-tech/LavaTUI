use std::fmt::Write as _;
use std::path::PathBuf;

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::StatefulWidget;

use super::*;
use crate::light::Lamplight;
use crate::sim::{Shape, World, ambient_temp};
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
    draw_synthetic_at(style, theme, area, 0.0, None)
}

fn draw_synthetic_lit(
    style: &StyleEntry,
    theme: &Theme,
    area: Rect,
    lighting: &dyn Lighting,
) -> Buffer {
    draw_synthetic_at(style, theme, area, 0.0, Some(lighting))
}

fn draw_synthetic_at(
    style: &StyleEntry,
    theme: &Theme,
    area: Rect,
    time: f64,
    lighting: Option<&dyn Lighting>,
) -> Buffer {
    let grid = style.grid();
    let width = usize::from(area.width * grid.x);
    let height = usize::from(area.height * grid.y);
    let aspect = f32::from(area.width) / (2.0 * f32::from(area.height));
    let samples = synthetic(width, height, aspect);
    let mask = vec![(0, width); height];
    let light = lighting.map(|lighting| {
        let mut light = vec![1.0; samples.len()];
        lighting.shade(&samples, width, height, &mut light);
        light
    });
    let canvas = Canvas {
        area,
        samples: &samples,
        light: light.as_deref(),
        mask: &mask,
        width,
        height,
        theme,
        time,
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
    let draw_at =
        |style: &StyleEntry, time: f64| draw_synthetic_at(style, &theme, area, time, None);
    for id in StyleId::all() {
        let style = id.style();
        assert_eq!(
            draw_at(style, 7.25),
            draw_at(style, 7.25),
            "{}",
            style.name()
        );
    }
    for name in ["crt", "synthwave", "matrix"] {
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
        let buf = draw_synthetic_at(matrix, &theme(ColorDepth::TrueColor), area, time, None);
        for (cell, s) in buf.content().iter().zip(&samples) {
            assert_eq!(cell.symbol() != " ", s.density >= SURFACE);
        }
    }
}

/// Render the live sim through `LampView` at many sizes. Every cell of the
/// area must be written and nothing outside it touched.
#[test]
fn lamp_view_fills_area_and_stays_inside_at_all_sizes() {
    let mut world = World::new(7, 1.6, Shape::Tank);
    world.prewarm(400, 1.0 / 120.0);
    let mut field = Field::default();
    field.prepare(&world, 0.5);
    let mut bottle = World::new(7, 0.8, Shape::Bottle);
    bottle.prewarm(400, 1.0 / 120.0);
    let mut bottle_field = Field::default();
    bottle_field.prepare(&bottle, 0.5);

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
        for (i, &(w, h)) in sizes.iter().enumerate() {
            let field = if i % 2 == 0 { &field } else { &bottle_field };
            // Offset area inside a bigger buffer, with a sentinel border.
            let outer = Rect::new(0, 0, w + 4, h + 3);
            let area = Rect::new(2, 1, w, h);
            let mut buf = Buffer::filled(outer, sentinel.clone());
            let view = LampView {
                field,
                style: id.style(),
                theme: &theme,
                time: 0.0,
                lighting: None,
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
    let mut world = World::new(3, 1.0, Shape::Tank);
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
                lighting: None,
                options: LampOptions::default(),
            };
            view.render(area, &mut buf, &mut state);
        }
    }
}

/// Over the sample budget the field is sampled coarser and upsampled.
#[test]
fn over_budget_upsamples() {
    let mut world = World::new(5, 2.0, Shape::Tank);
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
        lighting: None,
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

struct Bright;

impl Lighting for Bright {
    fn shade(&self, _: &[Sample], width: usize, height: usize, out: &mut [f32]) {
        assert_eq!(out.len(), width * height);
        assert!(out.iter().all(|&l| l == 1.0));
        out.fill(1.5);
    }
}

#[test]
fn lighting_seam_reaches_styles() {
    let mut world = World::new(9, 1.0, Shape::Tank);
    world.prewarm(300, 1.0 / 120.0);
    let mut field = Field::default();
    field.prepare(&world, 0.0);
    let theme = theme(ColorDepth::TrueColor);
    let mut state = LampState::default();
    let area = Rect::new(0, 0, 20, 10);
    let solid = StyleId::by_name("solid").unwrap().style();
    let mut render = |lighting: Option<&dyn Lighting>| {
        let mut buf = Buffer::empty(area);
        let view = LampView {
            field: &field,
            style: solid,
            theme: &theme,
            time: 0.0,
            lighting,
            options: LampOptions::default(),
        };
        view.render(area, &mut buf, &mut state);
        buf
    };
    let (plain, lit) = (render(None), render(Some(&Bright)));
    let luma = |buf: &Buffer| -> u32 {
        buf.content()
            .iter()
            .map(|c| match c.fg {
                Color::Rgb(r, g, b) => u32::from(r) + u32::from(g) + u32::from(b),
                _ => 0,
            })
            .sum()
    };
    assert!(luma(&lit) > luma(&plain));
}

#[test]
fn unlit_canvas_is_exactly_one() {
    let samples = synthetic(8, 8, 1.0);
    let mask = vec![(0, 8); 8];
    let theme = theme(ColorDepth::TrueColor);
    let canvas = Canvas {
        area: Rect::new(0, 0, 8, 4),
        samples: &samples,
        light: None,
        mask: &mask,
        width: 8,
        height: 8,
        theme: &theme,
        time: 0.0,
    };
    assert!((0..8).all(|y| (0..8).all(|x| canvas.light(x, y) == 1.0)));
    assert_eq!(lit(0.3, 1.0), 0.3);
    assert!(lit(0.3, 1.4) > 0.3 && lit(0.3, 0.7) < 0.3);
}

/// Every style responds to the real lighting pass: in colour by shading,
/// in 16 colours / NO_COLOR (where it can) through glyph density (§5.3).
#[test]
fn every_style_responds_to_lighting() {
    let area = Rect::new(0, 0, 36, 14);
    for id in StyleId::all() {
        let name = id.style().name();
        for (depth, depth_name) in DEPTHS {
            let theme = theme(depth);
            let plain = draw_synthetic(id.style(), &theme, area);
            let lit = draw_synthetic_lit(id.style(), &theme, area, &Lamplight);
            let changed = plain
                .content()
                .iter()
                .zip(lit.content())
                .filter(|(a, b)| a != b)
                .count();
            let glyphs_only = matches!(depth, ColorDepth::Ansi16 | ColorDepth::None);
            // Without colour only glyph-density styles can show light;
            // the rest are silhouettes there.
            let density = matches!(
                name,
                "heatmap" | "ascii" | "dither" | "braille" | "halftone"
            );
            if !glyphs_only || density {
                assert!(changed > 5, "{name} @ {depth_name}: {changed} cells lit");
            }
        }
    }
}

#[test]
fn registry_cycles_and_names_are_unique() {
    let names: Vec<_> = StyleId::all().map(|id| id.style().name()).collect();
    assert_eq!(
        names,
        [
            "solid",
            "outline",
            "heatmap",
            "ascii",
            "dither",
            "braille",
            "halftone",
            "crt",
            "synthwave",
            "matrix",
            "topo",
            "glass",
        ]
    );
    let n = names.len();
    let first = StyleId::default();
    assert_eq!(first.style().name(), "solid");
    let mut id = first;
    for i in 0..n {
        assert_eq!(id.index(), i);
        assert_eq!(id.next().prev(), id);
        id = id.next();
    }
    assert_eq!(id, first);
    assert_eq!(first.prev().index(), n - 1);
    for name in &names {
        assert_eq!(StyleId::by_name(name).unwrap().style().name(), *name);
        assert!(name.chars().all(|c| c.is_ascii_lowercase()));
    }
    assert!(StyleId::by_name("nope").is_none());
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
        for (depth, depth_name) in [
            (ColorDepth::TrueColor, "truecolor"),
            (ColorDepth::Ansi16, "16"),
        ] {
            let theme = theme(depth);
            let mut report = format!("{cols}x{rows} {depth_name}:");
            for (id, lighting) in StyleId::all()
                .flat_map(|id| [None, Some(&Lamplight as &dyn Lighting)].map(|l| (id, l)))
            {
                let aspect = f64::from(cols) / (2.0 * f64::from(rows));
                let mut world = World::new(7, aspect, Shape::Tank);
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
                        lighting,
                        options: LampOptions::default(),
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
                    "\n  {:8} {:5} {:>7.2?}/frame  {:>5} cells  {:>6} B/frame  {:>4} KB/s@60",
                    id.style().name(),
                    if lighting.is_some() { "lit" } else { "" },
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

#[test]
fn lighting_stays_inside_the_glass() {
    let mut world = World::new(9, 0.6, Shape::Bottle);
    world.prewarm(300, 1.0 / 120.0);
    let mut field = Field::default();
    field.prepare(&world, 0.0);
    let theme = theme(ColorDepth::TrueColor);
    let mut state = LampState::default();
    let area = Rect::new(0, 0, 24, 20);
    let solid = StyleId::by_name("solid").unwrap().style();
    let mut render = |lighting: Option<&dyn Lighting>| {
        let mut buf = Buffer::empty(area);
        let time = 0.0;
        LampView {
            field: &field,
            style: solid,
            theme: &theme,
            time,
            lighting,
            options: LampOptions::default(),
        }
        .render(area, &mut buf, &mut state);
        buf
    };
    let (plain, lit) = (render(None), render(Some(&Bright)));
    // The bottle's top corners are outside the glass: untouched by light.
    for x in [0, 1, area.width - 2, area.width - 1] {
        assert_eq!(plain[(x, 0)], lit[(x, 0)], "column {x}");
    }
    assert_ne!(plain, lit);
}

/// Bottle walls are cut at half columns: cells the wall passes through
/// become quadrant glyphs in front of `bg`, mirrored left ↔ right, and
/// only when the theme can show a liquid tint.
#[test]
fn bottle_walls_are_half_cells_and_mirrored() {
    let mut world = World::new(9, 0.6, Shape::Bottle);
    world.prewarm(300, 1.0 / 120.0);
    let mut field = Field::default();
    field.prepare(&world, 0.0);
    let mut state = LampState::default();
    let solid = StyleId::by_name("solid").unwrap().style();
    let quadrant = |s: &str| "▗▖▄▝▐▞▟▘▚▌▙▀▜▛".contains(s) && s != " ";
    for (cols, depth) in [
        (23, ColorDepth::TrueColor),
        (24, ColorDepth::Ansi256),
        (23, ColorDepth::Ansi16),
    ] {
        let theme = theme(depth);
        let area = Rect::new(0, 0, cols, 20);
        let mut buf = Buffer::empty(area);
        LampView {
            field: &field,
            style: solid,
            theme: &theme,
            time: 0.0,
            lighting: None,
            options: LampOptions::default(),
        }
        .render(area, &mut buf, &mut state);
        let bg = theme.role(Role::Bg);
        let mut edges = 0;
        for y in 0..area.height {
            for x in 0..cols {
                let cell = &buf[(x, y)];
                if !theme.blends() {
                    // No liquid tint: no reshaping, the glass draws a
                    // `▕ │ ▏` edge instead (ui::glass). Solid only uses
                    // half blocks.
                    assert!("▀▄█ ".contains(cell.symbol()), "{depth:?}");
                    continue;
                }
                if cell.bg != bg || !quadrant(cell.symbol()) {
                    continue;
                }
                edges += 1;
                let other = buf[(cols - 1 - x, y)].symbol();
                let mirrored: String = cell
                    .symbol()
                    .chars()
                    .map(|c| match c {
                        '▐' => '▌',
                        '▌' => '▐',
                        '▗' => '▖',
                        '▖' => '▗',
                        '▝' => '▘',
                        '▘' => '▝',
                        '▟' => '▙',
                        '▙' => '▟',
                        '▜' => '▛',
                        '▛' => '▜',
                        c => c,
                    })
                    .collect();
                assert_eq!(other, mirrored, "{cols} cols, row {y}, cell {x}");
            }
        }
        if theme.blends() {
            assert!(edges > 5, "{depth:?}: only {edges} half-cell edges");
        }
    }
}

/// The synthetic field drawn as `LampView` draws a bottle: mask from the
/// walls, then the half-cell edge pass.
fn draw_synthetic_bottle(style: &StyleEntry, theme: &Theme, area: Rect) -> Buffer {
    let grid = style.grid();
    let width = usize::from(area.width * grid.x);
    let height = usize::from(area.height * grid.y);
    let aspect = f32::from(area.width) / (2.0 * f32::from(area.height));
    let mut samples = synthetic(width, height, aspect);
    let mut mask = Vec::new();
    walls::mask(Shape::Bottle, area, grid, &mut mask);
    // The sim keeps wax inside its walls; so does this field.
    for (row, &(lo, hi)) in samples.chunks_exact_mut(width).zip(&mask) {
        for (x, s) in row.iter_mut().enumerate() {
            if !(lo..hi).contains(&x) {
                s.density = 0.0;
            }
        }
    }
    let canvas = Canvas {
        area,
        samples: &samples,
        light: None,
        mask: &mask,
        width,
        height,
        theme,
        time: 0.0,
    };
    let mut buf = Buffer::empty(area);
    style.draw(&canvas, &mut buf);
    if theme.blends() {
        walls::smooth(Shape::Bottle, theme, theme.role(Role::Bg), area, &mut buf);
    }
    buf
}

/// Outside the glass every style leaves the plain app background: no
/// scanlines, vignette or tint (lava-ebq.12). Snapshots show glyphs plus a
/// map of cells that show only `bg` (`.`), the half-cell wall
/// glyphs (`|`), and everything else (`#`).
#[test]
fn bottle_snapshots_leave_the_outside_plain() {
    let area = Rect::new(0, 0, 24, 14);
    let mut mask = Vec::new();
    walls::mask(Shape::Bottle, area, Grid::CELL, &mut mask);
    for name in ["solid", "crt"] {
        let style = StyleId::by_name(name).unwrap().style();
        for (depth, depth_name) in [
            (ColorDepth::TrueColor, "truecolor"),
            (ColorDepth::None, "none"),
        ] {
            let theme = theme(depth);
            let buf = draw_synthetic_bottle(style, &theme, area);
            let bg = theme.role(Role::Bg);
            let mut map = String::new();
            for y in 0..area.height {
                let (lo, hi) = mask[usize::from(y)];
                for x in 0..area.width {
                    let cell = &buf[(x, y)];
                    let inside = (lo..hi).contains(&usize::from(x));
                    // Shows nothing but `bg` (solid paints `█` in it).
                    let plain = cell.bg == bg && (cell.symbol() == " " || cell.fg == bg);
                    if !inside {
                        assert!(plain, "{name} {depth_name}: ({x}, {y}) outside the glass");
                    }
                    map.push(match (plain, cell.bg == bg) {
                        (true, _) => '.',
                        (false, true) if theme.blends() => '|',
                        _ => '#',
                    });
                }
                map.push('\n');
            }
            let text = format!("{}-- bg\n{map}", glyphs(&buf));
            assert_snapshot(&format!("bottle_{name}_{depth_name}"), &text);
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
    let mask = vec![(0, width); height];
    let theme = theme(ColorDepth::TrueColor);
    let area = Rect::new(0, 0, cols, rows);
    let canvas = Canvas {
        area,
        samples: &samples,
        light: None,
        mask: &mask,
        width,
        height,
        theme: &theme,
        time: 0.0,
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
