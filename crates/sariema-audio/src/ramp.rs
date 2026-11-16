// SPDX-License-Identifier: Apache-2.0

//! Gains that move over time, evaluated at an absolute frame so the result
//! does not depend on how the frames are split into buffers.

use std::f32::consts::FRAC_PI_2;

/// Shape of a fade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Linear,
    /// A rising fade follows sin(x·π/2) and a falling one cos(x·π/2), so a
    /// crossfade of equal length keeps in² + out² = 1.
    EqualPower,
    /// Smoothstep, 3x² − 2x³.
    SCurve,
}

impl Curve {
    fn rise(self, x: f32) -> f32 {
        match self {
            Curve::Linear => x,
            Curve::EqualPower => (x * FRAC_PI_2).sin(),
            Curve::SCurve => x * x * (3.0 - 2.0 * x),
        }
    }

    fn fall(self, x: f32) -> f32 {
        match self {
            Curve::EqualPower => 1.0 - (x * FRAC_PI_2).cos(),
            other => other.rise(x),
        }
    }
}

/// A value moving from `from` to `to` over `len` frames from frame `start`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    from: f32,
    to: f32,
    start: u64,
    len: u32,
    curve: Curve,
}

impl Ramp {
    pub fn constant(value: f32) -> Ramp {
        Ramp {
            from: value,
            to: value,
            start: 0,
            len: 0,
            curve: Curve::Linear,
        }
    }

    pub fn value(&self, t: u64) -> f32 {
        if t >= self.end() {
            return self.to;
        }
        if t <= self.start {
            return self.from;
        }
        let x = (t - self.start) as f32 / self.len as f32;
        let shape = if self.to >= self.from {
            self.curve.rise(x)
        } else {
            self.curve.fall(x)
        };
        self.from + (self.to - self.from) * shape
    }

    /// Moves from wherever the value is at `t` to `to`, so an interrupted
    /// fade stays continuous.
    pub fn retarget(&mut self, t: u64, to: f32, len: u32, curve: Curve) {
        *self = Ramp {
            from: self.value(t),
            to,
            start: t,
            len,
            curve,
        };
    }

    pub fn end(&self) -> u64 {
        self.start + u64::from(self.len)
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    /// Whether the value is the same over `[t, t + n)`.
    pub fn is_flat(&self, t: u64, n: u32) -> bool {
        t >= self.end() || (self.from == self.to && t + u64::from(n) <= self.start)
    }
}

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// −∞ for a gain of 0.
pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_reaches_its_target_on_the_last_frame() {
        let mut r = Ramp::constant(0.0);
        r.retarget(100, 1.0, 48, Curve::EqualPower);
        assert_eq!(r.value(100), 0.0);
        assert!(r.value(147) < 1.0);
        assert_eq!(r.value(148), 1.0);
    }

    #[test]
    fn equal_power_crossfade_keeps_power() {
        let (mut a, mut b) = (Ramp::constant(1.0), Ramp::constant(0.0));
        a.retarget(0, 0.0, 1000, Curve::EqualPower);
        b.retarget(0, 1.0, 1000, Curve::EqualPower);
        for t in 0..=1000 {
            let p = a.value(t).powi(2) + b.value(t).powi(2);
            assert!((p - 1.0).abs() < 1e-5, "{t}: {p}");
        }
    }

    #[test]
    fn retarget_mid_fade_is_continuous() {
        let mut r = Ramp::constant(0.0);
        r.retarget(0, 1.0, 100, Curve::SCurve);
        let before = r.value(40);
        r.retarget(40, 0.0, 10, Curve::SCurve);
        assert_eq!(r.value(40), before);
    }

    #[test]
    fn decibels() {
        assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-5);
        assert_eq!(gain_to_db(0.0), f32::NEG_INFINITY);
    }
}
