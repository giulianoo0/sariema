// SPDX-License-Identifier: Apache-2.0

//! Groups by sub-room, nodes, pieces with states and hinges, built from a
//! kit written here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::write_glb;
use glam::{Mat4, Vec3};
use sariema_format::{RoomPack, decode, encode};
use sariema_import::{ImportError, RoomSource, build_room};
use serde_json::{Value, json};

mod common;

/// Two pieces. `window`: a frame triangle at its origin and a hinged leaf
/// `window_leaf` 0.5 m east whose axis is vertical. `lamp`: a base triangle
/// and two state nodes, `lamp_on` (hidden at start) and `lamp_off`.
fn kit() -> PathBuf {
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let mut bin: Vec<u8> = bytemuck::cast_slice(&positions).to_vec();
    bin.extend([0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()));
    let json = r#"{
        "asset": { "version": "2.0" },
        "buffers": [{ "byteLength": 42 }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
            { "buffer": 0, "byteOffset": 36, "byteLength": 6 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3" },
            { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }
        ],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1 }] }],
        "nodes": [
            { "name": "window", "children": [1, 2] },
            { "name": "window_frame", "mesh": 0 },
            { "name": "window_leaf", "mesh": 0, "translation": [0.5, 0.0, 0.0],
              "extras": { "giro": { "eixo": "vertical" } } },
            { "name": "lamp", "children": [4, 5, 6],
              "extras": { "estados": { "on": ["lamp_on"], "off": ["lamp_off"] } } },
            { "name": "lamp_base", "mesh": 0 },
            { "name": "lamp_on", "mesh": 0, "extras": { "oculto_no_inicio": true } },
            { "name": "lamp_off", "mesh": 0 }
        ],
        "scenes": [{ "nodes": [0, 3] }],
        "scene": 0
    }"#;
    write_glb("scenery_kit.glb", json, &bin)
}

fn instance(piece: &str, x: f32, estado: Option<&str>, giros: Value) -> Value {
    json!({
        "peca": piece, "x": x, "y": 0.0, "z": 0.0, "rz": 0, "rot": null,
        "estado": estado, "giros": giros, "escala": 1.0
    })
}

fn write(name: &str, value: &Value) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, value.to_string()).unwrap();
    path
}

/// Sub-room `a` (corner at the origin) holds a window turned 30° and a lamp
/// that is on, lit by `a_light`; `b`, 3 m east, holds a lamp in its start
/// look, lit by `b_light`. `changes` replaces top-level keys of the room
/// description and `montagem` edits the placement file.
fn build(changes: Value, montagem: impl FnOnce(&mut Value)) -> Result<RoomPack, ImportError> {
    let mut m = json!({
        "celula": 1.5,
        "comodos": {
            "a": { "sw_celulas": [0, 0], "instancias": [
                instance("window", 0.0, None, json!({ "leaf": 30.0 })),
                instance("lamp", 1.0, Some("on"), Value::Null),
            ] },
            "b": { "sw_celulas": [2, 0], "instancias": [
                instance("lamp", 0.5, None, Value::Null),
            ] }
        }
    });
    montagem(&mut m);
    let light = |name: &str| json!({ "nome": name, "tipo": "ponto", "cor": "#FFFFFF", "int": 2.0, "pos": [1.0, 1.0, 2.0], "dist": 5.0 });
    let sky = |light_name: &str| {
        json!({
            "hemi_ceu": "#FFFFFF", "hemi_chao": "#000000", "hemi_int": 1.0,
            "sol": "#FFFFFF", "sol_int": 1.0, "sol_dir_three": [0.0, 1.0, 0.0],
            "luzes": [light(light_name)]
        })
    };
    let luz = json!({ "comodos": { "a": sky("a_light"), "b": sky("b_light") } });
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let tag = CALLS.fetch_add(1, Ordering::Relaxed);
    let mut sala = json!({
        "name": "house",
        "kit": kit(),
        "montagem": write(&format!("scenery_montagem_{tag}.json"), &m),
        "luz": write(&format!("scenery_luz_{tag}.json"), &luz),
        "show": ["a", "b"],
        "loose": ["window", "lamp"],
        "drawn_materials": [],
        "rim_materials": [],
        "additive_materials": [],
        "look": {
            "exposure": 1.0, "bloom_threshold": 1.0, "bloom_strength": 0.3,
            "tint": [1.0, 1.0, 1.0], "real": 0.0, "night": 0.0, "tilt": 0.0, "focus": 8.0
        },
        "camera": { "fov_y_deg": 40.0, "pitch_deg": 45.0, "distance": 8.0, "focus": [0.0, 0.0, 0.0] },
        "background": { "top": [0.0, 0.0, 0.0], "bottom": [0.0, 0.0, 0.0] }
    });
    for (key, value) in changes.as_object().unwrap() {
        sala[key] = value.clone();
    }
    let room = build_room(&RoomSource::from_json(&sala.to_string())?, Path::new("/"))?;
    Ok(decode(&encode(&room)).expect("the importer writes valid packs"))
}

fn draw_names(room: &RoomPack, draws: &[u32]) -> Vec<String> {
    let mut names: Vec<String> = (draws.iter())
        .map(|&d| room.draws[d as usize].name.clone().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn each_shown_sub_room_is_a_group_of_its_draws_and_lights() {
    let room = build(json!({}), |_| {}).unwrap();
    let names: Vec<_> = room
        .groups
        .iter()
        .map(|g| (g.name.as_str(), g.on))
        .collect();
    assert_eq!(names, [("a", true), ("b", true)]);
    let a = &room.groups[0];
    assert!(
        draw_names(&room, &a.draws)
            .iter()
            .all(|n| n.starts_with("a."))
    );
    assert!(
        draw_names(&room, &room.groups[1].draws)
            .iter()
            .all(|n| n.starts_with("b."))
    );
    let grouped: usize = room.groups.iter().map(|g| g.draws.len()).sum();
    assert_eq!(grouped, room.draws.len(), "every draw is in a group");
    let lights = |g: usize| -> Vec<&str> {
        (room.groups[g].lights.iter())
            .map(|&l| room.lights[l as usize].name.as_str())
            .collect()
    };
    assert_eq!(lights(0), ["a.a_light"]);
    assert_eq!(lights(1), ["b.b_light"]);
}

#[test]
fn groups_off_start_switched_off() {
    let room = build(json!({ "groups_off": ["b"] }), |_| {}).unwrap();
    assert_eq!(
        room.groups.iter().map(|g| g.on).collect::<Vec<_>>(),
        [true, false]
    );
    let error = build(json!({ "groups_off": ["c"] }), |_| {}).unwrap_err();
    assert!(error.to_string().contains("groups_off lists c"), "{error}");
}

#[test]
fn a_loose_piece_is_a_node_pivoted_at_its_placement() {
    let room = build(json!({}), |_| {}).unwrap();
    let node = room.nodes.iter().find(|n| n.name == "b.lamp").unwrap();
    // Corner 3 m east, then x 0.5 m: Blender (x, y, z) is the engine's (x, z, −y).
    let pivot = Mat4::from_cols_array(&node.pivot);
    assert!(pivot.abs_diff_eq(Mat4::from_translation(Vec3::new(3.5, 0.0, 0.0)), 1e-6));
    assert!(draw_names(&room, &node.draws).iter().all(|n| n == "b.lamp"));
    assert!(!node.draws.is_empty());
    assert_eq!(node.hinge, None);
}

#[test]
fn a_loose_piece_keeps_every_state_and_starts_in_its_own() {
    let room = build(json!({}), |_| {}).unwrap();
    let piece = |name: &str| room.pieces.iter().find(|p| p.name == name).unwrap();
    let names: Vec<&str> = (piece("a.lamp").states.iter())
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["", "off", "on"]);
    assert_eq!(piece("a.lamp").state, 2, "a's lamp is on");
    assert_eq!(piece("b.lamp").state, 0, "b's lamp has no state");
    // The start look shows the base and lamp_off; `on` the base and lamp_on.
    let a = piece("a.lamp");
    for look in &a.states {
        assert_eq!(look.draws.len(), 2, "state {:?}", look.name);
    }
    let node = room.nodes.iter().find(|n| n.name == "a.lamp").unwrap();
    let mut looks: Vec<u32> = a.states.iter().flat_map(|s| s.draws.clone()).collect();
    looks.sort();
    let mut node_draws = node.draws.clone();
    node_draws.sort();
    assert_eq!(looks, node_draws, "the node holds every look's draws");
}

#[test]
fn a_hinged_node_is_split_out_and_stored_turned() {
    let room = build(json!({}), |_| {}).unwrap();
    let leaf = room
        .nodes
        .iter()
        .find(|n| n.name == "a.window_leaf")
        .unwrap();
    let hinge = leaf.hinge.as_ref().unwrap();
    assert_eq!(hinge.axis, [0.0, 1.0, 0.0]);
    assert_eq!(hinge.angle_deg, 30.0);
    let pivot = Mat4::from_cols_array(&leaf.pivot);
    assert!(pivot.abs_diff_eq(Mat4::from_translation(Vec3::new(0.5, 0.0, 0.0)), 1e-6));
    assert_eq!(leaf.draws.len(), 1);
    let draw = &room.draws[leaf.draws[0] as usize];
    // The leaf's triangle, (0,0,0) (1,0,0) (0,1,0) at the pivot, turned 30° about +Y.
    let turned = Mat4::from_translation(Vec3::new(0.5, 0.0, 0.0))
        * Mat4::from_rotation_y(30f32.to_radians());
    let mesh = &room.meshes[draw.mesh as usize];
    let world = Mat4::from_cols_array(&draw.transform);
    let corner = world.transform_point3(Vec3::from(mesh.vertices[1].pos));
    assert!(
        corner.abs_diff_eq(turned.transform_point3(Vec3::X), 1e-5),
        "{corner}"
    );
    let window = room.nodes.iter().find(|n| n.name == "a.window").unwrap();
    assert!(
        window.draws.iter().all(|d| !leaf.draws.contains(d)),
        "the leaf is not also drawn by the window"
    );
}

#[test]
fn a_hinge_the_piece_lacks_is_an_error() {
    let error = build(json!({}), |m| {
        m["comodos"]["a"]["instancias"][0]["giros"] = json!({ "lid": 10.0 });
    })
    .unwrap_err();
    assert!(error.to_string().contains("has no hinge lid"), "{error}");
}
