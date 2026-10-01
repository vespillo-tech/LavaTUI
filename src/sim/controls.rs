//! User-facing sim controls (docs/design.md §6.1, §6.3, §9): the API the TUI
//! binds to keys and the mouse. Every control changes the lamp gradually:
//!
//! - **Heat** 1..=5 ([`World::set_heat`]): more heat = more, smaller,
//!   faster blobs. The level the sim *uses* eases toward the chosen one over
//!   a few seconds, so buoyancy, blob count and budding rate never jump.
//! - **Reseed** ([`World::reseed`]): every blob melts into the pool in about
//!   two seconds, then the pool buds a fresh lamp from the new seed.
//! - **Heat pulse** ([`World::heat_pulse`]): warms the wax around a point
//!   (in [`super::Field`] coordinates) for about a second so it rises; on
//!   the pool it raises a bud there.
//! - **Speed** ([`SimSpeed`]): scales the real time fed to the fixed-step
//!   clock. The sim's `dt` never changes; only how many steps run per frame.

use std::ops::RangeInclusive;

use super::{Phase, World};

/// Heat levels, and the one a fresh lamp (and the `0` reset key) uses.
pub const HEAT_LEVELS: RangeInclusive<u8> = 1..=5;
pub const DEFAULT_HEAT: u8 = 3;

/// The heat the sim uses approaches the chosen level at this rate (1/s):
/// ~95 % of the way in 4 s.
const HEAT_EASE: f64 = 0.75;
/// At heat 5 (heat 1 is the mirror image) relative to heat 3: buoyancy is
/// this much stronger, the lamp aims for this many more blobs, and the pool
/// buds this much more often.
const HEAT_BUOYANCY: f64 = 0.35;
const HEAT_BLOBS: f64 = 0.4;
const HEAT_SPAWN: f64 = 0.4;
/// At heat 1 the pool may get this much deeper (relative to heat 3) before
/// it buds past the blob count; at heat 5, this much shallower.
const HEAT_DEEP_POOL: f64 = 0.5;

/// Reseed: blobs melt at this fraction of their area per second, so even
/// the largest is gone in under two seconds.
pub(super) const RESEED_MELT_RATE: f64 = 3.0;
/// After the melt, the pool buds quickly for this long to refill the lamp.
const REFILL_TIME: f64 = 5.0;
/// While refilling, budding gaps shrink and buds grow this much faster.
pub(super) const REFILL_SPAWN_SPEEDUP: f64 = 5.0;
pub(super) const REFILL_BUD_SPEEDUP: f64 = 2.0;

/// Heat pulse reach beyond a blob's own radius (lamp heights), heating rate
/// (1/s) at full strength, and decay time constant (s).
const PULSE_RADIUS: f64 = 0.12;
const PULSE_RATE: f64 = 2.5;
const PULSE_DECAY: f64 = 0.8;
/// Pulses fade out below this strength; at most this many are live (a mouse
/// drag adds one per event, the oldest go first).
const PULSE_MIN: f64 = 0.02;
const MAX_PULSES: usize = 16;
/// Pulses this close to the pool surface raise a bud.
const PULSE_POOL_BAND: f64 = 0.06;

/// Where a reseed is up to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Reseed {
    /// Old blobs melting into the pool; no budding.
    Melting,
    /// Pool budding the new lamp quickly, for `left` more seconds.
    Refill { left: f64 },
}

/// A local warm spot that fades.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Pulse {
    x: f64,
    y: f64,
    strength: f64,
}

/// Simulation speed: ×0.25 · ×0.5 · ×1 · ×2 · ×4. App-side: it scales the
/// elapsed time fed into `timing::FixedStep`, so the sim still advances in
/// fixed steps (and the per-frame step cap still applies).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimSpeed(usize);

impl SimSpeed {
    const FACTORS: [f64; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];
    const NORMAL: usize = 2;

    /// One notch faster (saturates at ×4).
    pub fn faster(self) -> Self {
        Self((self.0 + 1).min(Self::FACTORS.len() - 1))
    }

    /// One notch slower (saturates at ×0.25).
    pub fn slower(self) -> Self {
        Self(self.0.saturating_sub(1))
    }

    /// The notch nearest `factor` (config files store the multiplier).
    pub fn from_factor(factor: f64) -> Self {
        let distance = |f: f64| (f.ln() - factor.max(1e-3).ln()).abs();
        let nearest = (0..Self::FACTORS.len())
            .min_by(|&a, &b| distance(Self::FACTORS[a]).total_cmp(&distance(Self::FACTORS[b])))
            .unwrap_or(Self::NORMAL);
        Self(nearest)
    }

    /// The multiplier, for display (`speed ×2`).
    pub fn factor(self) -> f64 {
        Self::FACTORS[self.0]
    }
}

impl Default for SimSpeed {
    fn default() -> Self {
        Self(Self::NORMAL)
    }
}

impl World {
    /// The chosen heat level (what the UI shows). The sim eases toward it.
    pub fn heat(&self) -> u8 {
        self.heat_target
    }

    /// Choose a heat level, clamped to [`HEAT_LEVELS`].
    pub fn set_heat(&mut self, level: u8) {
        self.heat_target = level.clamp(*HEAT_LEVELS.start(), *HEAT_LEVELS.end());
    }

    /// Melt every blob into the pool, then bud a new lamp from `seed`. The
    /// melt takes under ~2 s and wax is conserved throughout. Calling it
    /// again mid-reseed restarts the melt with the newer seed.
    pub fn reseed(&mut self, seed: u64) {
        self.rng = super::Rng::new(seed);
        self.reseed = Some(Reseed::Melting);
        self.pulses.clear();
        for blob in &mut self.blobs {
            blob.phase = Phase::Melting;
        }
    }

    /// True from [`World::reseed`] until the new lamp has budded.
    #[cfg(test)]
    pub fn is_reseeding(&self) -> bool {
        self.reseed.is_some()
    }

    /// Warm the wax around (`u`, `v`): normalised viewport coordinates, `v`
    /// down, exactly as in [`super::Field::sample`]. Nearby blobs heat up
    /// over the next second and rise; a pulse on the pool raises a bud.
    pub fn heat_pulse(&mut self, u: f64, v: f64) {
        let (x, y) = ((u - 0.5) * self.view_width, 1.0 - v);
        if !(x.is_finite() && y.is_finite()) {
            return;
        }
        if self.pulses.len() == MAX_PULSES {
            self.pulses.remove(0);
        }
        self.pulses.push(Pulse {
            x,
            y: y.clamp(0.0, 1.0),
            strength: 1.0,
        });
        if y < self.pool_level() + PULSE_POOL_BAND && self.reseed != Some(Reseed::Melting) {
            self.bud_at(Some(x), false);
        }
    }

    // --- used by the step ---------------------------------------------------

    /// Ease heat, fade pulses, advance a reseed. Runs once per step.
    pub(super) fn update_controls(&mut self, dt: f64) {
        let target = f64::from(self.heat_target);
        self.heat_level += (target - self.heat_level) * (1.0 - (-HEAT_EASE * dt).exp());

        let fade = (-dt / PULSE_DECAY).exp();
        for pulse in &mut self.pulses {
            pulse.strength *= fade;
        }
        self.pulses.retain(|p| p.strength > PULSE_MIN);

        self.reseed = match self.reseed {
            Some(Reseed::Melting) if self.blobs.is_empty() => {
                self.spawn_timer = 0.0;
                Some(Reseed::Refill { left: REFILL_TIME })
            }
            Some(Reseed::Refill { left }) if left <= dt => None,
            Some(Reseed::Refill { left }) => Some(Reseed::Refill { left: left - dt }),
            other => other,
        };
    }

    /// Eased heat mapped to -1 (heat 1) … 0 (heat 3) … +1 (heat 5).
    fn heat_offset(&self) -> f64 {
        (self.heat_level - f64::from(DEFAULT_HEAT)) / 2.0
    }

    pub(super) fn heat_buoyancy(&self) -> f64 {
        1.0 + HEAT_BUOYANCY * self.heat_offset()
    }

    pub(super) fn heat_blobs(&self) -> f64 {
        1.0 + HEAT_BLOBS * self.heat_offset()
    }

    /// Multiplier on the pool depth that forces budding.
    pub(super) fn heat_deep_pool(&self) -> f64 {
        1.0 - HEAT_DEEP_POOL * self.heat_offset()
    }

    /// Multiplier on the gap between bud attempts.
    pub(super) fn heat_spawn_gap(&self) -> f64 {
        1.0 - HEAT_SPAWN * self.heat_offset()
    }

    /// Heating rate (1/s, toward fully hot) from pulses for a blob at
    /// (`x`, `y`) of `radius`; 0 when none reach it.
    pub(super) fn pulse_heat(&self, x: f64, y: f64, radius: f64) -> f64 {
        self.pulses
            .iter()
            .map(|p| {
                let reach = PULSE_RADIUS + radius;
                let d2 = ((p.x - x).powi(2) + (p.y - y).powi(2)) / (reach * reach);
                PULSE_RATE * p.strength * (1.0 - d2).max(0.0)
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_steps_and_saturates() {
        let mut speed = SimSpeed::default();
        assert_eq!(speed.factor(), 1.0);
        for expected in [2.0, 4.0, 4.0] {
            speed = speed.faster();
            assert_eq!(speed.factor(), expected);
        }
        assert_eq!(SimSpeed::from_factor(2.0).factor(), 2.0);
        assert_eq!(SimSpeed::from_factor(0.3).factor(), 0.25);
        assert_eq!(SimSpeed::from_factor(100.0).factor(), 4.0);
        assert_eq!(SimSpeed::from_factor(-1.0).factor(), 0.25);
        for expected in [2.0, 1.0, 0.5, 0.25, 0.25] {
            speed = speed.slower();
            assert_eq!(speed.factor(), expected);
        }
    }
}
