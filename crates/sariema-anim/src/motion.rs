// SPDX-License-Identifier: Apache-2.0

//! What a clip's motion implies for the actor moving it.

use glam::{Mat4, Vec3};

use crate::pose::Transform;
use crate::rig::{ClipId, JointId, Rig};

/// The speed in metres per second at which the ground must pass under an
/// in-place clip played at speed 1 so the planted foot does not slide: the
/// mean horizontal speed of the lowest of `feet` over the samples where the
/// same foot stays lowest, taken `samples_per_s` times a second.
///
/// `None` without feet, for a clip of no duration, or when no foot stays
/// lowest across two samples.
pub fn ground_speed(rig: &Rig, clip: ClipId, feet: &[JointId], samples_per_s: f32) -> Option<f32> {
    let duration = rig.clip_info(clip).duration;
    if feet.is_empty() || duration <= 0.0 || samples_per_s.is_nan() || samples_per_s <= 0.0 {
        return None;
    }
    let samples = ((duration * samples_per_s).ceil() as usize).max(2);
    let mut pose = vec![Transform::IDENTITY; rig.joints()];
    let mut model = vec![Mat4::IDENTITY; rig.joints()];
    let mut lowest = |t: f32| {
        rig.sample(clip, t, &mut pose);
        rig.model_space(&pose, &mut model);
        feet.iter()
            .map(|f| (f.0, model[f.0 as usize].w_axis.truncate()))
            .min_by(|a, b| a.1.y.total_cmp(&b.1.y))
            .expect("feet is not empty")
    };
    let (mut distance, mut time) = (0.0, 0.0);
    let mut last = lowest(0.0);
    for i in 1..=samples {
        let t = duration * i as f32 / samples as f32;
        let now = lowest(t);
        if now.0 == last.0 {
            let moved = (now.1 - last.1) * Vec3::new(1.0, 0.0, 1.0);
            distance += moved.length();
            time += duration / samples as f32;
        }
        last = now;
    }
    (time > 0.0).then(|| distance / time)
}
