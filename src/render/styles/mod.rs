//! The style registry. To add a style: write `styles/<name>.rs` with a unit
//! struct implementing [`LampStyle`](super::LampStyle), declare the module here and
//! add it to [`ALL`]. Order is the cycle order; the first is the default.
//!
//! Small helpers shared by more than one style live here too.

mod ascii;
mod braille;
mod chrome;
mod crt;
mod dither;
mod halftone;
mod heatmap;
mod matrix;
mod outline;
mod solid;
mod synthwave;
mod topo;

use super::{Canvas, StyleEntry};
use crate::sim::SURFACE;

pub static ALL: &[StyleEntry] = &[
    StyleEntry::of::<solid::Solid>(),
    StyleEntry::of::<outline::Outline>(),
    StyleEntry::of::<heatmap::Heatmap>(),
    StyleEntry::of::<ascii::Ascii>(),
    StyleEntry::of::<dither::Dither>(),
    StyleEntry::of::<braille::Braille>(),
    StyleEntry::of::<halftone::Halftone>(),
    StyleEntry::of::<crt::Crt>(),
    StyleEntry::of::<synthwave::Synthwave>(),
    StyleEntry::of::<matrix::Matrix>(),
    StyleEntry::of::<topo::Topo>(),
    StyleEntry::of::<chrome::Chrome>(),
];

/// Old style names still accepted in config and on the command line, and
/// the style each now means.
pub static ALIASES: &[(&str, &str)] = &[("glass", "chrome")];

/// A wax pixel with liquid on at least one side. Off-canvas counts as wax,
/// so wax touching the walls or floor (the pool) isn't outlined there.
fn is_edge(c: &Canvas, x: usize, y: usize) -> bool {
    let wax = |x: usize, y: usize| x >= c.width || y >= c.height || c.at(x, y).density >= SURFACE;
    wax(x, y)
        && !(wax(x.wrapping_sub(1), y)
            && wax(x + 1, y)
            && wax(x, y.wrapping_sub(1))
            && wax(x, y + 1))
}

/// Round `v` to the nearest `1/steps`, so slowly drifting inputs change a
/// cell only now and then (bandwidth, §7).
#[inline]
fn quantise(v: f32, steps: f32) -> f32 {
    (v * steps).round() / steps
}

/// Wax colour steps for styles that draw one colour per cell, so a cell
/// whose heat drifts slowly changes colour only now and then.
const HEAT_STEPS: f32 = 16.0;

/// `heat` (0..1) snapped to one of [`HEAT_STEPS`] colours.
#[inline]
fn stepped_heat(heat: f32) -> f32 {
    quantise(heat, HEAT_STEPS)
}

/// A well-mixed 32-bit hash, for stable per-column / per-cell randomness.
#[inline]
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}
