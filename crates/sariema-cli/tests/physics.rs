// SPDX-License-Identifier: Apache-2.0

//! `build` and `inspect` of collision packs, `check` and `physics-map`, over
//! `examples/collision`.

use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::TempDir;

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .canonicalize()
        .unwrap()
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn sariema(args: &[&str]) -> (i32, Value) {
    let out = cargo_bin_cmd!("sariema")
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
    (out.status.code().unwrap(), value)
}

fn failure(args: &[&str]) -> (String, String) {
    let (code, out) = sariema(args);
    assert_eq!((code, &out["ok"]), (1, &json!(false)), "{out}");
    (
        out["error"]["code"].as_str().unwrap().to_owned(),
        out["error"]["message"].as_str().unwrap().to_owned(),
    )
}

fn hall() -> Value {
    serde_json::from_slice(&std::fs::read(example("collision").join("rooms/hall.json")).unwrap())
        .unwrap()
}

/// A copy of the example project in `tmp` whose hall is `room`.
fn project_with(tmp: &TempDir, room: Value) -> PathBuf {
    let dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(dir.join("rooms")).unwrap();
    let project = json!({
        "name": "collision",
        "assets_root": example("collision"),
        "rooms": { "hall": "rooms/hall.json" }
    });
    std::fs::write(dir.join("sariema.json"), project.to_string()).unwrap();
    std::fs::write(dir.join("rooms/hall.json"), room.to_string()).unwrap();
    dir
}

fn with_spot(name: &str, at: [f32; 3]) -> Value {
    let mut room = hall();
    let spots = room["collision"]["spots"].as_array_mut().unwrap();
    spots.iter_mut().find(|s| s["name"] == name).unwrap()["at"] = json!(at);
    room
}

#[test]
fn build_writes_the_collision_pack_beside_the_room_pack_and_inspect_reads_it() {
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("hall.srma");
    let (code, built) = sariema(&[
        "build",
        arg(&example("collision")),
        "--room",
        "hall",
        "-o",
        arg(&pack),
    ]);
    assert_eq!(code, 0, "{built}");
    let collision = tmp.path().join("hall.srmc");
    assert_eq!(built["collision"]["output"], arg(&collision));
    assert_eq!(
        built["collision"]["bytes"],
        std::fs::metadata(&collision).unwrap().len()
    );

    let (code, inspected) = sariema(&["inspect", arg(&collision)]);
    assert_eq!(code, 0, "{inspected}");
    assert_eq!(inspected["kind"], "collision");
    assert_eq!(inspected["room"], "hall");
    assert_eq!(inspected["colliders"]["count"], 12);
    assert_eq!(inspected["colliders"]["disabled"], json!(["a.lampada"]));
    assert_eq!(
        inspected["colliders"]["shapes"],
        json!({ "box": 12, "sphere": 0, "capsule": 0, "hull": 0, "mesh": 0 })
    );
    assert_eq!(inspected["triggers"], json!(["doorway"]));
    assert_eq!(inspected["actors"][0]["name"], "hero");
    assert_eq!(inspected["spots"], json!(["start", "desk", "back"]));
    assert_eq!(
        inspected["routes"],
        json!([["start", "back"], ["desk", "back"]])
    );
    assert_eq!(
        built["collision"]["colliders"],
        inspected["colliders"]["count"]
    );
}

#[test]
fn a_room_without_collision_writes_no_collision_pack() {
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("demo.srma");
    let (code, built) = sariema(&[
        "build",
        arg(&example("demo")),
        "--room",
        "demo",
        "-o",
        arg(&pack),
    ]);
    assert_eq!(code, 0, "{built}");
    assert_eq!(built["collision"], Value::Null);
    assert!(!tmp.path().join("demo.srmc").exists());
}

#[test]
fn check_passes_the_example() {
    let (code, out) = sariema(&["check", arg(&example("collision")), "--room", "hall"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["rooms"][0]["room"], "hall");
    assert_eq!(out["rooms"][0]["spots"], 3);
    assert_eq!(out["rooms"][0]["routes"], 2);
    assert_eq!(out["warnings"], json!([]));
    let (code, all) = sariema(&["check", arg(&example("collision"))]);
    assert_eq!(code, 0, "{all}");
    assert_eq!(all["rooms"][0], out["rooms"][0]);
    assert_eq!(all["rooms"][1]["room"], "sill");
}

#[test]
fn a_spot_inside_a_crate_fails_naming_room_and_spot() {
    let tmp = TempDir::new().unwrap();
    let dir = project_with(&tmp, with_spot("desk", [0.6, 0.0, -2.8]));
    let (code, message) = failure(&["check", arg(&dir), "--room", "hall"]);
    assert_eq!(code, "E_SPOT_BLOCKED");
    assert!(
        message.contains("room hall")
            && message.contains("spot desk")
            && message.contains("a.caixa"),
        "{message}"
    );
}

#[test]
fn a_spot_above_the_floor_stands_on_nothing() {
    let tmp = TempDir::new().unwrap();
    let dir = project_with(&tmp, with_spot("start", [4.5, 0.5, -1.0]));
    let (code, message) = failure(&["check", arg(&dir), "--room", "hall"]);
    assert_eq!(code, "E_SPOT_NO_GROUND");
    assert!(message.contains("spot start"), "{message}");
}

#[test]
fn a_closed_doorway_leaves_the_routes_without_a_path() {
    let tmp = TempDir::new().unwrap();
    let mut room = hall();
    room["collision"]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "name": "bars", "box": { "center": [3.5, 1.0, -3.0], "half_extents": [0.5, 1.0, 0.05] } }));
    let dir = project_with(&tmp, room);
    let (code, message) = failure(&["check", arg(&dir), "--room", "hall"]);
    assert_eq!(code, "E_PATH_NONE");
    assert!(
        message.contains("from spot start to spot back")
            && message.contains("from spot desk to spot back"),
        "{message}"
    );
}

/// The hall with the `back` spot on a landing 0.75 m up, reached by three
/// 0.25 m risers that run wall to wall.
fn with_stair() -> Value {
    let mut room = hall();
    let primitives = room["collision"]["primitives"].as_array_mut().unwrap();
    for (k, south) in [(1, -2.5), (2, -3.1), (3, -3.7)] {
        let top = 0.25 * k as f32;
        primitives.push(json!({ "name": format!("step{k}"), "box": {
            "center": [3.0, top / 2.0, (south - 6.0) / 2.0],
            "half_extents": [3.0, top / 2.0, (south + 6.0) / 2.0] } }));
    }
    let spots = room["collision"]["spots"].as_array_mut().unwrap();
    spots.iter_mut().find(|s| s["name"] == "back").unwrap()["at"] = json!([3.5, 0.75, -5.0]);
    room
}

#[test]
fn check_walks_the_routes_with_the_games_step_height() {
    let tmp = TempDir::new().unwrap();
    let dir = project_with(&tmp, with_stair());
    let (code, message) = failure(&["check", arg(&dir), "--room", "hall"]);
    assert_eq!(code, "E_PATH_NONE", "{message}");
    let path = dir.join("sariema.json");
    let mut project: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    project["game"] =
        json!({ "settings": { "physics": { "controller": { "step_height": 0.26 } } } });
    std::fs::write(&path, project.to_string()).unwrap();
    let (code, out) = sariema(&["check", arg(&dir), "--room", "hall"]);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_model_wider_than_its_collider_warns() {
    let tmp = TempDir::new().unwrap();
    let mut room = hall();
    room["collision"]["actors"][0]["visual_bounds"] =
        json!({ "min": [-0.5, 0.0, -0.3], "max": [0.5, 1.4, 0.3] });
    let dir = project_with(&tmp, room);
    let (code, out) = sariema(&["check", arg(&dir), "--room", "hall"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["warnings"][0]["code"], "W_ACTOR_BOUNDS");
    let message = out["warnings"][0]["message"].as_str().unwrap();
    assert!(
        message.contains("hero") && message.contains("0.16"),
        "{message}"
    );
    let (code, out) = sariema(&[
        "check",
        arg(&dir),
        "--room",
        "hall",
        "--bounds-margin",
        "0.2",
    ]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["warnings"], json!([]));
}

#[test]
fn check_of_an_unknown_room_is_a_schema_error() {
    let (code, message) = failure(&["check", arg(&example("collision")), "--room", "attic"]);
    assert_eq!(code, "E_SCHEMA");
    assert!(message.contains("attic"), "{message}");
}

#[test]
fn physics_map_draws_colliders_spots_and_a_path() {
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("hall.srma");
    let (code, built) = sariema(&[
        "build",
        arg(&example("collision")),
        "--room",
        "hall",
        "-o",
        arg(&pack),
    ]);
    assert_eq!(code, 0, "{built}");
    let map = tmp.path().join("map.png");
    let (code, out) = sariema(&[
        "physics-map",
        arg(&tmp.path().join("hall.srmc")),
        "-o",
        arg(&map),
        "--path",
        "start:back",
    ]);
    assert_eq!(code, 0, "{out}");
    let image = image::open(&map).unwrap().to_rgb8();
    assert_eq!(
        [image.width(), image.height()],
        [
            out["width"].as_u64().unwrap() as u32,
            out["height"].as_u64().unwrap() as u32
        ]
    );
    let ppm = out["pixels_per_metre"].as_f64().unwrap();
    let origin = &out["origin"];
    let pixel = |x: f64, z: f64| {
        let px = ((x - origin[0].as_f64().unwrap()) * ppm) as u32;
        let py = ((z - origin[1].as_f64().unwrap()) * ppm) as u32;
        image.get_pixel(px, py).0
    };
    let crate_top = pixel(0.5, -3.0);
    let floor = pixel(5.0, -4.0);
    assert_ne!(crate_top, floor);
    assert!(
        floor[1] > floor[0] && floor[1] > floor[2],
        "walkable floor is green: {floor:?}"
    );
    let start = out["spots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "start")
        .unwrap();
    assert_eq!(
        start["pixel"][0].as_u64().unwrap(),
        ((4.5 - origin[0].as_f64().unwrap()) * ppm) as u64
    );
    let path = out["path"]["points"].as_array().unwrap();
    assert!(!path.is_empty(), "{out}");
    let doorway = pixel(3.5, -3.0);
    assert!(
        doorway[0] > 200 && doorway[1] < 100 && doorway[2] > 200,
        "the path crosses the doorway in magenta: {doorway:?}"
    );
    assert_eq!(path.last().unwrap(), &json!([3.5, 0.0, -5.0]));

    let (code, message) = failure(&[
        "physics-map",
        arg(&tmp.path().join("hall.srmc")),
        "-o",
        arg(&map),
        "--path",
        "start:nowhere",
    ]);
    assert_eq!(code, "E_USAGE");
    assert!(message.contains("nowhere"), "{message}");
    let (code, _) = failure(&["physics-map", arg(&pack), "-o", arg(&map)]);
    assert_eq!(code, "E_PACK_BAD_MAGIC");
}

#[test]
fn a_spot_sunk_into_a_crate_top_says_where_it_stands_clear() {
    let tmp = TempDir::new().unwrap();
    let dir = project_with(&tmp, with_spot("desk", [2.5, 0.95, -3.0]));
    let (code, message) = failure(&["check", arg(&dir), "--room", "hall"]);
    assert_eq!(code, "E_SPOT_BLOCKED");
    assert!(message.contains("stands clear at y 1.0"), "{message}");
}

fn sill_with(edit: impl FnOnce(&mut Value)) -> Value {
    let mut room: Value = serde_json::from_slice(
        &std::fs::read(example("collision").join("rooms/sill.json")).unwrap(),
    )
    .unwrap();
    edit(&mut room["collision"]["traversals"][0]);
    room
}

#[test]
fn a_traversal_over_a_sill_ending_on_free_floor_passes_and_inspect_lists_it() {
    let (code, out) = sariema(&["check", arg(&example("collision")), "--room", "sill"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["rooms"][0]["traversals"], 1);
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("sill.srma");
    let (code, built) = sariema(&[
        "build",
        arg(&example("collision")),
        "--room",
        "sill",
        "-o",
        arg(&pack),
    ]);
    assert_eq!(code, 0, "{built}");
    let (code, inspected) = sariema(&["inspect", arg(&tmp.path().join("sill.srmc"))]);
    assert_eq!(code, 0, "{inspected}");
    assert_eq!(inspected["traversals"], json!(["window"]));
}

#[test]
fn a_traversal_ending_inside_the_sill_or_in_the_air_fails() {
    for (end, says) in [
        ([3.5, 0.0, -3.2], "hero's collider is 0.500 m inside sill"),
        ([3.5, 0.5, -4.2], "hero stands on nothing within 0.05 m"),
    ] {
        let tmp = TempDir::new().unwrap();
        let dir = project_with(&tmp, sill_with(|t| t["points"][1] = json!(end)));
        let (code, message) = failure(&["check", arg(&dir), "--room", "hall"]);
        assert_eq!(code, "E_TRAVERSAL_END");
        assert!(
            message.contains("room hall: traversal window: at the end, ") && message.contains(says),
            "{message}"
        );
    }
}

#[test]
fn a_traversal_that_holds_its_end_may_end_inside_the_sill_or_in_the_air() {
    for end in [[3.5, 0.0, -3.2], [3.5, 0.5, -4.2]] {
        let tmp = TempDir::new().unwrap();
        let dir = project_with(
            &tmp,
            sill_with(|t| {
                t["points"][1] = json!(end);
                t["hold_end"] = json!(true);
            }),
        );
        let (code, out) = sariema(&["check", arg(&dir), "--room", "hall"]);
        assert_eq!(code, 0, "{out}");
    }
}

#[test]
fn a_contact_off_the_sill_or_sunk_into_it_fails() {
    for (at, says) in [
        ([3.5, 1.2, -3.2], "is not within 0.02 m of a surface"),
        ([3.5, 0.5, -3.0], "is 0.400 m inside sill"),
    ] {
        let tmp = TempDir::new().unwrap();
        let dir = project_with(&tmp, sill_with(|t| t["contacts"][0]["at"] = json!(at)));
        let (code, message) = failure(&["check", arg(&dir), "--room", "hall"]);
        assert_eq!(code, "E_CONTACT_OFF_SURFACE");
        assert!(
            message.contains("room hall: traversal window: contact foot ")
                && message.contains(says),
            "{message}"
        );
    }
    let tmp = TempDir::new().unwrap();
    let dir = project_with(
        &tmp,
        sill_with(|t| t["contacts"][0]["at"] = json!([3.5, 1.2, -3.2])),
    );
    let (code, out) = sariema(&[
        "check",
        arg(&dir),
        "--room",
        "hall",
        "--contact-tolerance",
        "0.35",
    ]);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_contact_on_a_surface_out_of_the_actors_reach_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = project_with(
        &tmp,
        sill_with(|t| t["contacts"][0]["at"] = json!([0.5, 0.0, -0.5])),
    );
    let (code, message) = failure(&["check", arg(&dir), "--room", "hall"]);
    assert_eq!(code, "E_CONTACT_OUT_OF_REACH");
    assert!(
        message.contains("room hall: traversal window: contact foot ")
            && message.contains("hero's visual bounds"),
        "{message}"
    );
}
