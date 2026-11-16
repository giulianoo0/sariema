// SPDX-License-Identifier: Apache-2.0

//! The engine's seeded random numbers: PCG32 (O'Neill, pcg-random.org,
//! `pcg32_random_r`), the generator Godot's `RandomNumberGenerator` uses.
//! Shared verbatim with the no_std `sariema-sdk`, so it uses only `core`.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcg32 {
    state: u64,
    increment: u64,
}

const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

impl Pcg32 {
    /// `stream` selects one of 2⁶³ independent sequences.
    pub const fn new(seed: u64, stream: u64) -> Pcg32 {
        let mut rng = Pcg32 {
            state: 0,
            increment: (stream << 1) | 1,
        };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    pub const fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(MULTIPLIER).wrapping_add(self.increment);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        xorshifted.rotate_right((old >> 59) as u32)
    }

    /// Uniform in [0, 1), from the top 24 bits.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    pub fn state(&self) -> (u64, u64) {
        (self.state, self.increment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_reference_sequence() {
        // pcg32-demo from the reference implementation, seed 42, sequence 54.
        let mut rng = Pcg32::new(42, 54);
        let first: Vec<u32> = (0..6).map(|_| rng.next_u32()).collect();
        assert_eq!(
            first,
            [
                0xa15c02b7, 0x7b47f409, 0xba1d3330, 0x83d2f293, 0xbfa4784b, 0xcbed606e
            ]
        );
    }

    #[test]
    fn floats_stay_below_one() {
        let mut rng = Pcg32::new(1, 1);
        assert!(
            (0..10_000)
                .map(|_| rng.next_f32())
                .all(|f| (0.0..1.0).contains(&f))
        );
    }
}
