//! Nearest xterm-256 colour, matched perceptually.
//!
//! Plain RGB distance picks badly from the 256 palette: the cube's darkest
//! non-black level is 95, so dark tints fall onto the grey ramp or jump to
//! loud cube colours, and orange edges snap to olive. Instead every
//! candidate (cube + grey ramp; the 16 system colours are themed by the
//! terminal, so never chosen) is compared in OKLab, with lightness and hue
//! weighted over chroma: a colour keeps its hue where the palette has one
//! close in lightness, and otherwise falls to the grey of the same
//! lightness rather than a brighter, louder cube colour. Near-neutral
//! inputs have no hue to lose, so they land on greys.
//!
//! The search runs over 240 candidates, so results are cached per 6-bit
//! RGB bucket (one fixed colour per bucket is matched, so the answer never
//! depends on which colour asked first).

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use super::Rgb;

/// Channel levels of the colour cube (indices 16..=231).
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Distance weights: ΔL², ΔC² (chroma) and ΔH² (hue, the part of the a/b
/// difference that isn't chroma). Tuned on captures of every style and
/// palette against truecolor.
const W_L: f32 = 2.0;
const W_C: f32 = 1.0;
const W_H: f32 = 2.0;

/// Bits kept per channel for the cache key.
const BITS: u32 = 6;
/// Matched indices per bucket; 0 (a system colour, never an answer) means
/// not computed yet.
static CACHE: [AtomicU8; 1 << (3 * BITS)] = [const { AtomicU8::new(0) }; 1 << (3 * BITS)];

pub fn nearest(c: Rgb) -> u8 {
    let shift = 8 - BITS;
    let key = (usize::from(c.0 >> shift) << (2 * BITS))
        | (usize::from(c.1 >> shift) << BITS)
        | usize::from(c.2 >> shift);
    match CACHE[key].load(Ordering::Relaxed) {
        0 => {
            // The bucket's representative: its top bits, replicated down,
            // so black and white stay exact.
            let rep = |v: u8| {
                let q = v >> shift;
                (q << shift) | (q >> (BITS - shift))
            };
            let i = search(Rgb(rep(c.0), rep(c.1), rep(c.2)));
            CACHE[key].store(i, Ordering::Relaxed);
            i
        }
        i => i,
    }
}

/// The best candidate for `c` by the weighted OKLab distance.
fn search(c: Rgb) -> u8 {
    let candidates = CANDIDATES.get_or_init(|| std::array::from_fn(|i| Lab::of(rgb(16 + i as u8))));
    let x = Lab::of(c);
    let chroma = x.chroma();
    let mut best = (f32::INFINITY, 16);
    for (i, y) in candidates.iter().enumerate() {
        let dl = x.l - y.l;
        let dc = chroma - y.chroma();
        let dab = (x.a - y.a).powi(2) + (x.b - y.b).powi(2);
        let dh = (dab - dc * dc).max(0.0);
        let d = W_L * dl * dl + W_C * dc * dc + W_H * dh;
        if d < best.0 {
            best = (d, 16 + i as u8);
        }
    }
    best.1
}

/// OKLab of indices 16..=255.
static CANDIDATES: OnceLock<[Lab; 240]> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct Lab {
    l: f32,
    a: f32,
    b: f32,
}

impl Lab {
    /// sRGB → OKLab (Björn Ottosson's matrices).
    fn of(c: Rgb) -> Lab {
        let lin = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (lin(c.0), lin(c.1), lin(c.2));
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        Lab {
            l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        }
    }

    fn chroma(self) -> f32 {
        self.a.hypot(self.b)
    }
}

/// The colour of xterm index `i` (the 16 system colours as xterm's
/// defaults; terminals theme those, so treat them as approximate).
pub fn rgb(i: u8) -> Rgb {
    const SYSTEM: [u32; 16] = [
        0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f,
        0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    match i {
        0..=15 => Rgb::hex(SYSTEM[usize::from(i)]),
        16..=231 => {
            let i = i - 16;
            Rgb(
                CUBE[usize::from(i / 36)],
                CUBE[usize::from(i / 6 % 6)],
                CUBE[usize::from(i % 6)],
            )
        }
        _ => {
            let v = 8 + 10 * (i - 232);
            Rgb(v, v, v)
        }
    }
}
