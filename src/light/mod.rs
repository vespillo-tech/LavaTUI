//! Optional lighting / glow pass over the sampled field.
//!
//! The seam is [`Lighting`]: `render::LampView` fills its sample grid, then
//! (when a lighting pass is given) asks it for a brightness factor per
//! sample, which styles read through `Canvas::light`. The real pass (base
//! glow, highlight streak, rim light) is bead lava-5ak.

use crate::sim::Sample;

/// Computes per-sample brightness from the sampled field.
pub trait Lighting {
    /// Fill `out` (row-major, `width × height`, the same layout as
    /// `samples`) with a brightness factor: 1.0 = unlit, > 1 brighter.
    /// `out` arrives filled with 1.0.
    fn shade(&self, samples: &[Sample], width: usize, height: usize, out: &mut [f32]);
}
