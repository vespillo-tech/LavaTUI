use ratatui::style::{Color, Modifier};

use super::*;

fn lava() -> &'static Palette {
    Palette::by_name("lava").unwrap()
}

#[test]
fn palettes_are_complete_and_unique() {
    let names: Vec<_> = Palette::all().iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "lava",
            "ultraviolet",
            "abyss",
            "toxic",
            "synthwave",
            "mono",
            "paper",
            "ansi"
        ]
    );
    for palette in Palette::all() {
        for role in Role::ALL {
            let s = palette.swatch(role);
            if palette.name == "ansi" {
                assert_eq!((s.rgb, s.x256), (None, None), "ansi {role:?}");
            } else {
                assert!(
                    s.rgb.is_some() && s.x256.is_some(),
                    "{} {role:?}",
                    palette.name
                );
            }
        }
        // §5.3: in 16 colours the backgrounds are always the terminal's.
        assert_eq!(palette.swatch(Role::Bg).ansi, Color::Reset);
        assert_eq!(palette.swatch(Role::Liquid).ansi, Color::Reset);
        // The wax stops must be distinguishable in 16 colours.
        let wax = [Role::WaxCool, Role::WaxMid, Role::WaxHot].map(|r| palette.swatch(r).ansi);
        assert!(wax.iter().all(|&c| c != Color::Reset), "{}", palette.name);
    }
}

#[test]
fn spot_check_spec_values() {
    let s = lava().swatch(Role::WaxMid);
    assert_eq!(s.rgb, Some(Rgb(0xE2, 0x47, 0x1B)));
    assert_eq!(s.x256, Some(166));
    assert_eq!(s.ansi, Color::LightRed);
    let accent = Palette::by_name("synthwave").unwrap().swatch(Role::Accent);
    assert_eq!(accent.rgb, Some(Rgb::hex(0x2DE2E6)));
    assert_eq!(accent.x256, Some(44));
    assert!(Palette::by_name("nope").is_none());
}

#[test]
fn depth_detection_order() {
    use ColorDepth::*;
    let d = ColorDepth::from_env;
    assert_eq!(
        d(Some("1"), Some("truecolor"), Some("xterm-256color")),
        None
    );
    // An empty NO_COLOR doesn't count.
    assert_eq!(d(Some(""), Some("truecolor"), Option::None), TrueColor);
    assert_eq!(d(Option::None, Some("24bit"), Option::None), TrueColor);
    assert_eq!(d(Option::None, Some("TrueColor"), Option::None), TrueColor);
    assert_eq!(
        d(Option::None, Some("yes"), Some("xterm-256color")),
        Ansi256
    );
    assert_eq!(
        d(Option::None, Option::None, Some("screen-256color")),
        Ansi256
    );
    assert_eq!(d(Option::None, Option::None, Some("xterm")), Ansi16);
    assert_eq!(d(Option::None, Option::None, Option::None), Ansi16);
}

#[test]
fn truecolor_roles_exact_and_ramps_continuous() {
    let theme = Theme::new(lava(), ColorDepth::TrueColor);
    assert!(theme.blends() && theme.has_color());
    assert_eq!(theme.role(Role::WaxMid), Color::Rgb(0xE2, 0x47, 0x1B));
    let near = |c: Color, hex: u32| {
        let Color::Rgb(r, g, b) = c else {
            panic!("{c:?}")
        };
        let want = Rgb::hex(hex);
        [(r, want.0), (g, want.1), (b, want.2)]
            .iter()
            .all(|&(a, b)| a.abs_diff(b) <= TRUECOLOR_QUANT)
    };
    assert!(near(theme.color(Ink::Wax(0.0)), 0x8E1B12));
    assert!(near(theme.color(Ink::Wax(0.5)), 0xE2471B));
    assert!(near(theme.color(Ink::Wax(1.0)), 0xFFB04A));
    // Halfway between two stops is neither.
    let quarter = theme.color(Ink::Wax(0.25));
    assert_ne!(quarter, theme.color(Ink::Wax(0.0)));
    assert_ne!(quarter, theme.color(Ink::Wax(0.5)));
    // Mixing blends.
    let mid = theme
        .paint(Ink::Role(Role::Liquid))
        .mix(Ink::Role(Role::WaxHot), 0.5)
        .color();
    assert_ne!(mid, theme.role(Role::Liquid));
    assert_ne!(mid, theme.role(Role::WaxHot));
}

#[test]
fn ansi256_uses_hand_picked_indices_then_nearest() {
    let theme = Theme::new(lava(), ColorDepth::Ansi256);
    assert_eq!(theme.role(Role::Liquid), Color::Indexed(233));
    assert_eq!(theme.role(Role::WaxHot), Color::Indexed(215));
    for t in [0.0, 0.3, 0.7, 1.0] {
        assert!(matches!(theme.color(Ink::Wax(t)), Color::Indexed(_)));
    }
    assert_eq!(xterm::nearest(Rgb(255, 0, 0)), 196);
    assert_eq!(xterm::nearest(Rgb(0, 0, 0)), 16);
    assert_eq!(xterm::nearest(Rgb(128, 128, 128)), 244);
    assert_eq!(xterm::nearest(Rgb(95, 135, 175)), 67);
}

#[test]
fn ansi256_match_is_perceptual() {
    // Every cube / grey-ramp colour finds itself (through the cache's
    // 6-bit buckets).
    for i in 16..=255 {
        assert_eq!(xterm::nearest(xterm::rgb(i)), i, "{:?}", xterm::rgb(i));
    }
    let is_grey = |i: u8| i >= 232 || matches!(i, 16 | 59 | 102 | 145 | 188 | 231);
    // Near-neutrals land on greys, not tinted cube colours (paper's metal
    // used to go olive).
    for c in [
        Rgb(0x8A, 0x7E, 0x68),
        Rgb(0x6B, 0x5A, 0x4E),
        Rgb(0x30, 0x30, 0x32),
    ] {
        assert!(is_grey(xterm::nearest(c)), "{c:?} → {}", xterm::nearest(c));
    }
    // Dark tints stay dark: no jump to a loud cube colour.
    for c in [
        Rgb(0x1B, 0x0B, 0x2B),
        Rgb(0x17, 0x0F, 0x2C),
        Rgb(0x0B, 0x1A, 0x24),
    ] {
        let Rgb(r, g, b) = xterm::rgb(xterm::nearest(c));
        assert!(r.max(g).max(b) < 0x40, "{c:?} → {:?}", (r, g, b));
    }
    // Saturated colours keep their hue: orange wax edges don't go olive or
    // yellow (red stays well above green), purples stay purple.
    for c in [
        Rgb(0xB5, 0x48, 0x2A),
        Rgb(0xE2, 0x47, 0x1B),
        Rgb(0x8E, 0x1B, 0x12),
    ] {
        let Rgb(r, g, b) = xterm::rgb(xterm::nearest(c));
        assert!(
            r > g.saturating_add(60) && g >= b,
            "{c:?} → {:?}",
            (r, g, b)
        );
    }
    for c in [Rgb(0x4B, 0x1D, 0x8F), Rgb(0xB5, 0x17, 0x9E)] {
        let Rgb(r, g, b) = xterm::rgb(xterm::nearest(c));
        assert!(b > g && r > g, "{c:?} → {:?}", (r, g, b));
    }
}

#[test]
fn only_blending_256_dithers() {
    for palette in Palette::all() {
        for depth in [
            ColorDepth::TrueColor,
            ColorDepth::Ansi256,
            ColorDepth::Ansi16,
            ColorDepth::None,
        ] {
            let theme = Theme::new(palette, depth);
            assert_eq!(
                theme.dithering().is_some(),
                depth == ColorDepth::Ansi256 && theme.blends(),
                "{} {depth:?}",
                palette.name
            );
        }
    }
    let theme = Theme::new(lava(), ColorDepth::Ansi256).dithering().unwrap();
    // Unmixed roles keep their hand-picked index; blends wait for `dither`.
    assert_eq!(theme.role(Role::Liquid), Color::Indexed(233));
    let blend = theme.color(Ink::Wax(0.2));
    assert!(matches!(blend, Color::Rgb(..)));
    assert!(matches!(theme.dither(blend, 0.5), Color::Indexed(_)));
    assert_eq!(theme.dither(Color::Indexed(9), 0.5), Color::Indexed(9));
    assert_eq!(theme.dither(Color::Reset, 0.5), Color::Reset);
}

/// The indices `c` dithers to over all 64 thresholds of an 8×8 pattern.
fn dither_counts(c: Rgb) -> std::collections::BTreeMap<u8, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for v in 0..64 {
        *counts
            .entry(xterm::dither(c, (v as f32 + 0.5) / 64.0))
            .or_default() += 1;
    }
    counts
}

#[test]
fn dark_tints_dither_to_keep_their_hue() {
    // Dark purples and blues the cube can only show as grey get a
    // coloured second index mixed in; mixed in linear light they land
    // near the colour's own lightness.
    for c in [
        Rgb(0x29, 0x10, 0x4F),
        Rgb(0x2A, 0x0F, 0x30),
        Rgb(0x3C, 0x18, 0x0F),
        Rgb(0x0B, 0x4F, 0x6C),
    ] {
        let counts = dither_counts(c);
        assert_eq!(counts.len(), 2, "{c:?}: {counts:?}");
        let tinted = counts.keys().any(|&i| {
            let Rgb(r, g, b) = xterm::rgb(i);
            r.max(g).max(b) - r.min(g).min(b) > 40
        });
        assert!(tinted, "{c:?}: {counts:?}");
        let luma = |Rgb(r, g, b): Rgb| {
            let lin = |v: u8| (f32::from(v) / 255.0).powf(2.2);
            0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
        };
        let mixed: f32 = counts
            .iter()
            .map(|(&i, &n)| luma(xterm::rgb(i)) * n as f32 / 64.0)
            .sum();
        let want = luma(c);
        assert!(
            (mixed - want).abs() < want.max(0.01) * 0.6,
            "{c:?}: {mixed} vs {want}"
        );
    }
    // Synthwave's and ultraviolet's liquids have no dark index near their
    // hue: they stay grey, like their §5.2 index, rather than dithering
    // with blue (00005f, 30-40° off).
    for c in [Rgb(0x1B, 0x0B, 0x2B), Rgb(0x17, 0x0F, 0x2C)] {
        assert!(dither_counts(c).keys().all(|&i| i >= 232), "{c:?}");
    }
}

#[test]
fn colours_one_index_shows_well_never_dither() {
    // Exact cube / grey colours, mid tones and neutrals stay flat: a
    // pattern there costs more than it shows.
    let mut flat: Vec<Rgb> = (16..=255).map(xterm::rgb).collect();
    flat.extend([
        Rgb(0xB5, 0x17, 0x9E),
        Rgb(0x1A, 0x9B, 0xA8),
        Rgb(0xE2, 0x47, 0x1B),
        Rgb(0x4B, 0x1D, 0x8F),
        Rgb(0x16, 0x16, 0x16),
        Rgb(0x80, 0x80, 0x80),
    ]);
    for c in flat {
        let counts = dither_counts(c);
        assert_eq!(counts.len(), 1, "{c:?}: {counts:?}");
        assert_eq!(counts.keys().next(), Some(&xterm::nearest(c)), "{c:?}");
    }
}

#[test]
fn ansi16_steps_and_never_blends() {
    let theme = Theme::new(lava(), ColorDepth::Ansi16);
    assert!(!theme.blends());
    assert_eq!(theme.color(Ink::Wax(0.0)), Color::Red);
    assert_eq!(theme.color(Ink::Wax(0.5)), Color::LightRed);
    assert_eq!(theme.color(Ink::Wax(1.0)), Color::LightYellow);
    assert_eq!(theme.role(Role::Liquid), Color::Reset);
    let liquid = theme.paint(Ink::Role(Role::Liquid));
    assert_eq!(liquid.mix(Ink::Wax(1.0), 0.4).color(), Color::Reset);
    assert_eq!(liquid.mix(Ink::Wax(1.0), 0.6).color(), Color::LightYellow);
    assert_eq!(liquid.scale(3.0).color(), Color::Reset);
}

#[test]
fn no_color_is_reset_everywhere_and_accent_bold() {
    for palette in Palette::all() {
        let theme = Theme::new(palette, ColorDepth::None);
        assert!(!theme.has_color() && !theme.blends());
        for role in Role::ALL {
            assert_eq!(theme.role(role), Color::Reset);
        }
        for t in [0.0, 0.5, 1.0] {
            assert_eq!(theme.color(Ink::Wax(t)), Color::Reset);
        }
        assert!(
            theme
                .text(Role::Accent)
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(theme.text(Role::Dim).add_modifier.is_empty());
    }
}

#[test]
fn ansi_palette_uses_terminal_colours_at_every_depth() {
    let palette = Palette::by_name("ansi").unwrap();
    for depth in [
        ColorDepth::TrueColor,
        ColorDepth::Ansi256,
        ColorDepth::Ansi16,
    ] {
        let theme = Theme::new(palette, depth);
        assert!(!theme.blends(), "{depth:?}");
        assert_eq!(theme.role(Role::Bg), Color::Reset);
        assert_eq!(theme.color(Ink::Wax(1.0)), Color::LightYellow);
        assert_eq!(theme.role(Role::Accent), Color::Yellow);
    }
}

#[test]
fn every_palette_resolves_every_role_at_every_depth() {
    let depths = [
        ColorDepth::TrueColor,
        ColorDepth::Ansi256,
        ColorDepth::Ansi16,
        ColorDepth::None,
    ];
    for palette in Palette::all() {
        for depth in depths {
            let theme = Theme::new(palette, depth);
            assert_eq!(theme.depth(), depth);
            assert_eq!(theme.palette().name, palette.name);
            for role in Role::ALL {
                let c = theme.role(role);
                match depth {
                    ColorDepth::TrueColor if palette.has_rgb() => {
                        assert!(matches!(c, Color::Rgb(..)))
                    }
                    ColorDepth::Ansi256 if palette.has_rgb() => {
                        assert!(matches!(c, Color::Indexed(_)))
                    }
                    _ => assert!(!matches!(c, Color::Rgb(..) | Color::Indexed(_))),
                }
            }
        }
    }
}

#[test]
fn with_role_repaints_the_role_and_its_ramps() {
    let theme = Theme::new(lava(), ColorDepth::TrueColor);
    let accent = theme.role(Role::Accent);
    let liquid = theme.paint(Ink::Role(Role::Liquid));
    let flash = liquid.mix(Ink::Role(Role::Accent), 1.0);
    let flashed = theme.with_role(Role::Liquid, flash);
    assert_eq!(flashed.role(Role::Liquid), flash.color());
    // A wax role's ramp follows its repaint (to within truecolor
    // quantisation).
    let hot = theme.with_role(Role::WaxCool, liquid.mix(Ink::Role(Role::Accent), 1.0));
    let (Color::Rgb(r, g, b), Color::Rgb(ar, ag, ab)) = (hot.color(Ink::Wax(0.0)), accent) else {
        panic!("truecolor");
    };
    assert!(r.abs_diff(ar) <= 4 && g.abs_diff(ag) <= 4 && b.abs_diff(ab) <= 4);
    // Everything else is untouched.
    for role in Role::ALL.into_iter().filter(|&r| r != Role::Liquid) {
        assert_eq!(flashed.role(role), theme.role(role), "{role:?}");
    }
    assert_eq!(flashed.color(Ink::Wax(0.3)), theme.color(Ink::Wax(0.3)));

    // 256: an unmixed repaint keeps the hand-picked index.
    let t256 = Theme::new(lava(), ColorDepth::Ansi256);
    let same = t256.with_role(Role::Liquid, t256.paint(Ink::Role(Role::Metal)));
    assert_eq!(same.role(Role::Liquid), t256.role(Role::Metal));

    // 16: a mix under half never changes a colour that can't blend.
    let t16 = Theme::new(lava(), ColorDepth::Ansi16);
    let weak = t16.with_role(
        Role::Liquid,
        t16.paint(Ink::Role(Role::Liquid))
            .mix(Ink::Role(Role::Accent), 0.35),
    );
    assert_eq!(weak.role(Role::Liquid), t16.role(Role::Liquid));
}

#[test]
fn blend_mixes_resolved_colours_per_depth() {
    let tc = Theme::new(lava(), ColorDepth::TrueColor);
    let (a, b) = (Color::Rgb(0, 0, 0), Color::Rgb(200, 100, 40));
    assert_eq!(tc.blend(a, b, 0.0), a);
    assert_eq!(tc.blend(a, b, 1.0), b);
    assert_eq!(tc.blend(a, b, 0.5), Color::Rgb(100, 52, 20));
    // A terminal default can't be mixed: the dominant side wins.
    assert_eq!(tc.blend(Color::Reset, b, 0.4), Color::Reset);

    let t256 = Theme::new(lava(), ColorDepth::Ansi256);
    // Black (16) half way to white (231) lands on a mid grey.
    let Color::Indexed(i) = t256.blend(Color::Indexed(16), Color::Indexed(231), 0.5) else {
        panic!("256 blends to an index");
    };
    assert!((240..=246).contains(&i), "{i}");

    let t16 = Theme::new(lava(), ColorDepth::Ansi16);
    assert_eq!(t16.blend(Color::Red, Color::Yellow, 0.4), Color::Red);
    assert_eq!(t16.blend(Color::Red, Color::Yellow, 0.6), Color::Yellow);
}

/// How far (degrees) a shown index's hue may stray from the colour it
/// stands for, and the chroma (OKLab) from which a colour has a hue to
/// keep. Greys may stand in for anything.
const HUE_TOLERANCE: f32 = 35.0;
const CHROMATIC: f32 = 0.03;

/// The hue gap between `want` and what `shown` shows, when `want` has a
/// hue and `shown` isn't grey.
fn hue_gap(want: Rgb, shown: Rgb) -> Option<f32> {
    let (cw, hw) = xterm::chroma_hue(want);
    let (cs, hs) = xterm::chroma_hue(shown);
    if cw < CHROMATIC || cs < xterm::GREY {
        return None;
    }
    let d = (hw - hs).abs() % 360.0;
    Some(d.min(360.0 - d))
}

/// Every index `c` can show in 256 colours (flat match and every dither
/// threshold) that is off its hue.
fn off_hue(c: Rgb) -> Vec<(u8, f32)> {
    let mut shown = dither_counts(c);
    shown.insert(xterm::nearest(c), 0);
    shown
        .into_keys()
        .filter_map(|i| hue_gap(c, xterm::rgb(i)).map(|d| (i, d)))
        .filter(|&(_, d)| d > HUE_TOLERANCE)
        .collect()
}

#[test]
fn ansi256_dark_wax_never_turns_another_hue() {
    // The lava-ebq.32 repro: dim orange-brown wax (chrome) went
    // to 005f00 (22) and 5f5f00 (58), green dots on orange.
    for c in [
        Rgb(0x51, 0x30, 0x08),
        Rgb(0x6D, 0x3C, 0x08),
        Rgb(0x5D, 0x34, 0x1C),
        Rgb(0x59, 0x45, 0x0C),
    ] {
        let shown = dither_counts(c);
        assert!(
            !shown.contains_key(&22) && !shown.contains_key(&58),
            "{c:?}: {shown:?}"
        );
        assert_eq!(off_hue(c), [], "{c:?}");
    }
    // Every palette's wax ramp and roles, dimmed and brightened the way
    // styles scale them, and faded into the liquid and background.
    for palette in Palette::all().iter().filter(|p| p.has_rgb()) {
        // The hand-picked indices too (§5.2): an unmixed role shows its own.
        for role in Role::ALL {
            let s = palette.swatch(role);
            let (want, shown) = (s.rgb.unwrap(), xterm::rgb(s.x256.unwrap()));
            let gap = hue_gap(want, shown).unwrap_or(0.0);
            assert!(gap <= HUE_TOLERANCE, "{} {role:?}: {gap}°", palette.name);
        }
        let theme = Theme::new(palette, ColorDepth::Ansi256);
        let roles = Role::ALL.map(|r| theme.rgb(Ink::Role(r)));
        let bases = theme.wax.iter().chain(&roles);
        for &base in bases {
            for k in [0.7, 0.8, 0.9, 1.0, 1.1, 1.15, 1.2, 1.25, 1.35] {
                for (to, t) in [(Role::Liquid, 0.0), (Role::Liquid, 0.4), (Role::Bg, 0.7)] {
                    let c = base.scale(k).lerp(theme.rgb(Ink::Role(to)), t);
                    assert_eq!(off_hue(c), [], "{} {c:?}", palette.name);
                }
            }
        }
    }
}

#[test]
fn ansi256_no_colour_shows_another_hue() {
    // The whole RGB cube, coarsely: no tinted colour is ever matched or
    // dithered to an index of another hue.
    let levels = (0..=255u8).step_by(9).chain([255]);
    let levels: Vec<u8> = levels.collect();
    for &r in &levels {
        for &g in &levels {
            for &b in &levels {
                let c = Rgb(r, g, b);
                assert_eq!(off_hue(c), [], "{c:?}");
            }
        }
    }
}

/// Whole lamps, every style × palette, from live sim worlds: every
/// pixel 256 colours shows keeps the hue of the truecolor pixel it stands
/// for.
#[test]
fn ansi256_lamp_pixels_keep_their_truecolor_hue() {
    use ratatui::buffer::{Buffer, Cell};
    use ratatui::layout::Rect;
    use ratatui::widgets::StatefulWidget;

    use crate::render::{LampOptions, LampState, LampView, StyleId};
    use crate::sim::{Field, Shape, World};

    /// A cell's (top, bottom) pixel colours, or, for a glyph, its
    /// (fg, fg) and (bg, bg).
    fn pixels(cell: &Cell) -> [(Color, Color); 2] {
        match cell.symbol() {
            "▀" => [(cell.fg, cell.bg); 2],
            "▄" => [(cell.bg, cell.fg); 2],
            "█" => [(cell.fg, cell.fg); 2],
            " " => [(cell.bg, cell.bg); 2],
            _ => [(cell.fg, cell.fg), (cell.bg, cell.bg)],
        }
    }
    let rgb = |c: Color| match c {
        Color::Rgb(r, g, b) => Some(Rgb(r, g, b)),
        Color::Indexed(i) => Some(xterm::rgb(i)),
        _ => None,
    };

    let lamps = [
        (2, Shape::Tank, Rect::new(0, 0, 96, 30)),
        (7, Shape::Bottle, Rect::new(0, 0, 30, 20)),
    ];
    let mut state = LampState::default();
    let mut bad = Vec::new();
    for (seed, shape, area) in lamps {
        let aspect = f64::from(area.width) / f64::from(area.height) / 2.0;
        let mut world = World::new(seed, aspect, shape);
        world.prewarm(600, 1.0 / 120.0);
        let mut field = Field::default();
        field.prepare(&world, 0.0);
        for palette in Palette::all() {
            for id in StyleId::all() {
                let draw = |depth, state: &mut LampState| {
                    let theme = Theme::new(palette, depth);
                    let mut buf = Buffer::empty(area);
                    LampView {
                        field: &field,
                        style: id.style(),
                        theme: &theme,
                        time: 1.0,
                        options: LampOptions::default(),
                    }
                    .render(area, &mut buf, state);
                    buf
                };
                let truecolor = draw(ColorDepth::TrueColor, &mut state);
                let ansi256 = draw(ColorDepth::Ansi256, &mut state);
                for (pos, (want, shown)) in area
                    .positions()
                    .zip(truecolor.content().iter().zip(ansi256.content()))
                {
                    for (w, s) in pixels(want).into_iter().zip(pixels(shown)) {
                        for (w, s) in [(w.0, s.0), (w.1, s.1)] {
                            let (Some(w), Some(s)) = (rgb(w), rgb(s)) else {
                                continue;
                            };
                            if let Some(d) = hue_gap(w, s).filter(|&d| d > HUE_TOLERANCE) {
                                bad.push(format!(
                                    "{} {} {pos:?}: {w:?} shown as {s:?} ({d:.0}°)",
                                    palette.name,
                                    id.style().name()
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} off-hue pixels:\n{}",
        bad.len(),
        bad[..bad.len().min(20)].join("\n")
    );
}
