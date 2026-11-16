// SPDX-License-Identifier: Apache-2.0

//! Actor import from GLBs written by the tests: the demo arm's column skinned
//! to a skeleton listed out of order, with clips stored as gltfpack does.

mod common;

use std::path::Path;

use common::write_glb;
use glam::{Mat4, Quat, Vec3};
use sariema_anim::{Rig, Transform};
use sariema_format::actor::demo::demo_arm;
use sariema_format::actor::{ActorPack, Interpolation, Property, Values, decode, encode};
use sariema_import::{ActorSource, build_actor};
use serde_json::{Value, json};

/// Accessors and buffer views over one binary chunk.
#[derive(Default)]
struct Bin {
    bytes: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Bin {
    fn view(&mut self, data: &[u8]) -> usize {
        self.bytes.resize(self.bytes.len().next_multiple_of(4), 0);
        self.views
            .push(json!({"buffer": 0, "byteOffset": self.bytes.len(), "byteLength": data.len()}));
        self.bytes.extend(data);
        self.views.len() - 1
    }

    fn accessor(
        &mut self,
        view: usize,
        component: u32,
        count: usize,
        kind: &str,
        normalized: bool,
    ) -> usize {
        self.accessors.push(json!({
            "bufferView": view, "componentType": component, "count": count, "type": kind, "normalized": normalized
        }));
        self.accessors.len() - 1
    }

    fn floats(&mut self, data: &[f32], kind: &str, per: usize) -> usize {
        let view = self.view(bytemuck::cast_slice(data));
        self.accessor(view, 5126, data.len() / per, kind, false)
    }

    fn u16s(&mut self, data: &[u16], kind: &str, per: usize, normalized: bool) -> usize {
        let view = self.view(bytemuck::cast_slice(data));
        self.accessor(view, 5123, data.len() / per, kind, normalized)
    }

    /// Rotations as normalized shorts in an `EXT_meshopt_compression` view,
    /// as gltfpack stores them.
    fn packed_rotations(&mut self, quats: &[Quat]) -> usize {
        let shorts: Vec<[i16; 4]> = quats
            .iter()
            .map(|q| q.to_array().map(|c| (c * 32767.0).round() as i16))
            .collect();
        let encoded = meshopt::encode_vertex_buffer(&shorts).unwrap();
        self.bytes.resize(self.bytes.len().next_multiple_of(4), 0);
        let offset = self.bytes.len();
        self.bytes.extend(&encoded);
        self.views.push(json!({
            "buffer": 1, "byteLength": shorts.len() * 8, "byteStride": 8,
            "extensions": {"EXT_meshopt_compression": {
                "buffer": 0, "byteOffset": offset, "byteLength": encoded.len(),
                "byteStride": 8, "mode": "ATTRIBUTES", "count": shorts.len()
            }}
        }));
        let view = self.views.len() - 1;
        self.accessor(view, 5122, quats.len(), "VEC4", true)
    }
}

fn degrees(axis: Vec3, deg: &[f32]) -> Vec<Quat> {
    deg.iter()
        .map(|d| Quat::from_axis_angle(axis, d.to_radians()))
        .collect()
}

/// The skin lists joints as elbow, shoulder, wrist: not parents first.
const SKIN_ORDER: [usize; 3] = [1, 0, 2];

fn arm_glb(name: &str) -> std::path::PathBuf {
    scaled_arm_glb(name, 1.0)
}

/// The same arm under a rig node scaled by `s`, as gltfpack writes a model
/// whose joints it shortened: the joints' world bind pose is unchanged.
fn scaled_arm_glb(name: &str, s: f32) -> std::path::PathBuf {
    let arm = demo_arm();
    let mut bin = Bin::default();
    let column = &arm.meshes[0];
    let flat = |f: fn(&sariema_format::actor::SkinnedVertex) -> Vec<f32>| -> Vec<f32> {
        column.vertices.iter().flat_map(f).collect()
    };
    let position = bin.floats(&flat(|v| v.pos.to_vec()), "VEC3", 3);
    let normal = bin.floats(&flat(|v| v.normal.to_vec()), "VEC3", 3);
    let skin_index = |j: u16| SKIN_ORDER.iter().position(|&s| s == j as usize).unwrap() as u16;
    let joints: Vec<u16> = column
        .vertices
        .iter()
        .flat_map(|v| v.joints.map(skin_index))
        .collect();
    let joints = bin.u16s(&joints, "VEC4", 4, false);
    let weights: Vec<u16> = column.vertices.iter().flat_map(|v| v.weights).collect();
    let weights = bin.u16s(&weights, "VEC4", 4, true);
    let indices: Vec<u32> = column.indices.clone();
    let index_view = bin.view(bytemuck::cast_slice(&indices));
    let indices = bin.accessor(index_view, 5125, column.indices.len(), "SCALAR", false);
    let hand = &arm.meshes[1];
    let hand_pos: Vec<f32> = hand
        .vertices
        .iter()
        .flat_map(|v| [v.pos[0], v.pos[1] - 1.3, v.pos[2]])
        .collect();
    let hand_position = bin.floats(&hand_pos, "VEC3", 3);
    let hand_view = bin.view(bytemuck::cast_slice(&hand.indices));
    let hand_indices = bin.accessor(hand_view, 5125, hand.indices.len(), "SCALAR", false);
    let ibm: Vec<f32> = SKIN_ORDER
        .iter()
        .flat_map(|&j| {
            (Mat4::from_scale(Vec3::splat(1.0 / s))
                * Mat4::from_cols_array(&arm.joints[j].inverse_bind))
            .to_cols_array()
        })
        .collect();
    let ibm = bin.floats(&ibm, "MAT4", 16);

    let bend_times = bin.floats(&[0.0, 0.25, 0.5, 0.75, 1.0], "SCALAR", 1);
    let bend = bin.packed_rotations(&degrees(Vec3::Z, &[0.0, 45.0, 90.0, 45.0, 0.0]));
    let still_times = bin.floats(&[0.0, 1.0], "SCALAR", 1);
    let still = bin.floats(&[0.0; 6], "VEC3", 3);
    let wave_times = bin.floats(&[0.0, 0.4, 0.8], "SCALAR", 1);
    let wave_rot: Vec<f32> = degrees(Vec3::X, &[0.0, 60.0, 0.0])
        .iter()
        .flat_map(|q| q.to_array())
        .collect();
    let wave_rot = bin.floats(&wave_rot, "VEC4", 4);
    let wave_scale = bin.floats(&[1.0, 1.0, 1.0, 1.5, 1.5, 1.5, 1.0, 1.0, 1.0], "VEC3", 3);
    let turn_times = bin.floats(&[0.0, 1.0], "SCALAR", 1);
    let zero = [0.0f32; 4];
    let turn_keys: Vec<f32> = [zero, Quat::IDENTITY.to_array(), zero, zero]
        .into_iter()
        .chain([Quat::from_rotation_y(90f32.to_radians()).to_array(), zero])
        .flatten()
        .collect();
    let turn = bin.floats(&turn_keys, "VEC4", 4);

    let doc = json!({
        "asset": {"version": "2.0"},
        "extensionsUsed": ["EXT_meshopt_compression", "KHR_mesh_quantization"],
        "extensionsRequired": ["EXT_meshopt_compression", "KHR_mesh_quantization"],
        "buffers": [{"byteLength": bin.bytes.len()}, {"byteLength": 1024, "extensions": {"EXT_meshopt_compression": {"fallback": true}}}],
        "bufferViews": bin.views,
        "accessors": bin.accessors,
        "materials": [{"name": "mat_arm"}, {"name": "mat_hand"}],
        "meshes": [
            {"primitives": [{"attributes": {"POSITION": position, "NORMAL": normal, "JOINTS_0": joints, "WEIGHTS_0": weights}, "indices": indices, "material": 0}]},
            {"primitives": [{"attributes": {"POSITION": hand_position}, "indices": hand_indices, "material": 1}]}
        ],
        "skins": [{"joints": [2, 1, 3], "inverseBindMatrices": ibm}],
        "nodes": [
            {"name": "rig", "scale": [s, s, s], "children": [1, 4, 5]},
            {"name": "shoulder", "children": [2]},
            {"name": "elbow", "translation": [0.0, 0.5 / s, 0.0], "children": [3]},
            {"name": "wrist", "translation": [0.0, 0.5 / s, 0.0], "children": [6, 7]},
            {"name": "arm__mat_arm", "mesh": 0, "skin": 0, "translation": [5.0, 5.0, 5.0], "scale": [2.0, 2.0, 2.0]},
            {"name": "sleeve__mat_arm", "mesh": 0, "skin": 0, "extras": {"visible": false}},
            {"name": "tip", "translation": [0.0, 0.2, 0.0]},
            {"name": "hand__mat_hand", "mesh": 1, "translation": [0.0, 0.3, 0.0]}
        ],
        "scenes": [{"nodes": [0]}],
        "animations": [
            {"name": "bend", "extras": {"loop": true},
             "samplers": [{"input": bend_times, "output": bend}, {"input": still_times, "output": still}],
             "channels": [{"sampler": 0, "target": {"node": 2, "path": "rotation"}},
                          {"sampler": 1, "target": {"node": 1, "path": "translation"}}]},
            {"name": "wave",
             "samplers": [{"input": wave_times, "output": wave_rot}, {"input": wave_times, "output": wave_scale},
                          {"input": still_times, "output": still}],
             "channels": [{"sampler": 0, "target": {"node": 3, "path": "rotation"}},
                          {"sampler": 1, "target": {"node": 3, "path": "scale"}},
                          {"sampler": 2, "target": {"node": 0, "path": "translation"}}]},
            {"name": "turn",
             "samplers": [{"input": turn_times, "output": turn, "interpolation": "CUBICSPLINE"}],
             "channels": [{"sampler": 0, "target": {"node": 1, "path": "rotation"}}]}
        ]
    });
    write_glb(name, &doc.to_string(), &bin.bytes)
}

fn source(model: &Path, extra: Value) -> ActorSource {
    let mut description =
        json!({"name": "arm", "model": model.file_name().unwrap().to_str().unwrap()});
    description
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    ActorSource::from_json(&description.to_string()).unwrap()
}

fn build(model: &Path, extra: Value) -> ActorPack {
    build_actor(&source(model, extra), model.parent().unwrap()).unwrap()
}

fn error_code(model: &Path, extra: Value) -> &'static str {
    build_actor(&source(model, extra), model.parent().unwrap())
        .unwrap_err()
        .code()
}

#[test]
fn imports_joints_parents_first_with_their_bind_pose() {
    let pack = build(&arm_glb("joints.glb"), json!({}));
    let names: Vec<&str> = pack.joints.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(names, ["shoulder", "elbow", "wrist"]);
    let parents: Vec<Option<u32>> = pack.joints.iter().map(|j| j.parent).collect();
    assert_eq!(parents, [None, Some(0), Some(1)]);
    assert_eq!(pack.joints, demo_arm().joints);
}

#[test]
fn imports_clips_with_their_durations_and_loops() {
    let pack = build(&arm_glb("clips.glb"), json!({}));
    let clips: Vec<(&str, f32, bool)> = pack
        .clips
        .iter()
        .map(|c| (c.name.as_str(), c.duration, c.looping))
        .collect();
    assert_eq!(
        clips,
        [
            ("bend", 1.0, true),
            ("wave", 0.8, false),
            ("turn", 1.0, false)
        ]
    );
    let bend = &pack.clips[0];
    assert_eq!(
        bend.channels.len(),
        1,
        "the still shoulder translation is dropped"
    );
    let elbow = &bend.channels[0];
    assert_eq!(
        (elbow.joint, elbow.property, elbow.interpolation),
        (1, Property::Rotation, Interpolation::Linear)
    );
    assert_eq!(
        elbow.times,
        [0.0, 0.5, 1.0],
        "45° keys halfway along a slerp are redundant"
    );
    assert!(matches!(elbow.values, Values::QuatSnorm16(_)));
    assert_eq!(
        pack.clips[1].channels.len(),
        2,
        "the channel on a node that is no joint is skipped"
    );
}

#[test]
fn imported_clips_animate_like_the_source() {
    let pack = build(&arm_glb("animate.glb"), json!({}));
    let rig = Rig::new(&pack).unwrap();
    let mut pose = vec![Transform::IDENTITY; rig.joints()];
    rig.sample(rig.clip("bend").unwrap(), 0.25, &mut pose);
    let elbow = Quat::from_rotation_z(45f32.to_radians());
    assert!(pose[1].rotation.angle_between(elbow).to_degrees() < 0.01);
    rig.sample(rig.clip("wave").unwrap(), 0.4, &mut pose);
    assert!((pose[2].scale - Vec3::splat(1.5)).abs().max_element() < 1e-6);
    rig.sample(rig.clip("turn").unwrap(), 0.5, &mut pose);
    let half = Quat::from_rotation_y(45f32.to_radians());
    assert!(
        pose[0].rotation.angle_between(half).to_degrees() < 0.05,
        "{}",
        pose[0].rotation
    );
}

/// Skinned positions of the column at `time` into `clip`.
fn skinned_column(pack: &ActorPack, clip: &str, time: f32) -> Vec<Vec3> {
    let rig = Rig::new(pack).unwrap();
    let mut pose = vec![Transform::IDENTITY; rig.joints()];
    let mut model = vec![Mat4::IDENTITY; rig.joints()];
    rig.sample(rig.clip(clip).unwrap(), time, &mut pose);
    rig.model_space(&pose, &mut model);
    pack.meshes[0]
        .vertices
        .iter()
        .map(|v| {
            v.joints
                .iter()
                .zip(v.weights)
                .map(|(&j, w)| {
                    let joint = &pack.joints[j as usize];
                    let palette = model[j as usize] * Mat4::from_cols_array(&joint.inverse_bind);
                    palette.transform_point3(Vec3::from(v.pos)) * (f32::from(w) / 65535.0)
                })
                .sum()
        })
        .collect()
}

#[test]
fn a_scaled_node_above_the_skeleton_folds_into_the_root_joint() {
    let plain = build(&arm_glb("unscaled.glb"), json!({}));
    let scaled = build(&scaled_arm_glb("scaled.glb", 1.0614), json!({}));
    assert!((scaled.joints[0].rest.scale[1] - 1.0614).abs() < 1e-6);
    for (clip, time) in [("bend", 0.0), ("bend", 0.5), ("wave", 0.4)] {
        let worst = skinned_column(&plain, clip, time)
            .iter()
            .zip(skinned_column(&scaled, clip, time))
            .map(|(a, b)| a.distance(b))
            .fold(0.0, f32::max);
        assert!(worst < 1e-4, "{clip} at {time}: {worst} m apart");
    }
}

#[test]
fn skinned_vertices_ignore_their_node_transform() {
    let pack = build(&arm_glb("vertices.glb"), json!({}));
    type Influences = Vec<(u16, u16)>;
    let skinning = |m: &sariema_format::actor::SkinnedMesh| -> Vec<([f32; 3], Influences)> {
        let used = |v: &sariema_format::actor::SkinnedVertex| {
            v.joints
                .into_iter()
                .zip(v.weights)
                .filter(|w| w.1 > 0)
                .collect()
        };
        m.vertices.iter().map(|v| (v.pos, used(v))).collect()
    };
    let same = skinning(&pack.meshes[0]) == skinning(&demo_arm().meshes[0]);
    assert!(
        same,
        "positions, joints and weights differ from the demo arm's"
    );
    assert!(
        pack.bounds.min[1] > -0.1 && pack.bounds.max[1] < 1.7,
        "{:?}",
        pack.bounds
    );
}

#[test]
fn parts_keep_their_names_visibility_and_joints() {
    let pack = build(&arm_glb("parts.glb"), json!({}));
    let parts: Vec<(&str, bool)> = pack
        .parts
        .iter()
        .map(|p| (p.name.as_str(), p.visible))
        .collect();
    assert_eq!(parts, [("arm", true), ("sleeve", false), ("hand", true)]);
    let hand = &pack.meshes[pack.parts[2].mesh as usize];
    assert!(
        hand.vertices
            .iter()
            .all(|v| v.joints[0] == 2 && v.weights[0] == u16::MAX)
    );
    let top = hand
        .vertices
        .iter()
        .map(|v| v.pos[1])
        .fold(f32::MIN, f32::max);
    assert!(
        (top - 1.4).abs() < 1e-5,
        "the hand sits 0.3 m above the wrist, 1 m up: {top}"
    );
    let hidden = build(&arm_glb("hidden.glb"), json!({"hidden": ["hand"]}));
    assert!(!hidden.parts[2].visible);
}

#[test]
fn empty_nodes_under_joints_and_declared_ones_are_sockets() {
    let pack = build(
        &arm_glb("sockets.glb"),
        json!({"sockets": {"grip": {"joint": "elbow", "translation": [0.0, 0.1, 0.0]}}}),
    );
    let sockets: Vec<(&str, u32)> = pack
        .sockets
        .iter()
        .map(|s| (s.name.as_str(), s.joint))
        .collect();
    assert_eq!(sockets, [("tip", 2), ("grip", 1)]);
    for (socket, y) in pack.sockets.iter().zip([0.2, 0.1]) {
        let at = Mat4::from_cols_array(&socket.offset).w_axis.truncate();
        assert!(
            at.distance(Vec3::new(0.0, y, 0.0)) < 1e-6,
            "{} at {at}",
            socket.name
        );
    }
}

#[test]
fn the_description_adds_markers_faces_loops_and_looks() {
    let pack = build(
        &arm_glb("described.glb"),
        json!({
            "loop": ["w*"],
            "markers": {"wave": {"tap": 0.25}},
            "faces": {"look": {"materials": ["mat_hand"], "grid": [2, 2], "frames": ["a", "b", "c"], "first_cell": 1, "start": "b"}},
            "clip_faces": {"wave": {"look": [[0.0, "a"], [0.4, "c"]]}},
            "materials": {"mat_arm": {"rim": 0.25, "drawn": true}}
        }),
    );
    let looping: Vec<bool> = pack.clips.iter().map(|c| c.looping).collect();
    assert_eq!(looping, [false, true, false]);
    assert_eq!(pack.clips[1].markers[0].name, "tap");
    let face = &pack.faces[0];
    assert_eq!(
        (face.materials.as_slice(), face.start, face.step),
        (&[1][..], 2, [0.5, 0.5])
    );
    assert_eq!(pack.clips[1].faces[0].cells, [1, 3]);
    assert_eq!(
        (pack.materials[0].rim, pack.materials[0].drawn),
        (0.25, true)
    );
}

#[test]
fn every_clip_has_bounds_around_its_motion() {
    let pack = build(&arm_glb("bounds.glb"), json!({}));
    let bend = &pack.clips[0].bounds;
    // The hand's far corner, 0.9 m up and 0.1 m aside from the elbow, reaches
    // furthest along −X, sqrt(0.82) m, at atan(9) ≈ 83.7°.
    assert!((bend.min[0] + 0.82f32.sqrt()).abs() < 0.003, "{bend:?}");
    let wave = &pack.clips[1].bounds;
    assert!(wave.max[2] > 0.25, "the wrist waves towards +Z: {wave:?}");
    for clip in &pack.clips {
        for k in 0..3 {
            assert!(
                pack.bounds.min[k] <= clip.bounds.min[k]
                    && pack.bounds.max[k] >= clip.bounds.max[k]
            );
        }
    }
}

#[test]
fn an_imported_pack_round_trips() {
    let pack = build(&arm_glb("roundtrip.glb"), json!({}));
    assert_eq!(decode(&encode(&pack)).unwrap(), pack);
}

#[test]
fn clips_from_other_files_join_by_joint_name() {
    let model = arm_glb("model.glb");
    let mut bin = Bin::default();
    let times = bin.floats(&[0.0, 2.0], "SCALAR", 1);
    let rot: Vec<f32> = degrees(Vec3::Z, &[0.0, 30.0])
        .iter()
        .flat_map(|q| q.to_array())
        .collect();
    let rot = bin.floats(&rot, "VEC4", 4);
    let doc = json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": bin.bytes.len()}],
        "bufferViews": bin.views, "accessors": bin.accessors,
        "nodes": [{"name": "rig", "children": [1]}, {"name": "elbow"}],
        "scenes": [{"nodes": [0]}],
        "animations": [{"name": "lean", "samplers": [{"input": times, "output": rot}],
                        "channels": [{"sampler": 0, "target": {"node": 1, "path": "rotation"}}]}]
    });
    write_glb("more_clips.glb", &doc.to_string(), &bin.bytes);
    let pack = build(&model, json!({"clips_from": ["more_clips.glb"]}));
    let lean = pack.clips.iter().find(|c| c.name == "lean").unwrap();
    assert_eq!((lean.duration, lean.channels[0].joint), (2.0, 1));
    assert_eq!(
        error_code(&model, json!({"clips_from": ["model.glb"]})),
        "E_SCHEMA"
    );
}

#[test]
fn a_model_without_a_skin_is_a_rigid_prop() {
    let mut bin = Bin::default();
    let position = bin.floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], "VEC3", 3);
    let doc = json!({
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": bin.bytes.len()}],
        "bufferViews": bin.views, "accessors": bin.accessors,
        "meshes": [{"primitives": [{"attributes": {"POSITION": position}}]}],
        "nodes": [{"name": "cup", "children": [1, 2]}, {"name": "cup_mesh", "mesh": 0, "translation": [0.0, 2.0, 0.0]}, {"name": "handle", "translation": [0.5, 0.0, 0.0]}],
        "scenes": [{"nodes": [0]}]
    });
    let model = write_glb("cup.glb", &doc.to_string(), &bin.bytes);
    let pack = build(&model, json!({}));
    assert_eq!(pack.joints.len(), 1);
    assert!(pack.clips.is_empty());
    let ys: Vec<f32> = pack.meshes[0].vertices.iter().map(|v| v.pos[1]).collect();
    assert_eq!(ys, [2.0, 2.0, 3.0]);
    assert_eq!(pack.sockets[0].name, "handle");
}

#[test]
fn an_unsupported_extension_is_a_stable_error() {
    let doc = json!({
        "asset": {"version": "2.0"},
        "extensionsRequired": ["KHR_texture_basisu"],
        "scenes": [{"nodes": []}]
    });
    let model = write_glb("basisu.glb", &doc.to_string(), &[]);
    assert_eq!(error_code(&model, json!({})), "E_GLTF_EXTENSION");
}

#[test]
fn names_the_model_lacks_are_e_actor_name() {
    let model = arm_glb("names.glb");
    for extra in [
        json!({"sockets": {"grip": {"joint": "ankle"}}}),
        json!({"markers": {"jump": {"top": 0.1}}}),
        json!({"feet": ["toe"]}),
        json!({"materials": {"glass": {"rim": 1.0}}}),
        json!({"hidden": ["hat"]}),
        json!({"faces": {"look": {"materials": ["mat_hand"], "grid": [2, 2], "frames": ["a"], "start": "z"}}}),
    ] {
        assert_eq!(error_code(&model, extra.clone()), "E_ACTOR_NAME", "{extra}");
    }
}

#[test]
fn a_description_off_the_schema_is_e_schema() {
    let err =
        ActorSource::from_json(r#"{"name": "x", "model": "m.glb", "colour": 1}"#).unwrap_err();
    assert_eq!(err.code(), "E_SCHEMA");
}

#[test]
#[ignore = "rewrites examples/demo/actors/arm.glb"]
fn write_example_arm() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo/actors/arm.glb");
    std::fs::copy(arm_glb("example_arm.glb"), example).unwrap();
}
