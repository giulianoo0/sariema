// SPDX-License-Identifier: Apache-2.0

//! The curves a traversal may time its distance with: polynomial, so the
//! same on every platform, from 0 to 1 and never going back.

use sariema_format::script::Ease;
use sariema_runtime::ease::ease;

const TIMING: [(Ease, f32); 11] = [
    (Ease::Linear, 0.5),
    (Ease::InQuad, 0.25),
    (Ease::OutQuad, 0.75),
    (Ease::InOutQuad, 0.5),
    (Ease::InCubic, 0.125),
    (Ease::OutCubic, 0.875),
    (Ease::InOutCubic, 0.5),
    (Ease::InQuart, 0.0625),
    (Ease::OutQuart, 0.9375),
    (Ease::InOutQuart, 0.5),
    (Ease::Smoothstep, 0.5),
];

#[test]
fn each_timing_curve_starts_at_zero_ends_at_one_and_crosses_its_midpoint() {
    for (curve, middle) in TIMING {
        assert_eq!(ease(curve, 0.0), 0.0, "{curve:?} at 0");
        assert_eq!(ease(curve, 1.0), 1.0, "{curve:?} at 1");
        assert_eq!(ease(curve, 0.5), middle, "{curve:?} at 0.5");
    }
}

#[test]
fn each_timing_curve_never_goes_back() {
    for (curve, _) in TIMING {
        let mut last = 0.0;
        for k in 0..=1000 {
            let now = ease(curve, k as f32 / 1000.0);
            assert!(now >= last, "{curve:?} goes back at {k}: {now} < {last}");
            last = now;
        }
    }
}

#[test]
fn only_the_polynomial_curves_that_never_go_back_time_a_traversal() {
    for v in Ease::SPEC.variants {
        let curve = Ease::from_u8(v.value).unwrap();
        let timing = TIMING.iter().any(|(c, _)| *c == curve);
        assert_eq!(curve.is_timing(), timing, "{}", v.name);
    }
}
