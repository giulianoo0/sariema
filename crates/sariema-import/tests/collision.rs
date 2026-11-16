// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use sariema_format::collision::{CollisionPack, Shape, decode, encode};
use sariema_import::{ImportError, RoomSource, build_collision};
use serde_json::{Value, json};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn sala(collision: Value) -> Value {
    let mut sala: Value =
        serde_json::from_slice(&std::fs::read(fixtures().join("sala_min.json")).unwrap()).unwrap();
    sala["collision"] = collision;
    sala
}

fn bake(sala: &Value) -> Result<Option<CollisionPack>, ImportError> {
    build_collision(&RoomSource::from_json(&sala.to_string())?, &fixtures())
}

fn baked(collision: Value) -> CollisionPack {
    bake(&sala(collision)).unwrap().unwrap()
}

fn schema_error(collision: Value) -> String {
    let e = bake(&sala(collision)).unwrap_err();
    assert_eq!(e.code(), "E_SCHEMA", "{e}");
    e.to_string()
}

fn floor() -> Value {
    json!({ "name": "floor", "box": { "center": [2.0, -0.05, -1.5], "half_extents": [3.0, 0.05, 2.0] } })
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-4)
}

#[test]
fn a_room_without_collision_bakes_nothing() {
    let mut plain = sala(Value::Null);
    plain.as_object_mut().unwrap().remove("collision");
    assert!(bake(&plain).unwrap().is_none());
}

#[test]
fn box_pieces_bake_to_boxes_named_like_loose_draws() {
    let pack = baked(json!({ "primitives": [floor()] }));
    let names: Vec<&str> = pack.colliders.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["a.caixa", "a.caixa.2", "a.mesa", "b.lampada", "floor"]
    );
    let shape = |name: &str| {
        &pack
            .colliders
            .iter()
            .find(|c| c.name == name)
            .unwrap()
            .shape
    };
    let Shape::Box {
        center,
        half_extents,
        ..
    } = shape("a.caixa")
    else {
        panic!("{:?}", shape("a.caixa"));
    };
    assert!(close(*center, [0.75, 0.5, -0.75]), "{center:?}");
    assert!(close(*half_extents, [0.5; 3]), "{half_extents:?}");
    let Shape::Box { rotation, .. } = shape("a.caixa.2") else {
        panic!("{:?}", shape("a.caixa.2"));
    };
    let s = std::f32::consts::FRAC_1_SQRT_2;
    assert!(
        close([rotation[0], rotation[1], rotation[2]], [s, 0.0, -s]),
        "{rotation:?}"
    );
    let Shape::Box { center, .. } = shape("a.mesa") else {
        panic!("{:?}", shape("a.mesa"));
    };
    assert!(close(*center, [1.5, 1.0, -2.25]), "{center:?}");
    let enabled: Vec<bool> = pack.colliders.iter().map(|c| c.enabled).collect();
    assert_eq!(enabled, [true, true, true, false, true]);
    assert_eq!(decode(&encode(&pack)).unwrap(), pack);
}

#[test]
fn shape_lists_force_a_mesh_or_a_hull_and_exclude_drops_pieces() {
    let pack = baked(json!({
        "as_mesh": ["caixa"],
        "as_hull": ["mesa"],
        "exclude": ["lampada"]
    }));
    let names: Vec<&str> = pack.colliders.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["a.caixa", "a.caixa.2", "a.mesa"]);
    let Shape::Mesh {
        vertices,
        triangles,
    } = &pack.colliders[0].shape
    else {
        panic!("{:?}", pack.colliders[0].shape);
    };
    assert_eq!((vertices.len(), triangles.len()), (8, 12));
    let Shape::Hull { points } = &pack.colliders[2].shape else {
        panic!("{:?}", pack.colliders[2].shape);
    };
    assert_eq!(points.len(), 8);
    assert!(
        points
            .iter()
            .all(|p| (p[1] - 0.5).abs() < 1e-4 || (p[1] - 1.5).abs() < 1e-4)
    );
}

#[test]
fn lists_naming_pieces_the_rooms_do_not_place_are_errors() {
    for key in ["exclude", "as_mesh", "as_hull", "as_box"] {
        let e = schema_error(json!({ key: ["sofa"] }));
        assert!(e.contains(key) && e.contains("sofa"), "{e}");
    }
    let e = schema_error(json!({ "as_mesh": ["caixa"], "as_hull": ["caixa"] }));
    assert!(e.contains("caixa"), "{e}");
}

#[test]
fn actors_spots_triggers_and_routes_resolve_by_name() {
    let pack = baked(json!({
        "primitives": [floor(), { "name": "root", "capsule": { "a": [0.0, 0.0, 0.0], "b": [1.0, 1.0, 0.0], "radius": 0.2 }, "enabled": false }],
        "triggers": [{ "name": "door", "sphere": { "center": [3.0, 1.0, -1.0], "radius": 0.5 } }],
        "actors": [
            { "name": "hero", "radius": 0.3, "height": 1.0, "head": { "radius": 0.34, "height": 1.1 } },
            { "name": "mila", "visual_bounds": { "min": [-0.25, 0.0, -0.2], "max": [0.25, 1.2, 0.2] } }
        ],
        "spots": [
            { "name": "desk", "actor": "hero", "at": [2.0, 0.0, 0.5], "yaw_deg": 90.0 },
            { "name": "bed", "actor": "mila", "at": [4.0, 0.0, -2.0] }
        ],
        "routes": [["desk", "bed"]]
    }));
    assert_eq!(pack.room, "sala_min");
    let root = pack.colliders.iter().find(|c| c.name == "root").unwrap();
    assert!(!root.enabled);
    assert_eq!(pack.triggers[0].name, "door");
    let mila = &pack.actors[1];
    assert_eq!(
        (mila.radius, mila.height, mila.head.as_ref()),
        (0.25, 1.2, None)
    );
    assert_eq!(pack.actors[0].head.as_ref().unwrap().forward, 0.0);
    assert_eq!(pack.spots[0].actor, 0);
    assert!((pack.spots[0].yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    assert_eq!((pack.spots[1].actor, pack.spots[1].yaw), (1, 0.0));
    assert_eq!(pack.routes, [[0, 1]]);

    let e = schema_error(
        json!({ "spots": [{ "name": "x", "actor": "nobody", "at": [0.0, 0.0, 0.0] }] }),
    );
    assert!(e.contains("nobody"), "{e}");
    let e = schema_error(json!({
        "actors": [{ "name": "hero", "radius": 0.3, "height": 1.0 }],
        "spots": [{ "name": "x", "actor": "hero", "at": [0.0, 0.0, 0.0] }],
        "routes": [["x", "y"]]
    }));
    assert!(e.contains("route") && e.contains('y'), "{e}");
    let e = schema_error(json!({ "actors": [{ "name": "ghost" }] }));
    assert!(e.contains("ghost"), "{e}");
    let e = schema_error(json!({ "primitives": [floor(), floor()] }));
    assert!(e.contains("floor"), "{e}");
}

#[test]
fn the_schema_rejects_unknown_keys_and_bad_settings() {
    for collision in [
        json!({ "colliders": [] }),
        json!({ "bake": { "simplify_error": -1.0 } }),
        json!({ "bake": { "hull_points": 3 } }),
        json!({ "triggers": [{ "name": "t", "cone": {} }] }),
    ] {
        schema_error(collision);
    }
}
