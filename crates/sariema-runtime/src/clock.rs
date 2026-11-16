// SPDX-License-Identifier: Apache-2.0

//! The fixed-step accumulator between display frames and simulation steps.

use std::time::Duration;

use sariema_core::STEP_HZ;

const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// What one display frame asks of the simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    pub steps: u32,
    /// Whole steps of time thrown away because the frame was over the cap.
    pub dropped: u32,
}

/// Turns frame times into a whole number of fixed steps. Time is kept in
/// nanoseconds times [`STEP_HZ`], so a step is exactly 10⁹ units and frame
/// times never accumulate rounding.
#[derive(Clone, Debug)]
pub struct FixedClock {
    max_steps_per_frame: u32,
    accumulated: u128,
}

impl FixedClock {
    pub fn new(max_steps_per_frame: u32) -> FixedClock {
        FixedClock {
            max_steps_per_frame,
            accumulated: 0,
        }
    }

    pub fn advance(&mut self, frame: Duration) -> Tick {
        self.accumulated += frame.as_nanos() * STEP_HZ as u128;
        let owed = self.accumulated / NANOS_PER_SECOND;
        self.accumulated %= NANOS_PER_SECOND;
        let steps = owed.min(self.max_steps_per_frame as u128) as u32;
        Tick {
            steps,
            dropped: (owed - steps as u128).min(u32::MAX as u128) as u32,
        }
    }

    /// How far rendering is between the last two steps, in [0, 1).
    pub fn alpha(&self) -> f32 {
        // A remainder within 60 ns of a step rounds to 1.0 in f32.
        ((self.accumulated as f64 / NANOS_PER_SECOND as f64) as f32).min(1.0 - f32::EPSILON)
    }
}
