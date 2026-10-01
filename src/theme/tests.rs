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
    assert!(near(theme.color(Ink::Heat(0.0)), 0x23160C));
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
fn ansi16_steps_and_never_blends() {
    let theme = Theme::new(lava(), ColorDepth::Ansi16);
    assert!(!theme.blends());
    assert_eq!(theme.color(Ink::Wax(0.0)), Color::Red);
    assert_eq!(theme.color(Ink::Wax(0.5)), Color::LightRed);
    assert_eq!(theme.color(Ink::Wax(1.0)), Color::LightYellow);
    assert_eq!(theme.color(Ink::Heat(0.0)), Color::Reset);
    assert_eq!(theme.color(Ink::Heat(1.0)), Color::LightYellow);
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
            assert_eq!(theme.color(Ink::Heat(t)), Color::Reset);
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
fn lighting_shade_darkens_and_eases_brightening() {
    let dark = Rgb(40, 20, 10);
    assert_eq!(dark.shade(1.0), dark);
    assert_eq!(dark.shade(0.5), Rgb(20, 10, 5));
    // Dark colours brighten almost fully; near-white barely moves.
    let Rgb(r, ..) = dark.shade(2.0);
    assert!((72..=80).contains(&r), "{r}");
    let paper = Rgb(231, 222, 203);
    let Rgb(r, g, b) = paper.shade(1.5);
    assert!(r <= 247 && g <= 238 && b <= 220, "{:?}", (r, g, b));
    // Saturated wax keeps its hue: red stays the dominant channel.
    let Rgb(r, g, b) = Rgb(226, 71, 27).shade(1.3);
    assert!(r > g && g > b && g < 100, "{:?}", (r, g, b));
}
