// SPDX-License-Identifier: Apache-2.0

//! What is measured on the finished actor: bounds and ground speeds.

use glam::{Mat4, Vec3};
use sariema_anim::{JointId, Rig, Transform, ground_speed};
use sariema_format::Aabb;
use sariema_format::actor::{ActorPack, validate};

use super::{ActorSource, name_error};
use crate::ImportError;
use crate::gltf::invalid;

/// Fills the bounds of every clip and of the pack, and each clip's ground speed.
pub(super) fn measure(pack: &mut ActorPack, source: &ActorSource) -> Result<(), ImportError> {
    validate(pack).map_err(|why| invalid(format!("the actor would be invalid: {why}")))?;
    let rig = Rig::new(pack).map_err(invalid)?;
    let feet = feet(&rig, source)?;
    let skin: Vec<(Vec3, [u16; 4], [u16; 4])> = pack
        .meshes
        .iter()
        .flat_map(|m| &m.vertices)
        .map(|v| (Vec3::from(v.pos), v.joints, v.weights))
        .collect();
    let inverse_bind: Vec<Mat4> = pack
        .joints
        .iter()
        .map(|j| Mat4::from_cols_array(&j.inverse_bind))
        .collect();
    let mut pose = vec![Transform::IDENTITY; rig.joints()];
    let mut model = vec![Mat4::IDENTITY; rig.joints()];
    let mut palette = vec![Mat4::IDENTITY; rig.joints()];
    let mut around = |pose: &[Transform], (lo, hi): &mut (Vec3, Vec3)| {
        rig.model_space(pose, &mut model);
        for ((p, m), ib) in palette.iter_mut().zip(&model).zip(&inverse_bind) {
            *p = *m * *ib;
        }
        for (pos, joints, weights) in &skin {
            let moved = joints.iter().zip(weights).filter(|(_, w)| **w > 0).fold(
                Vec3::ZERO,
                |sum, (&j, &w)| {
                    sum + palette[j as usize].transform_point3(*pos) * (f32::from(w) / 65535.0)
                },
            );
            *lo = lo.min(moved);
            *hi = hi.max(moved);
        }
    };
    let empty = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    let mut all = empty;
    around(rig.rest_pose(), &mut all);
    let fps = source.keys.bounds_fps;
    for (i, clip) in pack.clips.iter_mut().enumerate() {
        let id = sariema_anim::ClipId(i as u32);
        let samples = ((clip.duration * fps).ceil() as usize).max(1);
        let mut bounds = empty;
        for s in 0..=samples {
            rig.sample(id, clip.duration * s as f32 / samples as f32, &mut pose);
            around(&pose, &mut bounds);
        }
        clip.bounds = aabb(bounds);
        all = (all.0.min(bounds.0), all.1.max(bounds.1));
        clip.ground_speed = ground_speed(&rig, id, &feet, source.keys.ground_speed_fps);
    }
    pack.bounds = aabb(all);
    Ok(())
}

fn feet(rig: &Rig, source: &ActorSource) -> Result<Vec<JointId>, ImportError> {
    let names: Vec<&str> = (0..rig.joints() as u32)
        .map(|j| rig.joint_name(JointId(j)))
        .collect();
    match &source.feet {
        Some(feet) => feet
            .iter()
            .map(|f| rig.joint(f).ok_or_else(|| name_error("joint", f, &names)))
            .collect(),
        None => Ok((0..rig.joints() as u32)
            .map(JointId)
            .filter(|&j| rig.joint_name(j).to_lowercase().contains("foot"))
            .collect()),
    }
}

/// An empty actor is a point at the origin.
fn aabb((lo, hi): (Vec3, Vec3)) -> Aabb {
    if lo.cmple(hi).all() {
        Aabb {
            min: lo.to_array(),
            max: hi.to_array(),
        }
    } else {
        Aabb {
            min: [0.0; 3],
            max: [0.0; 3],
        }
    }
}
