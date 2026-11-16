// SPDX-License-Identifier: Apache-2.0

//! `build --actor`, `inspect` of an actor pack and `run` with actors, on the
//! demo project's arm.

use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::TempDir;

fn demo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo")
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

fn error_code(value: &Value) -> &str {
    assert_eq!(value["ok"], false, "{value}");
    value["error"]["code"].as_str().unwrap()
}

/// The demo room and arm packs in `dir`.
fn packs(dir: &Path) -> (PathBuf, PathBuf) {
    let (room, arm) = (dir.join("demo.srma"), dir.join("arm.srmk"));
    let (code, out) = sariema(&["build", arg(&demo()), "--room", "demo", "-o", arg(&room)]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = sariema(&["build", arg(&demo()), "--actor", "arm", "-o", arg(&arm)]);
    assert_eq!(code, 0, "{out}");
    (room, arm)
}

/// `run --headless` of `room` at 320×180 with `extra`, into `shot`.
fn run(room: &Path, shot: &Path, extra: &[&str]) -> (i32, Value) {
    let mut args = vec![
        "run",
        arg(room),
        "--headless",
        "--frames",
        "3",
        "--screenshot",
        arg(shot),
        "--width",
        "320",
        "--height",
        "180",
    ];
    args.extend(extra);
    sariema(&args)
}

#[test]
fn build_then_inspect_an_actor() {
    let tmp = TempDir::new().unwrap();
    let arm = tmp.path().join("arm.srmk");
    let (code, built) = sariema(&["build", arg(&demo()), "--actor", "arm", "-o", arg(&arm)]);
    assert_eq!(code, 0, "{built}");
    assert_eq!(built["actor"], "arm");
    assert_eq!(
        (built["joints"].as_u64(), built["clips"].as_u64()),
        (Some(3), Some(3))
    );
    assert_eq!(built["bytes"], std::fs::metadata(&arm).unwrap().len());

    let (code, inspected) = sariema(&["inspect", arg(&arm)]);
    assert_eq!(code, 0, "{inspected}");
    assert_eq!(inspected["joints"], json!(["shoulder", "elbow", "wrist"]));
    let clips = inspected["clips"].as_array().unwrap();
    let durations: Vec<(&str, f64)> = clips
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap(),
                c["duration_s"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(durations, [("bend", 1.0), ("wave", 0.8), ("turn", 1.0)]);
    assert_eq!(clips[0]["looping"], true);
    assert_eq!(inspected["sockets"], json!(["tip", "grip"]));
    assert!(inspected["stats"]["key_bytes"].as_u64().unwrap() > 0);
}

#[test]
fn an_actor_the_project_lacks_is_e_schema() {
    let tmp = TempDir::new().unwrap();
    let (demo, out) = (demo(), tmp.path().join("x.srmk"));
    let (code, value) = sariema(&["build", arg(&demo), "--actor", "leg", "-o", arg(&out)]);
    assert_eq!((code, error_code(&value)), (1, "E_SCHEMA"));
    let both = [
        "build",
        arg(&demo),
        "--room",
        "demo",
        "--actor",
        "arm",
        "-o",
        arg(&out),
    ];
    let (code, value) = sariema(&both);
    assert_eq!((code, error_code(&value)), (1, "E_USAGE"));
}

#[test]
fn run_poses_actors_at_a_time_of_a_clip() {
    let tmp = TempDir::new().unwrap();
    let (room, arm) = packs(tmp.path());
    let placed = format!("{}@0.6,0,1.2,30", arg(&arm));
    let shot = |name: &str, time: &str| {
        let shot = tmp.path().join(name);
        let (code, out) = run(
            &room,
            &shot,
            &["--actor", &placed, "--clip", "bend", "--time", time],
        );
        assert_eq!(code, 0, "{out}");
        (image::open(&shot).unwrap().to_rgba8(), out)
    };
    let (rest, out) = shot("rest.png", "0");
    let (bent, _) = shot("bent.png", "0.5");
    assert_eq!(out["frame_stats"]["actors_drawn"], 1, "{out}");
    assert_eq!(out["pipelines_created_during_frames"], 0, "{out}");
    assert_eq!(out["actors"][0]["joints"], 3, "{out}");
    assert!(out["actors_ms_per_frame"].as_f64().unwrap() >= 0.0, "{out}");
    assert!(rest.pixels().zip(bent.pixels()).any(|(a, b)| a != b));
}

#[test]
fn a_filmstrip_tiles_frames_over_the_clip() {
    let tmp = TempDir::new().unwrap();
    let (room, arm) = packs(tmp.path());
    let placed = format!("{}@0.6,0,1.2,30", arg(&arm));
    let shot = tmp.path().join("strip.png");
    let extra = [
        "--actor",
        &placed,
        "--clip",
        "bend",
        "--then",
        "wave@0.4",
        "--fade",
        "0.2",
        "--filmstrip",
        "3",
        "--columns",
        "3",
        "--prop",
        &format!("{}@0:tip", arg(&arm)),
    ];
    let (code, out) = run(&room, &shot, &extra);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["filmstrip"]["times_s"], json!([0.0, 0.5, 1.0]), "{out}");
    assert_eq!(image::image_dimensions(&shot).unwrap(), (960, 180));
    assert_eq!(out["frame_stats"]["actors_drawn"], 2, "{out}");
}

#[test]
fn names_the_actor_lacks_are_e_actor_name() {
    let tmp = TempDir::new().unwrap();
    let (room, arm) = packs(tmp.path());
    let placed = format!("{}@0,0,0,0", arg(&arm));
    let shot = tmp.path().join("x.png");
    let prop = format!("{}@0:elbow_tip", arg(&arm));
    let cases: [&[&str]; 3] = [
        &["--actor", &placed, "--clip", "dance"],
        &["--actor", &placed, "--face", "eyes=open"],
        &["--actor", &placed, "--prop", &prop],
    ];
    for extra in cases {
        let (code, out) = run(&room, &shot, extra);
        assert_eq!((code, error_code(&out)), (1, "E_ACTOR_NAME"), "{extra:?}");
    }
}

#[test]
fn malformed_stage_arguments_are_usage_errors() {
    let cases: [&[&str]; 4] = [
        &["run", "x.srma", "--actor", "arm.srmk@1,2"],
        &[
            "run",
            "x.srma",
            "--actor",
            "arm.srmk@0,0,0,0",
            "--then",
            "wave",
        ],
        &["run", "x.srma", "--clip", "bend"],
        &["run", "x.srma", "--filmstrip", "3"],
    ];
    for args in cases {
        let (code, out) = sariema(args);
        assert_eq!((code, error_code(&out)), (1, "E_USAGE"), "{args:?}");
    }
}
