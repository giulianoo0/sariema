// SPDX-License-Identifier: Apache-2.0

//! A skeleton with its clips decoded for sampling.

use glam::{Affine3A, Mat4, Quat, Vec3};
use sariema_format::actor::{ActorPack, Interpolation, Property, Values};

use crate::pose::{Transform, model_space};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JointId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClipId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SocketId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceId(pub u32);

#[derive(Clone, Debug, PartialEq)]
pub struct ClipInfo {
    pub name: String,
    pub duration: f32,
    pub looping: bool,
    /// Metres per second at speed 1 at which the feet do not slide.
    pub ground_speed: Option<f32>,
    /// Model-space bounds over the whole clip.
    pub bounds: (Vec3, Vec3),
    /// Names and times, in the pack's order; [`crate::EventKind::Marker`] indexes it.
    pub markers: Vec<(String, f32)>,
}

enum Keys {
    Vec3(Box<[Vec3]>),
    Quat(Box<[Quat]>),
}

struct Track {
    joint: usize,
    property: Property,
    step: bool,
    times: Box<[f32]>,
    keys: Keys,
}

struct FaceTrack {
    face: usize,
    times: Box<[f32]>,
    cells: Box<[u32]>,
}

struct Clip {
    info: ClipInfo,
    tracks: Vec<Track>,
    faces: Vec<FaceTrack>,
    /// The pose at time 0, which additive layers subtract.
    reference: Box<[Transform]>,
}

struct Face {
    name: String,
    frames: Vec<(String, u32)>,
    start: u32,
}

/// What an [`crate::Animator`] needs of an actor pack, read once.
pub struct Rig {
    names: Vec<String>,
    parents: Vec<Option<u16>>,
    rest: Vec<Transform>,
    inverse_bind: Vec<Affine3A>,
    clips: Vec<Clip>,
    sockets: Vec<(String, usize, Affine3A)>,
    faces: Vec<Face>,
}

impl Rig {
    /// Fails, with the reason, on a pack that does not pass
    /// [`sariema_format::actor::validate`]; a decoded pack always does.
    pub fn new(pack: &ActorPack) -> Result<Rig, String> {
        sariema_format::actor::validate(pack)?;
        if pack.joints.len() > u16::MAX as usize + 1 {
            return Err(format!(
                "{} joints; vertices address 65536",
                pack.joints.len()
            ));
        }
        let mut rig = Rig {
            names: pack.joints.iter().map(|j| j.name.clone()).collect(),
            parents: pack
                .joints
                .iter()
                .map(|j| j.parent.map(|p| p as u16))
                .collect(),
            rest: pack.joints.iter().map(|j| j.rest.into()).collect(),
            inverse_bind: pack
                .joints
                .iter()
                .map(|j| Affine3A::from_mat4(Mat4::from_cols_array(&j.inverse_bind)))
                .collect(),
            clips: Vec::with_capacity(pack.clips.len()),
            sockets: pack
                .sockets
                .iter()
                .map(|s| {
                    let offset = Affine3A::from_mat4(Mat4::from_cols_array(&s.offset));
                    (s.name.clone(), s.joint as usize, offset)
                })
                .collect(),
            faces: pack
                .faces
                .iter()
                .map(|f| Face {
                    name: f.name.clone(),
                    frames: f
                        .frames
                        .iter()
                        .map(|fr| (fr.name.clone(), fr.cell))
                        .collect(),
                    start: f.start,
                })
                .collect(),
        };
        for clip in &pack.clips {
            let tracks = clip.channels.iter().map(track).collect();
            let faces = clip
                .faces
                .iter()
                .map(|t| FaceTrack {
                    face: t.face as usize,
                    times: t.times.clone().into(),
                    cells: t.cells.clone().into(),
                })
                .collect();
            let info = ClipInfo {
                name: clip.name.clone(),
                duration: clip.duration,
                looping: clip.looping,
                ground_speed: clip.ground_speed,
                bounds: (clip.bounds.min.into(), clip.bounds.max.into()),
                markers: clip
                    .markers
                    .iter()
                    .map(|m| (m.name.clone(), m.time))
                    .collect(),
            };
            let mut decoded = Clip {
                info,
                tracks,
                faces,
                reference: rig.rest.clone().into(),
            };
            let mut reference = rig.rest.clone();
            sample_tracks(&decoded.tracks, 0.0, &mut reference);
            decoded.reference = reference.into();
            rig.clips.push(decoded);
        }
        Ok(rig)
    }

    pub fn joints(&self) -> usize {
        self.names.len()
    }

    pub fn joint(&self, name: &str) -> Option<JointId> {
        let i = self.names.iter().position(|n| n == name)?;
        Some(JointId(i as u32))
    }

    pub fn joint_name(&self, joint: JointId) -> &str {
        &self.names[joint.0 as usize]
    }

    pub fn parent(&self, joint: JointId) -> Option<JointId> {
        self.parents[joint.0 as usize].map(|p| JointId(p.into()))
    }

    pub fn rest_pose(&self) -> &[Transform] {
        &self.rest
    }

    pub fn clips(&self) -> usize {
        self.clips.len()
    }

    pub fn clip(&self, name: &str) -> Option<ClipId> {
        let i = self.clips.iter().position(|c| c.info.name == name)?;
        Some(ClipId(i as u32))
    }

    pub fn clip_info(&self, clip: ClipId) -> &ClipInfo {
        &self.clips[clip.0 as usize].info
    }

    pub fn socket(&self, name: &str) -> Option<SocketId> {
        let i = self.sockets.iter().position(|s| s.0 == name)?;
        Some(SocketId(i as u32))
    }

    pub fn faces(&self) -> usize {
        self.faces.len()
    }

    pub fn face(&self, name: &str) -> Option<FaceId> {
        let i = self.faces.iter().position(|f| f.name == name)?;
        Some(FaceId(i as u32))
    }

    /// The cell of the frame called `name` of `face`.
    pub fn face_frame(&self, face: FaceId, name: &str) -> Option<u32> {
        let frames = &self.faces[face.0 as usize].frames;
        frames.iter().find(|f| f.0 == name).map(|f| f.1)
    }

    pub(crate) fn face_start(&self, face: usize) -> u32 {
        self.faces[face].start
    }

    /// The pose of `clip` at `time`, clamped to the clip, into `out`. Joints
    /// the clip does not animate keep their rest transform.
    pub fn sample(&self, clip: ClipId, time: f32, out: &mut [Transform]) {
        let clip = &self.clips[clip.0 as usize];
        out.copy_from_slice(&self.rest);
        sample_tracks(&clip.tracks, time.clamp(0.0, clip.info.duration), out);
    }

    pub(crate) fn reference(&self, clip: ClipId) -> &[Transform] {
        &self.clips[clip.0 as usize].reference
    }

    /// The cell `clip` shows on `face` at `time`, if it has a track for it.
    pub(crate) fn face_cell(&self, clip: ClipId, face: usize, time: f32) -> Option<u32> {
        let track = self.clips[clip.0 as usize]
            .faces
            .iter()
            .find(|t| t.face == face)?;
        let after = track.times.partition_point(|&k| k <= time);
        Some(track.cells[after.saturating_sub(1)])
    }

    /// Model-space matrices of the local pose `local`.
    pub fn model_space(&self, local: &[Transform], out: &mut [Mat4]) {
        for (j, (parent, pose)) in self.parents.iter().zip(local).enumerate() {
            out[j] = match parent {
                Some(p) => out[*p as usize] * pose.matrix(),
                None => pose.matrix(),
            };
        }
    }

    pub(crate) fn model_space_affine(&self, local: &[Transform], out: &mut [Affine3A]) {
        model_space(&self.parents, local, out);
    }

    /// Skinning matrices, bind pose to model space, from model-space joints.
    pub(crate) fn palette(&self, model: &[Affine3A], out: &mut [Mat4]) {
        for ((out, m), ib) in out.iter_mut().zip(model).zip(&self.inverse_bind) {
            *out = Mat4::from(*m * *ib);
        }
    }

    pub(crate) fn socket_in_model(&self, socket: SocketId, model: &[Affine3A]) -> Mat4 {
        let (_, joint, offset) = &self.sockets[socket.0 as usize];
        Mat4::from(model[*joint] * *offset)
    }
}

fn track(ch: &sariema_format::actor::Channel) -> Track {
    let keys = match &ch.values {
        Values::Vec3(v) => Keys::Vec3(v.iter().map(|&x| Vec3::from(x)).collect()),
        Values::Quat(v) => Keys::Quat(v.iter().map(|&q| Quat::from_array(q).normalize()).collect()),
        Values::QuatSnorm16(v) => Keys::Quat(
            v.iter()
                .map(|q| {
                    Quat::from_array(q.map(|c| (c as f32 / i16::MAX as f32).max(-1.0))).normalize()
                })
                .collect(),
        ),
    };
    Track {
        joint: ch.joint as usize,
        property: ch.property,
        step: ch.interpolation == Interpolation::Step,
        times: ch.times.clone().into(),
        keys,
    }
}

/// Overwrites what `tracks` animate in `out` with their values at `time`.
fn sample_tracks(tracks: &[Track], time: f32, out: &mut [Transform]) {
    for track in tracks {
        let after = track.times.partition_point(|&k| k <= time);
        let (a, b, f) = if after == 0 {
            (0, 0, 0.0)
        } else if after == track.times.len() || track.step {
            (after - 1, after - 1, 0.0)
        } else {
            let (t0, t1) = (track.times[after - 1], track.times[after]);
            (after - 1, after, (time - t0) / (t1 - t0))
        };
        let pose = &mut out[track.joint];
        match (&track.keys, track.property) {
            (Keys::Quat(q), _) => pose.rotation = q[a].slerp(q[b], f),
            (Keys::Vec3(v), Property::Translation) => pose.translation = v[a].lerp(v[b], f),
            (Keys::Vec3(v), _) => pose.scale = v[a].lerp(v[b], f),
        }
    }
}
