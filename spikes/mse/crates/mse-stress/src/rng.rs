// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A seeded xorshift64* generator for the T12 fuzz scenario: the same seed gives the same
//! operation sequence on every run, so a hang or crash can be replayed from the seed printed in
//! the report. Not for anything but test data.

#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A seed of 0 is mapped to a fixed non-zero constant (xorshift has no zero state).
    pub fn new(seed: u64) -> Self {
        Rng {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    /// xorshift64* (Vigna): shifts 12, 25, 27 and the multiplier below.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `0..bound` (`bound` of 0 gives 0). The modulo bias is irrelevant for fuzz data.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }

    /// A value in `low..=high`.
    pub fn range(&mut self, low: u64, high: u64) -> u64 {
        if high <= low {
            low
        } else {
            low + self.below(high - low + 1)
        }
    }

    /// `true` with the given probability in percent.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// An index chosen with the given integer weights (all zero gives 0).
    pub fn weighted(&mut self, weights: &[u32]) -> usize {
        let total: u64 = weights.iter().map(|w| u64::from(*w)).sum();
        if total == 0 {
            return 0;
        }
        let mut roll = self.below(total);
        for (index, weight) in weights.iter().enumerate() {
            let weight = u64::from(*weight);
            if roll < weight {
                return index;
            }
            roll -= weight;
        }
        weights.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let first: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let second: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_eq!(first, second);
        let mut c = Rng::new(43);
        assert_ne!(first[0], c.next_u64());
    }

    #[test]
    fn a_zero_seed_does_not_stick_at_zero() {
        let mut rng = Rng::new(0);
        assert_ne!(rng.next_u64(), 0);
        assert_ne!(rng.next_u64(), 0);
    }

    #[test]
    fn the_known_first_value_for_seed_one() {
        // xorshift64* with state 1: x ^= x>>12 (1), x ^= x<<25 (1 | 1<<25), x ^= x>>27 (same), times the multiplier.
        let mut rng = Rng::new(1);
        let expected = (1u64 ^ (1u64 << 25)).wrapping_mul(0x2545_F491_4F6C_DD1D);
        assert_eq!(rng.next_u64(), expected);
    }

    #[test]
    fn bounds_are_respected() {
        let mut rng = Rng::new(7);
        for _ in 0..1000 {
            assert!(rng.below(10) < 10);
            let v = rng.range(5, 8);
            assert!((5..=8).contains(&v));
        }
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.range(9, 3), 9);
    }

    #[test]
    fn weighted_follows_the_weights() {
        let mut rng = Rng::new(99);
        let mut hits = [0u32; 3];
        for _ in 0..3000 {
            hits[rng.weighted(&[1, 0, 9])] += 1;
        }
        assert_eq!(hits[1], 0);
        assert!(hits[2] > hits[0] * 4, "{hits:?}");
        assert_eq!(rng.weighted(&[0, 0]), 0);
    }

    #[test]
    fn chance_extremes() {
        let mut rng = Rng::new(5);
        assert!((0..100).all(|_| !rng.chance(0)));
        assert!((0..100).all(|_| rng.chance(100)));
    }
}
