// SPDX-License-Identifier: Apache-2.0

//! Easing curves (Robert Penner's equations, as in Godot's `Tween` and
//! easings.net): f(0) = 0, f(1) = 1.

use std::f32::consts::PI;

use sariema_format::script::Ease;

// Penner's default overshoot for the "back" curves: about 10 %.
const BACK: f32 = 1.701_58;

pub fn ease(curve: Ease, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let in_out = |f: fn(f32) -> f32| {
        if t < 0.5 {
            f(2.0 * t) / 2.0
        } else {
            1.0 - f(2.0 - 2.0 * t) / 2.0
        }
    };
    let out = |f: fn(f32) -> f32| 1.0 - f(1.0 - t);
    match curve {
        Ease::Linear => t,
        Ease::InQuad => quad(t),
        Ease::OutQuad => out(quad),
        Ease::InOutQuad => in_out(quad),
        Ease::InCubic => cubic(t),
        Ease::OutCubic => out(cubic),
        Ease::InOutCubic => in_out(cubic),
        Ease::InQuart => quart(t),
        Ease::OutQuart => out(quart),
        Ease::InOutQuart => in_out(quart),
        Ease::InSine => sine(t),
        Ease::OutSine => out(sine),
        Ease::InOutSine => in_out(sine),
        Ease::InExpo => expo(t),
        Ease::OutExpo => out(expo),
        Ease::InOutExpo => in_out(expo),
        Ease::InBack => back(t),
        Ease::OutBack => out(back),
        Ease::InOutBack => in_out(back),
        Ease::Smoothstep => t * t * (3.0 - 2.0 * t),
    }
}

fn quad(t: f32) -> f32 {
    t * t
}

fn cubic(t: f32) -> f32 {
    t * t * t
}

fn quart(t: f32) -> f32 {
    t * t * t * t
}

fn sine(t: f32) -> f32 {
    1.0 - (t * PI / 2.0).cos()
}

fn expo(t: f32) -> f32 {
    if t <= 0.0 {
        0.0
    } else {
        (2.0f32).powf(10.0 * t - 10.0)
    }
}

fn back(t: f32) -> f32 {
    (BACK + 1.0) * t * t * t - BACK * t * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_starts_at_zero_and_ends_at_one() {
        for v in Ease::SPEC.variants {
            let curve = Ease::from_u8(v.value).unwrap();
            assert!(ease(curve, 0.0).abs() < 1e-3, "{} at 0", v.name);
            assert_eq!(ease(curve, 1.0), 1.0, "{} at 1", v.name);
        }
    }

    #[test]
    fn in_out_curves_cross_the_middle_at_half() {
        for curve in [
            Ease::InOutQuad,
            Ease::InOutCubic,
            Ease::InOutSine,
            Ease::Smoothstep,
        ] {
            assert!((ease(curve, 0.5) - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn out_back_overshoots_and_returns() {
        assert!(ease(Ease::OutBack, 0.7) > 1.0);
    }
}
