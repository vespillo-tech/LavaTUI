//! The palettes of docs/design.md §5.2, role for role. Order is the cycle
//! order; the first is the default.

use ratatui::style::Color;

use super::{Palette, Rgb, Swatch};

/// `hex`, xterm-256 index, 16-colour fallback (`Color::Reset` = terminal
/// default).
const fn sw(hex: u32, x256: u8, ansi: Color) -> Swatch {
    Swatch {
        rgb: Some(Rgb::hex(hex)),
        x256: Some(x256),
        ansi,
    }
}

/// A role that only ever uses the terminal's own 16-colour theme.
const fn ansi(ansi: Color) -> Swatch {
    Swatch {
        rgb: None,
        x256: None,
        ansi,
    }
}

const D: Color = Color::Reset;

/// Role order: bg, liquid, wax_cool, wax_mid, wax_hot, metal, text, dim, accent.
pub(super) static PALETTES: [Palette; 8] = [
    Palette {
        name: "lava",
        swatches: [
            sw(0x0F0B0A, 232, D),
            sw(0x23160C, 233, D),
            sw(0x8E1B12, 88, Color::Red),
            sw(0xE2471B, 166, Color::LightRed),
            sw(0xFFB04A, 215, Color::LightYellow),
            sw(0x6B5A4E, 240, Color::DarkGray),
            sw(0xE9DCCF, 253, D),
            sw(0x7D6E62, 242, Color::DarkGray),
            sw(0xFF8A3D, 209, Color::Yellow),
        ],
    },
    Palette {
        name: "ultraviolet",
        swatches: [
            sw(0x0B0816, 233, D),
            sw(0x170F2C, 234, D),
            sw(0x4B1D8F, 54, Color::Blue),
            sw(0xB5179E, 127, Color::Magenta),
            sw(0xFF7AD9, 212, Color::LightMagenta),
            sw(0x4A4166, 239, Color::DarkGray),
            sw(0xE4DDF5, 254, D),
            sw(0x776E93, 96, Color::DarkGray),
            sw(0xA78BFA, 141, Color::LightBlue),
        ],
    },
    Palette {
        name: "abyss",
        swatches: [
            sw(0x060B10, 232, D),
            sw(0x0B1A24, 234, D),
            sw(0x0B4F6C, 23, Color::Blue),
            sw(0x1A9BA8, 31, Color::Cyan),
            sw(0xA8F5E4, 158, Color::LightCyan),
            sw(0x34495A, 238, Color::DarkGray),
            sw(0xD6E7EE, 254, D),
            sw(0x5F7785, 66, Color::DarkGray),
            sw(0x4FD1C5, 80, Color::LightCyan),
        ],
    },
    Palette {
        name: "toxic",
        swatches: [
            sw(0x080A06, 232, D),
            sw(0x121A0B, 233, D),
            sw(0x2F6B1F, 22, Color::Green),
            sw(0x7FBF2A, 106, Color::LightGreen),
            sw(0xE8FF6A, 191, Color::LightYellow),
            sw(0x3E4A33, 238, Color::DarkGray),
            sw(0xE2ECD5, 254, D),
            sw(0x6C7A5E, 65, Color::DarkGray),
            sw(0xC6FF3D, 154, Color::LightGreen),
        ],
    },
    Palette {
        name: "synthwave",
        swatches: [
            sw(0x0E0718, 233, D),
            sw(0x1B0B2B, 234, D),
            sw(0xFF2E88, 198, Color::Magenta),
            sw(0xFF7A59, 209, Color::LightRed),
            sw(0xFFD66B, 221, Color::LightYellow),
            sw(0x4B3264, 238, Color::DarkGray),
            sw(0xF4E6FF, 255, D),
            sw(0x8A73A3, 97, Color::DarkGray),
            sw(0x2DE2E6, 44, Color::LightCyan),
        ],
    },
    Palette {
        name: "mono",
        swatches: [
            sw(0x0C0C0C, 232, D),
            sw(0x161616, 233, D),
            sw(0x3D3D3D, 237, Color::DarkGray),
            sw(0x9A9A9A, 247, Color::Gray),
            sw(0xF0F0F0, 255, Color::White),
            sw(0x3A3A3A, 236, Color::DarkGray),
            sw(0xE0E0E0, 254, D),
            sw(0x6E6E6E, 242, Color::DarkGray),
            sw(0xFFFFFF, 231, Color::White),
        ],
    },
    Palette {
        name: "paper",
        swatches: [
            sw(0xF3EEE3, 255, D),
            sw(0xE7DECB, 253, D),
            sw(0x7A2617, 88, Color::Red),
            sw(0xC24D2C, 130, Color::LightRed),
            sw(0xF08A3C, 209, Color::Yellow),
            sw(0xA8997E, 138, Color::Gray),
            sw(0x3B342C, 236, D),
            sw(0x8C8173, 244, Color::Gray),
            sw(0x1F6F8B, 24, Color::Blue),
        ],
    },
    // The 16-colour column of lava in every depth; bg/liquid stay default
    // so terminal transparency shows through.
    Palette {
        name: "ansi",
        swatches: [
            ansi(D),
            ansi(D),
            ansi(Color::Red),
            ansi(Color::LightRed),
            ansi(Color::LightYellow),
            ansi(Color::DarkGray),
            ansi(D),
            ansi(Color::DarkGray),
            ansi(Color::Yellow),
        ],
    },
];
