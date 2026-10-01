//! The style registry. To add a style: write `styles/<name>.rs` with a unit
//! struct implementing [`Style`](super::Style), declare the module here and
//! add it to [`ALL`]. Order is the cycle order; the first is the default.

mod ascii;
mod dither;
mod heatmap;
mod outline;
mod solid;

use super::Style;

pub static ALL: &[&dyn Style] = &[
    &solid::Solid,
    &outline::Outline,
    &heatmap::Heatmap,
    &ascii::Ascii,
    &dither::Dither,
];
