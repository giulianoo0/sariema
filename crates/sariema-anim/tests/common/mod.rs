// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use glam::{Mat4, Quat, Vec3};
use sariema_anim::{AnimSettings, Animator, ClipId, Rig, Transform};
use sariema_format::actor::demo::demo_arm;

pub const STEP: f32 = 1.0 / 60.0;

pub fn arm() -> Rig {
    Rig::new(&demo_arm()).unwrap()
}

pub fn clip(rig: &Rig, name: &str) -> ClipId {
    rig.clip(name).unwrap()
}

pub fn animator(rig: &Rig) -> Animator {
    Animator::new(rig, AnimSettings::default())
}

pub fn sample(rig: &Rig, name: &str, time: f32) -> Vec<Transform> {
    let mut pose = vec![Transform::IDENTITY; rig.joints()];
    rig.sample(clip(rig, name), time, &mut pose);
    pose
}

/// Degrees `q` turns about the unit `axis`, in -180..180.
pub fn angle_about(q: Quat, axis: Vec3) -> f32 {
    let (a, angle) = q.to_axis_angle();
    let signed = if a.dot(axis) < 0.0 { -angle } else { angle };
    let deg = signed.to_degrees();
    (deg + 180.0).rem_euclid(360.0) - 180.0
}

/// Model-space positions of every joint in the last evaluated pose.
pub fn joint_positions(rig: &Rig, animator: &Animator) -> Vec<Vec3> {
    let mut model = vec![Mat4::IDENTITY; rig.joints()];
    rig.model_space(animator.pose(), &mut model);
    model.iter().map(|m| m.w_axis.truncate()).collect()
}

/// The arm's skinned tip, 1.2 m up in the bind pose, after the palette.
pub fn tip(animator: &Animator) -> Vec3 {
    animator.palette()[2].transform_point3(Vec3::new(0.0, 1.2, 0.0))
}

pub fn assert_close(a: Vec3, b: Vec3, tolerance: f32) {
    assert!(
        a.distance(b) <= tolerance,
        "{a} is not within {tolerance} of {b}"
    );
}
