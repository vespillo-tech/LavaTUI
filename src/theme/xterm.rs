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
//!
//! Neither ever shows a colour of another hue: a candidate whose hue is
//! more than [`HUE_SPREAD`] from the colour's own is never picked, single
//! or in a pair (greys always may be). Without that guard dark orange
//! found its match in olive, or in a red and green pair that averages to
//! brown but shows as green dots.
//!
//! Even the best single index can be far off: the cube has no dark tints,
//! so dark purples and reds band (grey, then one loud row). [`dither`]
//! picks a *pair* of indices and a mix level instead, which an 8×8 Bayer
//! dither anchored to the lamp (`render/dither256.rs`) spreads over
//! neighbouring pixels.

use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::{LazyLock, OnceLock};

use super::Rgb;

/// Channel levels of the colour cube (indices 16..=231).
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Distance weights: ΔL², ΔC² (chroma) and ΔH² (hue, the part of the a/b
/// difference that isn't chroma). Tuned on captures of every style and
/// palette against truecolor.
const W_L: f32 = 2.0;
const W_C: f32 = 1.0;
const W_H: f32 = 2.0;

/// Candidates with at least this much chroma (OKLab) have a hue to get
/// wrong; below it they read as grey and may stand in for any colour.
const NEUTRAL: f32 = 0.02;
/// The most a chromatic candidate's hue may differ from the colour's
/// (degrees). Wider lets dark browns go olive or red-on-green; much
/// narrower leaves mid tones only greys.
const HUE_SPREAD: f32 = 30.0;
/// Colours with less chroma than this have no hue to speak of (rounding
/// noise on a grey): only greys show for them.
const HUELESS: f32 = 0.005;

/// Bits kept per channel for the cache key.
const BITS: u32 = 6;
/// Matched indices per bucket; 0 (a system colour, never an answer) means
/// not computed yet.
static CACHE: [AtomicU8; 1 << (3 * BITS)] = [const { AtomicU8::new(0) }; 1 << (3 * BITS)];

pub fn nearest(c: Rgb) -> u8 {
    let (key, rep) = bucket(c);
    match CACHE[key].load(Ordering::Relaxed) {
        0 => {
            let i = search(rep);
            CACHE[key].store(i, Ordering::Relaxed);
            i
        }
        i => i,
    }
}

/// `c`'s cache key, and the bucket's representative colour: its top bits,
/// replicated down, so black and white stay exact.
fn bucket(c: Rgb) -> (usize, Rgb) {
    let shift = 8 - BITS;
    let key = (usize::from(c.0 >> shift) << (2 * BITS))
        | (usize::from(c.1 >> shift) << BITS)
        | usize::from(c.2 >> shift);
    let rep = |v: u8| {
        let q = v >> shift;
        (q << shift) | (q >> (BITS - shift))
    };
    (key, Rgb(rep(c.0), rep(c.1), rep(c.2)))
}

/// Mix levels a [`Pair`] is quantised to: one per threshold of the 8×8
/// ordered dither that resolves it.
const LEVELS: u32 = 64;
/// How many of the closest single matches [`pair`] tries as one end.
const NEAR_ENDS: usize = 6;
/// Pairs are only for dark (OKLab lightness below this), tinted (chroma
/// above this) colours whose single match keeps less than this share of
/// their chroma: the gap in the cube. Elsewhere a flat colour that's a
/// little off beats a pattern.
const DARK: f32 = 0.5;
const TINT: f32 = 0.025;
const HUE_KEPT: f32 = 0.6;
/// Cost of the pattern's visibility, per unit of squared distance between
/// the pair: higher keeps to pairs of nearer colours.
const PATTERN_COST: f32 = 0.02;
/// A pair has to beat the single match by this factor to be worth its
/// pattern (and the bytes when it moves).
const GAIN: f32 = 0.6;
/// Pairs closer than this (squared, weighted OKLab) look the same: no
/// pattern, the single match is shown.
const SAME: f32 = 0.02 * 0.02;

/// Two indices and how much of the second to mix in: `far` shows on
/// `level` of every [`LEVELS`] dither thresholds, `near` on the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pair {
    near: u8,
    far: u8,
    level: u8,
}

impl Pair {
    fn pack(self) -> u32 {
        u32::from(self.near) | u32::from(self.far) << 8 | u32::from(self.level) << 16
    }

    fn unpack(v: u32) -> Pair {
        Pair {
            near: v as u8,
            far: (v >> 8) as u8,
            level: (v >> 16) as u8,
        }
    }
}

/// Matched pairs per bucket, packed; 0 means not computed yet (`near` is
/// never a system colour, so a real pair is never 0).
static PAIRS: [AtomicU32; 1 << (3 * BITS)] = [const { AtomicU32::new(0) }; 1 << (3 * BITS)];

/// The index to show for `c` where the ordered dither's threshold is
/// `threshold` (0..1).
#[inline]
pub fn dither(c: Rgb, threshold: f32) -> u8 {
    let p = pair(c);
    if threshold * (LEVELS as f32) < f32::from(p.level) {
        p.far
    } else {
        p.near
    }
}

/// The dither pair for `c` (cached per bucket, like [`nearest`]).
fn pair(c: Rgb) -> Pair {
    let (key, rep) = bucket(c);
    match PAIRS[key].load(Ordering::Relaxed) {
        0 => {
            let p = search_pair(rep);
            PAIRS[key].store(p.pack(), Ordering::Relaxed);
            p
        }
        v => Pair::unpack(v),
    }
}

/// The best pair for `c`. One end is among the few closest single
/// matches; the other is any candidate. The mix level is where `c`
/// projects onto the pair in linear light (how the eye averages a
/// pattern), and the score is the mix's distance to `c` plus a cost for
/// the pattern's contrast.
fn search_pair(c: Rgb) -> Pair {
    let cands = candidates();
    let x = Lab::of(c);
    let hues = Hues::of(c);
    let xl = linear(c);
    // At most 240 candidates. Keep cache misses off the allocator; an
    // explicit index tie-break preserves the old stable sort's order.
    let mut storage = [(0.0, 0usize); 240];
    let mut len = 0;
    for (i, cand) in cands.iter().enumerate() {
        if hues.allow(cand.lab) {
            storage[len] = (distance(x, cand.lab), i);
            len += 1;
        }
    }
    let ranked = &mut storage[..len];
    let rank = |a: &(f32, usize), b: &(f32, usize)| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1));
    let &(single, best) = ranked.iter().min_by(|a, b| rank(a, b)).unwrap();
    let alone = Pair {
        near: 16 + best as u8,
        far: 16 + best as u8,
        level: 0,
    };
    // Only dark tints whose single match loses the hue: anywhere else the
    // single match is close enough, and a flat colour beats a pattern.
    let chroma = x.chroma();
    if x.l > DARK || chroma < TINT || cands[best].lab.chroma() > HUE_KEPT * chroma {
        return alone;
    }
    // Most misses need just one index; only sort when a pair is useful.
    ranked.sort_unstable_by(rank);
    let mut found = (single * GAIN, alone);
    for &(_, a) in ranked.iter().take(NEAR_ENDS) {
        let ca = &cands[a];
        for &(_, b) in ranked.iter() {
            let cb = &cands[b];
            let spread = distance(ca.lab, cb.lab);
            if spread < SAME {
                continue;
            }
            let d: [f32; 3] = std::array::from_fn(|k| cb.linear[k] - ca.linear[k]);
            let len2: f32 = d.iter().map(|v| v * v).sum();
            let proj: f32 = (0..3).map(|k| (xl[k] - ca.linear[k]) * d[k]).sum::<f32>() / len2;
            let level = (proj * LEVELS as f32).round();
            if !(1.0..LEVELS as f32).contains(&level) {
                continue;
            }
            let f = level / LEVELS as f32;
            let mix = Lab::of_linear(std::array::from_fn(|k| ca.linear[k] + d[k] * f));
            let score = distance(x, mix) + PATTERN_COST * spread;
            if score < found.0 {
                found = (
                    score,
                    Pair {
                        near: 16 + a as u8,
                        far: 16 + b as u8,
                        level: level as u8,
                    },
                );
            }
        }
    }
    found.1
}

/// The colours of one cache bucket, as far as hue goes: its eight
/// corners. Dark and dull buckets span a wide arc of hue, and every colour
/// in the bucket gets the same answer, so a candidate has to suit them all.
struct Hues([Lab; 8]);

impl Hues {
    /// The bucket `c` (a bucket's representative) stands for.
    fn of(c: Rgb) -> Hues {
        let shift = 8 - BITS;
        let span = |v: u8, hi: bool| {
            let lo = v >> shift << shift;
            if hi { lo | ((1 << shift) - 1) } else { lo }
        };
        Hues(std::array::from_fn(|k| {
            Lab::of(Rgb(
                span(c.0, k & 1 != 0),
                span(c.1, k & 2 != 0),
                span(c.2, k & 4 != 0),
            ))
        }))
    }

    /// Whether `y` may show for these colours: it's near grey, or its hue
    /// is within [`HUE_SPREAD`] of every one of theirs. A near-grey colour
    /// has no hue to keep, so only greys may show for it.
    fn allow(&self, y: Lab) -> bool {
        let cy = y.chroma();
        if cy < NEUTRAL {
            return true;
        }
        let cos = HUE_SPREAD.to_radians().cos();
        self.0.iter().all(|x| {
            // cos of the hue angle between them, against cos HUE_SPREAD.
            let cx = x.chroma();
            cx >= HUELESS && x.a * y.a + x.b * y.b >= cx * cy * cos
        })
    }
}

/// The weighted distance between two colours (squared).
fn distance(x: Lab, y: Lab) -> f32 {
    let dl = x.l - y.l;
    let dc = x.chroma() - y.chroma();
    let dab = (x.a - y.a).powi(2) + (x.b - y.b).powi(2);
    let dh = (dab - dc * dc).max(0.0);
    W_L * dl * dl + W_C * dc * dc + W_H * dh
}

/// The best candidate for `c` by the weighted OKLab distance.
fn search(c: Rgb) -> u8 {
    let x = Lab::of(c);
    let hues = Hues::of(c);
    let mut best = (f32::INFINITY, 16);
    for (i, y) in candidates().iter().enumerate() {
        if !hues.allow(y.lab) {
            continue;
        }
        let d = distance(x, y.lab);
        if d < best.0 {
            best = (d, 16 + i as u8);
        }
    }
    best.1
}

/// Indices 16..=255, in OKLab and linear light.
fn candidates() -> &'static [Candidate; 240] {
    static CANDIDATES: OnceLock<[Candidate; 240]> = OnceLock::new();
    CANDIDATES.get_or_init(|| {
        std::array::from_fn(|i| {
            let c = rgb(16 + i as u8);
            Candidate {
                lab: Lab::of(c),
                linear: linear(c),
            }
        })
    })
}

struct Candidate {
    lab: Lab,
    linear: [f32; 3],
}

/// sRGB → linear light.
#[inline]
fn linear(c: Rgb) -> [f32; 3] {
    /// Per 8-bit value: [`distance`] runs in the lamp's cell loop.
    static LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| {
        std::array::from_fn(|v| {
            let v = v as f32 / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
    });
    let lin = |v: u8| LINEAR[usize::from(v)];
    [lin(c.0), lin(c.1), lin(c.2)]
}

/// Perceptual distance between two colours: their OKLab ΔE (about 0.02
/// is a just-noticeable difference).
#[cfg(test)]
pub fn delta_e(x: Rgb, y: Rgb) -> f32 {
    delta_e_squared(x, y).sqrt()
}

/// [`delta_e`] squared, for the lamp's cell loop: OKLab without the
/// chroma [`Lab`] keeps, and a cube root accurate to ~1e-6.
#[inline]
pub fn delta_e_squared(x: Rgb, y: Rgb) -> f32 {
    let (x, y) = (oklab(x), oklab(y));
    (x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)
}

#[inline]
fn oklab(c: Rgb) -> [f32; 3] {
    let [r, g, b] = linear(c);
    let l = cbrt(0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b);
    let m = cbrt(0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b);
    let s = cbrt(0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b);
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Cube root of `x` ≥ 0: a bit-level first guess, then two Newton steps.
#[inline]
fn cbrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut y = f32::from_bits(x.to_bits() / 3 + 0x2a51_4067);
    y = (2.0 * y + x / (y * y)) / 3.0;
    (2.0 * y + x / (y * y)) / 3.0
}

#[derive(Debug, Clone, Copy)]
struct Lab {
    l: f32,
    a: f32,
    b: f32,
    chroma: f32,
}

impl Lab {
    /// sRGB → OKLab (Björn Ottosson's matrices).
    #[inline]
    fn of(c: Rgb) -> Lab {
        Lab::of_linear(linear(c))
    }

    /// Linear-light RGB → OKLab.
    #[inline]
    fn of_linear([r, g, b]: [f32; 3]) -> Lab {
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
        let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
        Lab {
            l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            a,
            b,
            chroma: a.hypot(b),
        }
    }

    fn chroma(self) -> f32 {
        self.chroma
    }
}

#[test]
fn fast_cbrt_matches_std() {
    for i in 0..=100_000 {
        let x = i as f32 / 100_000.0;
        assert!(
            (cbrt(x) - x.cbrt()).abs() < 2e-6,
            "{x}: {} vs {}",
            cbrt(x),
            x.cbrt()
        );
    }
}

/// `c`'s OKLab chroma and hue (degrees), for tests that check hue.
#[cfg(test)]
pub fn chroma_hue(c: Rgb) -> (f32, f32) {
    let x = Lab::of(c);
    (x.chroma(), x.b.atan2(x.a).to_degrees())
}

/// Below this chroma a candidate counts as grey (see [`NEUTRAL`]).
#[cfg(test)]
pub const GREY: f32 = NEUTRAL;

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

#[cfg(test)]
mod tests;
