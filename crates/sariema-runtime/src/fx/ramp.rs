// SPDX-License-Identifier: Apache-2.0

use crate::tween::steps_of;

/// A value eased linearly from where it is to a target over whole steps,
/// so a new target never makes it jump.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Ramp {
    from: f32,
    to: f32,
    steps: u32,
    elapsed: u32,
}

impl Ramp {
    pub fn to(&mut self, to: f32, seconds: f32) {
        self.from = self.at(1.0);
        self.to = to;
        self.steps = steps_of(seconds);
        self.elapsed = 0;
    }

    pub fn step(&mut self) {
        self.elapsed = (self.elapsed + 1).min(self.steps);
    }

    /// Its value `alpha` of the way from the last step to this one.
    pub fn at(&self, alpha: f32) -> f32 {
        if self.elapsed >= self.steps {
            return self.to;
        }
        let t = ((self.elapsed as f32 - 1.0 + alpha) / self.steps as f32).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * t
    }

    /// At its target, and the target is 0.
    pub fn out(&self) -> bool {
        self.elapsed >= self.steps && self.to == 0.0
    }
}
