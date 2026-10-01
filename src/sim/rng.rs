//! Tiny seeded PRNG (SplitMix64). The sim's only source of randomness, so a
//! seed fully determines a run.

#[derive(Debug, Clone)]
pub(super) struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN);
        finalise(self.0)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

/// The 64-bit golden ratio: SplitMix64's step.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// A stateless SplitMix64 draw: the first value of `Rng::new(x)`, for
/// randomness fixed by an id with no state to keep.
pub(super) fn hash(x: u64) -> u64 {
    finalise(x.wrapping_add(GOLDEN))
}

/// SplitMix64's output mix.
fn finalise(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_in_range() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..1000 {
            let x = a.unit();
            assert_eq!(x, b.unit());
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
        assert_eq!(hash(9), Rng::new(9).next_u64());
    }
}
