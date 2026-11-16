// SPDX-License-Identifier: Apache-2.0

//! A damped spring advanced by its exact solution over one fixed step, so it
//! is stable at any stiffness. Coefficients after Ryan Juckett, "Damped
//! Springs" (2012), for the offset from the target `x` and its velocity `v`:
//! `x' = pp·x + pv·v`, `v' = vp·x + vv·v`.

use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pp: f32,
    pv: f32,
    vp: f32,
    vv: f32,
}

impl Spring {
    /// `frequency` in Hz (undamped); `damping` 1 is critical, below
    /// oscillates, above creeps. A frequency of 0 or less never moves.
    pub fn new(frequency: f32, damping: f32, dt: f32) -> Spring {
        let omega = TAU * frequency.max(0.0);
        let zeta = damping.max(0.0);
        if omega <= 0.0 {
            return Spring {
                pp: 1.0,
                pv: dt,
                vp: 0.0,
                vv: 1.0,
            };
        }
        if zeta > 1.0 + 1e-4 {
            let za = -omega * zeta;
            let zb = omega * (zeta * zeta - 1.0).sqrt();
            let (z1, z2) = (za - zb, za + zb);
            let (e1, e2) = ((z1 * dt).exp(), (z2 * dt).exp());
            let inv = 1.0 / (2.0 * zb);
            let (e1i, e2i) = (e1 * inv, e2 * inv);
            let (z1e1i, z2e2i) = (z1 * e1i, z2 * e2i);
            Spring {
                pp: e1i * z2 - z2e2i + e2,
                pv: -e1i + e2i,
                vp: (z1e1i - z2e2i + e2) * z2,
                vv: -z1e1i + z2e2i,
            }
        } else if zeta < 1.0 - 1e-4 {
            let oz = omega * zeta;
            let alpha = omega * (1.0 - zeta * zeta).sqrt();
            let e = (-oz * dt).exp();
            let (sin, cos) = (alpha * dt).sin_cos();
            let (es, ec) = (e * sin, e * cos);
            let eozs = e * oz * sin / alpha;
            Spring {
                pp: ec + eozs,
                pv: es / alpha,
                vp: -es * alpha - oz * eozs,
                vv: ec - eozs,
            }
        } else {
            let e = (-omega * dt).exp();
            let te = dt * e;
            let tef = te * omega;
            Spring {
                pp: tef + e,
                pv: te,
                vp: -omega * tef,
                vv: -tef + e,
            }
        }
    }

    /// Advances `value` and `velocity` one step towards `target`.
    pub fn step(&self, value: f32, velocity: f32, target: f32) -> (f32, f32) {
        let x = value - target;
        (
            target + self.pp * x + self.pv * velocity,
            self.vp * x + self.vv * velocity,
        )
    }
}
