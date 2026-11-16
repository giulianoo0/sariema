// SPDX-License-Identifier: Apache-2.0

use super::{ActorPack, Channel, Clip, FaceAtlas, Property, Values};
use crate::Aabb;

pub(super) fn actor(a: &ActorPack) -> Result<(), String> {
    if a.joints.is_empty() {
        return Err("it has no joints".into());
    }
    let joints = a.joints.len();
    for (i, joint) in a.joints.iter().enumerate() {
        if joint.parent.is_some_and(|p| p as usize >= i) {
            return Err(format!(
                "joint {i} ({}) has parent {:?}, which does not come before it",
                joint.name, joint.parent
            ));
        }
        let rest = &joint.rest;
        let numbers = rest
            .translation
            .iter()
            .chain(&rest.rotation)
            .chain(&rest.scale);
        if !numbers.chain(&joint.inverse_bind).all(|x| x.is_finite()) {
            return Err(format!("joint {i} ({}) is not finite", joint.name));
        }
    }
    for (i, mesh) in a.meshes.iter().enumerate() {
        let vertices = mesh.vertices.len();
        if !mesh.indices.len().is_multiple_of(3) {
            return Err(format!(
                "mesh {i} has {} indices, not whole triangles",
                mesh.indices.len()
            ));
        }
        if let Some(&index) = mesh.indices.iter().find(|&&ix| ix as usize >= vertices) {
            return Err(format!(
                "mesh {i} uses vertex {index} but has {vertices} vertices"
            ));
        }
        for (k, v) in mesh.vertices.iter().enumerate() {
            let sum: u32 = v.weights.iter().map(|&w| u32::from(w)).sum();
            if sum != u32::from(u16::MAX) {
                return Err(format!(
                    "mesh {i} vertex {k} has weights summing to {sum}, not 65535"
                ));
            }
            let used = v.joints.iter().zip(v.weights).filter(|&(_, w)| w > 0);
            if let Some((&joint, _)) = used.into_iter().find(|&(&j, _)| j as usize >= joints) {
                return Err(format!(
                    "mesh {i} vertex {k} uses joint {joint} of {joints}"
                ));
            }
        }
    }
    let textures = a.textures.len();
    for (i, m) in a.materials.iter().enumerate() {
        let used = [m.base_color_texture, m.emissive_texture];
        if let Some(t) = used.into_iter().flatten().find(|&t| t as usize >= textures) {
            return Err(format!("material {i} uses texture {t} of {textures}"));
        }
    }
    for (i, p) in a.parts.iter().enumerate() {
        if p.mesh as usize >= a.meshes.len() || p.material as usize >= a.materials.len() {
            return Err(format!(
                "part {i} ({}) uses mesh {} of {} and material {} of {}",
                p.name,
                p.mesh,
                a.meshes.len(),
                p.material,
                a.materials.len()
            ));
        }
    }
    for (i, s) in a.sockets.iter().enumerate() {
        if s.joint as usize >= joints || !s.offset.iter().all(|x| x.is_finite()) {
            return Err(format!(
                "socket {i} ({}) has joint {} of {joints} or a non-finite offset",
                s.name, s.joint
            ));
        }
    }
    for (i, face) in a.faces.iter().enumerate() {
        face_atlas(a, face).map_err(|why| format!("face {i} ({}) {why}", face.name))?;
    }
    for (i, clip) in a.clips.iter().enumerate() {
        self::clip(a, clip).map_err(|why| format!("clip {i} ({}) {why}", clip.name))?;
    }
    bounds(&a.bounds).map_err(|why| format!("its bounds {why}"))
}

fn face_atlas(a: &ActorPack, face: &FaceAtlas) -> Result<(), String> {
    let cells = face.columns as u64 * face.rows as u64;
    if cells == 0 {
        return Err(format!("has a {}×{} grid", face.columns, face.rows));
    }
    if let Some(&m) = face
        .materials
        .iter()
        .find(|&&m| m as usize >= a.materials.len())
    {
        return Err(format!("uses material {m} of {}", a.materials.len()));
    }
    if !face.origin.iter().chain(&face.step).all(|x| x.is_finite()) {
        return Err("has a non-finite origin or step".into());
    }
    let cell_ok = |c: u32| u64::from(c) < cells;
    if !cell_ok(face.start) || !face.frames.iter().all(|f| cell_ok(f.cell)) {
        return Err(format!("names a cell past its {cells}"));
    }
    Ok(())
}

fn clip(a: &ActorPack, clip: &Clip) -> Result<(), String> {
    let duration = clip.duration;
    if !(duration.is_finite() && duration >= 0.0) {
        return Err(format!("lasts {duration} s"));
    }
    let in_clip = |t: f32| t.is_finite() && (0.0..=duration).contains(&t);
    let ascending = |times: &[f32]| times.iter().all(|&t| in_clip(t)) && times.is_sorted();
    let mut seen = Vec::with_capacity(clip.channels.len());
    for (i, ch) in clip.channels.iter().enumerate() {
        channel(a, ch, &ascending).map_err(|why| format!("channel {i} {why}"))?;
        if seen.contains(&(ch.joint, ch.property)) {
            return Err(format!(
                "has two {:?} channels for joint {}",
                ch.property, ch.joint
            ));
        }
        seen.push((ch.joint, ch.property));
    }
    if let Some(m) = clip.markers.iter().find(|m| !in_clip(m.time)) {
        return Err(format!("has marker {} at {} s", m.name, m.time));
    }
    for (i, track) in clip.faces.iter().enumerate() {
        let Some(face) = a.faces.get(track.face as usize) else {
            return Err(format!(
                "face track {i} uses face {} of {}",
                track.face,
                a.faces.len()
            ));
        };
        let cells = face.columns as u64 * face.rows as u64;
        if track.times.len() != track.cells.len()
            || track.times.is_empty()
            || !ascending(&track.times)
            || track.cells.iter().any(|&c| u64::from(c) >= cells)
        {
            return Err(format!("face track {i} has bad times or cells"));
        }
    }
    if clip
        .ground_speed
        .is_some_and(|s| !(s.is_finite() && s >= 0.0))
    {
        return Err(format!("has ground speed {:?}", clip.ground_speed));
    }
    bounds(&clip.bounds).map_err(|why| format!("has bounds that {why}"))
}

fn channel(a: &ActorPack, ch: &Channel, ascending: &dyn Fn(&[f32]) -> bool) -> Result<(), String> {
    if ch.joint as usize >= a.joints.len() {
        return Err(format!("targets joint {} of {}", ch.joint, a.joints.len()));
    }
    if ch.times.is_empty() || ch.times.len() != ch.values.len() {
        return Err(format!(
            "has {} times and {} values",
            ch.times.len(),
            ch.values.len()
        ));
    }
    if !ascending(&ch.times) {
        return Err("has times that are not ascending within the clip".into());
    }
    let finite = match (&ch.values, ch.property) {
        (Values::Vec3(v), Property::Translation | Property::Scale) => {
            v.iter().flatten().all(|x| x.is_finite())
        }
        (Values::Quat(v), Property::Rotation) => v.iter().flatten().all(|x| x.is_finite()),
        (Values::QuatSnorm16(_), Property::Rotation) => true,
        _ => return Err(format!("has values that do not fit {:?}", ch.property)),
    };
    if !finite {
        return Err("has values that are not finite".into());
    }
    Ok(())
}

fn bounds(b: &Aabb) -> Result<(), String> {
    let Aabb { min, max } = b;
    if (0..3).all(|k| min[k].is_finite() && max[k].is_finite() && min[k] <= max[k]) {
        Ok(())
    } else {
        Err(format!("{min:?}..{max:?} are not finite or not ordered"))
    }
}
