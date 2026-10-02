//! A model of how a terminal shows half-block cells, for tests: each
//! cell's top and bottom pixel as the screen composites it, opaque or with
//! translucent cell backgrounds (Ghostty's `background-opacity-cells`:
//! glyphs stay opaque, backgrounds are blended over the window behind).
//! `opacity_pngs` writes whole lamps as pictures for eyeballing.

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::StatefulWidget;

use super::{LampOptions, LampState, LampView, StyleId};
use crate::sim::{Field, World};
use crate::theme::{ColorDepth, Palette, Rgb, Theme};

/// What shows through a translucent background: a dark desktop.
pub const WINDOW: Rgb = Rgb(12, 12, 16);
/// The user's Ghostty `background-opacity`.
pub const ALPHA: f32 = 0.75;

/// A drawn half pixel: its colour and whether it's the cell's foreground
/// (a glyph, opaque) or its background (translucent under `background-
/// opacity-cells`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Half {
    pub color: Color,
    pub glyph: bool,
}

/// A half-block (or blank) cell's top and bottom halves; `None` for any
/// other glyph.
pub fn halves(cell: &Cell) -> Option<[Half; 2]> {
    let fg = Half {
        color: cell.fg,
        glyph: true,
    };
    let bg = Half {
        color: cell.bg,
        glyph: false,
    };
    match cell.symbol() {
        "▀" => Some([fg, bg]),
        "▄" => Some([bg, fg]),
        "█" => Some([fg, fg]),
        " " => Some([bg, bg]),
        _ => None,
    }
}

/// How `half` shows on screen: as drawn, or with its background blended
/// `alpha` over [`WINDOW`].
pub fn shown(half: Half, alpha: f32) -> Rgb {
    let Color::Rgb(r, g, b) = half.color else {
        panic!("composite: truecolor only, got {:?}", half.color);
    };
    let c = Rgb(r, g, b);
    if half.glyph { c } else { WINDOW.lerp(c, alpha) }
}

/// `style` drawing a live lamp of `area` (seeded, prewarmed) in truecolor.
pub fn lamp(
    style: StyleId,
    palette: &'static Palette,
    seed: u64,
    area: Rect,
    translucent: bool,
) -> Buffer {
    lamp_with(
        style,
        palette,
        seed,
        area,
        translucent,
        &mut LampState::default(),
    )
}

/// [`lamp`], leaving the samples it drew from in `state`.
fn lamp_with(
    style: StyleId,
    palette: &'static Palette,
    seed: u64,
    area: Rect,
    translucent: bool,
    state: &mut LampState,
) -> Buffer {
    let aspect = f64::from(area.width) / f64::from(area.height) / 2.0;
    let mut world = World::new(seed, aspect);
    world.prewarm(900, 1.0 / 120.0);
    let mut field = Field::default();
    field.prepare(&world, 0.0);
    let theme = Theme::new(palette, ColorDepth::TrueColor);
    let mut buf = Buffer::empty(area);
    LampView {
        field: &field,
        style: style.style(),
        theme: &theme,
        time: 1.0,
        options: LampOptions {
            reduced: false,
            translucent,
        },
    }
    .render(area, &mut buf, state);
    buf
}

/// Write the lamps of `$LAVATUI_PNG_OUT` (default: the temp dir) as
/// `<style>-<terminal>.png`, 6×12 screen pixels a cell.
#[test]
#[ignore = "writes PNGs: LAVATUI_PNG_OUT=/some/dir cargo test opacity_pngs -- --ignored"]
fn opacity_pngs() {
    const CW: u32 = 6;
    const CH: u32 = 12;
    let out = std::env::var_os("LAVATUI_PNG_OUT")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    std::fs::create_dir_all(&out).unwrap();
    let area = Rect::new(0, 0, 120, 36);
    for name in ["solid", "synthwave", "chrome"] {
        let style = StyleId::by_name(name).unwrap();
        // As drawn for each terminal (`display.cells`), and the
        // translucent drawing on an opaque terminal, to see its cost.
        for (kind, translucent, alpha) in [
            ("opaque", false, 1.0),
            ("translucent", true, ALPHA),
            ("translucent-on-opaque", true, 1.0),
        ] {
            let buf = lamp(style, &Palette::all()[0], 2, area, translucent);
            let mut img =
                image::RgbImage::new(u32::from(area.width) * CW, u32::from(area.height) * CH);
            for (pos, cell) in area.positions().zip(buf.content()) {
                let [top, bottom] = halves(cell).expect("half-block style");
                for dy in 0..CH {
                    let Rgb(r, g, b) = shown(if dy < CH / 2 { top } else { bottom }, alpha);
                    for dx in 0..CW {
                        img.put_pixel(
                            u32::from(pos.x) * CW + dx,
                            u32::from(pos.y) * CH + dy,
                            image::Rgb([r, g, b]),
                        );
                    }
                }
            }
            let path = out.join(format!("{name}-{kind}.png"));
            img.save(&path).unwrap();
            println!("{}", path.display());
        }
    }
}

/// The half-block styles that blend (the ones ever drawn in two colours
/// a cell), and the palettes with colours to blend.
const STYLES: [&str; 3] = ["solid", "synthwave", "chrome"];

fn palettes() -> impl Iterator<Item = &'static Palette> {
    Palette::all()
        .iter()
        .filter(|p| Theme::new(p, ColorDepth::TrueColor).blends())
}

fn rgb(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// The half-row seams of lava-di9: wherever a cell's two halves look the
/// same on an opaque terminal they still do with translucent backgrounds
/// (never one in the glyph and one in the background), whichever way the
/// lamp draws.
#[test]
fn matching_halves_stay_matched_with_translucent_backgrounds() {
    let theme = Theme::new(&Palette::all()[0], ColorDepth::TrueColor);
    let mut bad = Vec::new();
    for name in STYLES {
        let style = StyleId::by_name(name).unwrap();
        for palette in palettes() {
            for (seed, area) in [(2, Rect::new(0, 0, 80, 26)), (7, Rect::new(0, 0, 40, 30))] {
                for translucent in [false, true] {
                    let buf = lamp(style, palette, seed, area, translucent);
                    for (pos, cell) in area.positions().zip(buf.content()) {
                        let [top, bottom] = halves(cell).expect("half-block style");
                        let opaque = (shown(top, 1.0), shown(bottom, 1.0));
                        let seen = (shown(top, ALPHA), shown(bottom, ALPHA));
                        if theme.near(rgb(opaque.0), rgb(opaque.1))
                            && !theme.near(rgb(seen.0), rgb(seen.1))
                        {
                            bad.push(format!(
                                "{name} {} seed {seed} translucent {translucent} {pos:?}: {cell:?}",
                                palette.name
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} seams, e.g. {:#?}",
        bad.len(),
        &bad[..bad.len().min(5)]
    );
}

/// Drawn for translucent backgrounds, a cell of wax is one opaque glyph,
/// and in every mode the liquid is always a background (as see-through
/// as the window), never the glyph.
#[test]
fn wax_is_whole_and_liquid_is_background() {
    use crate::render::coverage;
    use crate::sim::SURFACE;

    for name in STYLES {
        let style = StyleId::by_name(name).unwrap();
        for palette in palettes() {
            for translucent in [false, true] {
                let area = Rect::new(0, 0, 60, 24);
                let mut state = LampState::default();
                let buf = lamp_with(style, palette, 3, area, translucent, &mut state);
                let width = usize::from(area.width);
                let sample =
                    |cx: usize, cy: usize, row: usize| state.samples[(2 * cy + row) * width + cx];
                for (i, cell) in buf.content().iter().enumerate() {
                    let (cx, cy) = (i % width, i / width);
                    let [top, bottom] = halves(cell).expect("half-block style");
                    let (t, b) = (sample(cx, cy, 0), sample(cx, cy, 1));
                    let at = format!(
                        "{name} {} translucent {translucent} ({cx}, {cy})",
                        palette.name
                    );
                    if translucent && t.density >= SURFACE && b.density >= SURFACE {
                        assert_eq!(cell.symbol(), "█", "{at}: {cell:?}");
                    }
                    // Synthwave's backdrop is a scene of its own, with lines.
                    if name != "synthwave" {
                        for (half, s) in [(top, t), (bottom, b)] {
                            if coverage(s.density) == 0.0 {
                                assert!(!half.glyph, "{at}: liquid in the glyph: {cell:?}");
                            }
                        }
                    }
                }
            }
        }
    }
}
