// SPDX-License-Identifier: Apache-2.0

//! Local joint transforms, how they blend, and what they become in model space.

use glam::{Affine3A, Mat4, Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    pub(crate) fn affine(&self) -> Affine3A {
        Affine3A::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// `self` at `t = 0`, `to` at `t = 1`; rotations take the shortest arc.
    pub fn blend(&self, to: &Transform, t: f32) -> Transform {
        Transform {
            translation: self.translation.lerp(to.translation, t),
            rotation: nlerp(self.rotation, to.rotation, t),
            scale: self.scale.lerp(to.scale, t),
        }
    }
}

impl From<sariema_format::actor::Transform> for Transform {
    fn from(t: sariema_format::actor::Transform) -> Transform {
        Transform {
            translation: t.translation.into(),
            rotation: Quat::from_array(t.rotation).normalize(),
            scale: t.scale.into(),
        }
    }
}

/// Normalized linear interpolation along the shorter arc: slerp's path at a
/// non-uniform speed, which blending poses does not need, for less work.
pub(crate) fn nlerp(a: Quat, b: Quat, t: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    a.lerp(b, t).normalize()
}

/// What a pose adds to `reference`, applied by [`apply_delta`].
pub(crate) fn delta(pose: &Transform, reference: &Transform) -> Transform {
    Transform {
        translation: pose.translation - reference.translation,
        rotation: (pose.rotation * reference.rotation.inverse()).normalize(),
        scale: pose.scale / reference.scale,
    }
}

/// `base` with `weight` of `delta` added on top.
pub(crate) fn apply_delta(base: &Transform, delta: &Transform, weight: f32) -> Transform {
    Transform {
        translation: base.translation + delta.translation * weight,
        rotation: (nlerp(Quat::IDENTITY, delta.rotation, weight) * base.rotation).normalize(),
        scale: base.scale * Vec3::ONE.lerp(delta.scale, weight),
    }
}

/// Model-space matrices of `local`, whose parents come first.
pub(crate) fn model_space(parents: &[Option<u16>], local: &[Transform], out: &mut [Affine3A]) {
    for (j, (parent, pose)) in parents.iter().zip(local).enumerate() {
        out[j] = match parent {
            Some(p) => out[*p as usize] * pose.affine(),
            None => pose.affine(),
        };
    }
}
