//! The style registry. To add a style: write `styles/<name>.rs` with a unit
//! struct implementing [`Style`](super::Style), declare the module here and
//! add it to [`ALL`]. Order is the cycle order; the first is the default.
//!
//! Small helpers shared by more than one style live here too.

mod ascii;
mod braille;
mod crt;
mod dither;
mod glass;
mod halftone;
mod heatmap;
mod matrix;
mod outline;
mod solid;
mod synthwave;
mod topo;

use super::{Canvas, Style};
use crate::sim::SURFACE;

pub static ALL: &[&dyn Style] = &[
    &solid::Solid,
    &outline::Outline,
    &heatmap::Heatmap,
    &ascii::Ascii,
    &dither::Dither,
    &braille::Braille,
    &halftone::Halftone,
    &crt::Crt,
    &synthwave::Synthwave,
    &matrix::Matrix,
    &topo::Topo,
    &glass::Glass,
];

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

/// A well-mixed 32-bit hash, for stable per-column / per-cell randomness.
#[inline]
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}
