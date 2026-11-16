// SPDX-License-Identifier: Apache-2.0

//! Scalar helpers the simulation may use. `glam`'s `Vec3` and `Mat3` are plain
//! scalar code on every target; its SIMD types (`Vec3A`, `Vec4`, `Quat`, `Mat4`)
//! and its trigonometry are not used here, because their results can differ
//! between targets. Trigonometry goes through `libm`, which is the same Rust
//! code everywhere.

use std::f32::consts::{PI, TAU};

use glam::{Mat3, Vec3};

/// Rotation by `yaw` radians about +Y: yaw 0 faces +Z, yaw π faces −Z (north).
pub fn yaw_matrix(yaw: f32) -> Mat3 {
    let (s, c) = (libm::sinf(yaw), libm::cosf(yaw));
    Mat3::from_cols(Vec3::new(c, 0.0, -s), Vec3::Y, Vec3::new(s, 0.0, c))
}

/// The ground-plane direction an actor with this yaw faces.
pub fn forward(yaw: f32) -> Vec3 {
    Vec3::new(libm::sinf(yaw), 0.0, libm::cosf(yaw))
}

/// The yaw that faces the ground-plane direction `(x, z)`.
pub fn yaw_of(x: f32, z: f32) -> f32 {
    libm::atan2f(x, z)
}

/// `angle` brought into (−π, π].
pub fn wrap_angle(angle: f32) -> f32 {
    let mut a = angle - TAU * libm::floorf((angle + PI) / TAU);
    if a <= -PI {
        a += TAU;
    }
    a
}

/// The horizontal part of `v`.
pub fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    pub fn of_points(points: impl IntoIterator<Item = Vec3>) -> Bounds {
        points.into_iter().fold(Bounds::EMPTY, Bounds::with)
    }

    pub fn with(self, p: Vec3) -> Bounds {
        Bounds {
            min: self.min.min(p),
            max: self.max.max(p),
        }
    }

    pub fn union(self, other: Bounds) -> Bounds {
        Bounds {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn grown(self, by: f32) -> Bounds {
        Bounds {
            min: self.min - Vec3::splat(by),
            max: self.max + Vec3::splat(by),
        }
    }

    pub fn overlaps(&self, other: &Bounds) -> bool {
        self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn is_empty(&self) -> bool {
        self.min.cmpgt(self.max).any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_pi_faces_north() {
        let f = forward(PI);
        assert!((f - Vec3::NEG_Z).length() < 1e-6, "{f}");
        assert!((yaw_matrix(PI) * Vec3::Z - Vec3::NEG_Z).length() < 1e-6);
        assert!((yaw_of(0.0, -1.0) - PI).abs() < 1e-6);
    }

    #[test]
    fn wrap_angle_stays_in_half_open_range() {
        for a in [-10.0f32, -PI, -3.0, 0.0, 3.0, PI, 7.0, 100.0] {
            let w = wrap_angle(a);
            assert!(w > -PI - 1e-6 && w <= PI + 1e-6, "{a} -> {w}");
            assert!((libm::sinf(w) - libm::sinf(a)).abs() < 1e-4);
        }
    }
}
