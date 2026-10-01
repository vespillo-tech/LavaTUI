//! Wax simulation: blobs, temperature field, buoyancy, metaball field
//! sampling. Pure and deterministic — no terminal code.
//!
//! Stub: only tracks simulated time so the app loop has something to step.
//! The real engine (lava-lno) replaces [`World`]'s internals.

#[derive(Debug, Default)]
pub struct World {
    /// Total simulated seconds.
    pub time: f64,
}

impl World {
    /// Advance the simulation by exactly `dt` seconds (always the fixed step).
    pub fn step(&mut self, dt: f64) {
        self.time += dt;
    }
}
