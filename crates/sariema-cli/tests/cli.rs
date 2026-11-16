// SPDX-License-Identifier: Apache-2.0

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

/// Runs `sariema <args> --json` and returns the exit code and stdout, which
/// must be a single JSON value.
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

fn build_demo(dir: &Path) -> PathBuf {
    let pack = dir.join("demo.srma");
    let (code, out) = sariema(&["build", arg(&demo()), "--room", "demo", "-o", arg(&pack)]);
    assert_eq!(code, 0, "{out}");
    pack
}

#[test]
fn build_then_inspect_demo() {
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("demo.srma");
    let (code, built) = sariema(&["build", arg(&demo()), "--room", "demo", "-o", arg(&pack)]);
    assert_eq!(code, 0, "{built}");
    assert_eq!(built["ok"], true);
    assert_eq!(built["room"], "demo");
    assert_eq!(built["output"], arg(&pack));
    assert_eq!(built["bytes"], std::fs::metadata(&pack).unwrap().len());
    assert_eq!(built["stats"]["triangles"], 36);

    let (code, inspected) = sariema(&["inspect", arg(&pack)]);
    assert_eq!(code, 0, "{inspected}");
    assert_eq!(inspected["stats"], built["stats"]);
    assert_eq!(inspected["stats"]["triangles"], 36);
    assert_eq!(inspected["lights"], json!(["a.foco", "a.abajur"]));
    assert_eq!(inspected["materials"], json!(["mat0"]));
    assert_eq!(inspected["draws"], json!(["a.mesa"]));
}

#[test]
fn textures_flag_overrides_the_room_file() {
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("demo.srma");
    let demo = demo();
    let texture_bytes = |extra: &[&str]| {
        let mut args = vec!["build", arg(&demo), "--room", "demo", "-o", arg(&pack)];
        args.extend(extra);
        let (code, out) = sariema(&args);
        assert_eq!(code, 0, "{out}");
        out["stats"]["texture_bytes"].as_u64().unwrap()
    };
    let astc = texture_bytes(&["--textures", "astc"]);
    let rgba8 = texture_bytes(&["--textures", "rgba8"]);
    assert!(astc < rgba8, "astc {astc}, rgba8 {rgba8}");
    assert_eq!(texture_bytes(&[]), astc);
}

#[test]
fn headless_flags_without_headless_or_their_partners_are_usage_errors() {
    let cases: [&[&str]; 4] = [
        &["run", "x.srma", "--headless", "--frames", "1"],
        &["run", "x.srma", "--headless", "--screenshot", "x.png"],
        &["run", "x.srma", "--screenshot", "x.png"],
        &["run", "x.srma", "--max-background", "0.5"],
    ];
    for args in cases {
        let (code, out) = sariema(args);
        assert_eq!(code, 1, "{args:?}");
        assert_eq!(error_code(&out), "E_USAGE", "{args:?}");
    }
}

#[test]
fn headless_writes_screenshot_and_reports_zero_pipelines() {
    let tmp = TempDir::new().unwrap();
    let pack = build_demo(tmp.path());
    let shot = tmp.path().join("shot.png");
    let (code, out) = sariema(&[
        "run",
        arg(&pack),
        "--headless",
        "--frames",
        "3",
        "--screenshot",
        arg(&shot),
    ]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["frames"], 3);
    assert_eq!(out["screenshot"], arg(&shot));
    assert_eq!(out["pipelines_created_during_frames"], 0);
    assert!(out["cpu_ms_per_frame"].as_f64().unwrap() > 0.0, "{out}");
    let background = out["background_ratio"].as_f64().unwrap();
    assert!(background > 0.0 && background < 1.0, "{out}");
    assert_eq!(out["frame_stats"]["lights_in_frustum"], 2, "{out}");
    assert_eq!(out["frame_stats"]["lights_dropped"], 0, "{out}");
    assert_eq!(image::image_dimensions(&shot).unwrap(), (1920, 1080));
}

#[test]
fn run_reports_cpu_and_wall_time_per_frame() {
    let tmp = TempDir::new().unwrap();
    let pack = build_demo(tmp.path());
    let shot = tmp.path().join("shot.png");
    let (code, out) = sariema(&[
        "run",
        arg(&pack),
        "--headless",
        "--frames",
        "4",
        "--screenshot",
        arg(&shot),
        "--width",
        "320",
        "--height",
        "180",
    ]);
    assert_eq!(code, 0, "{out}");
    let cpu = out["cpu_ms_per_frame"].as_f64().unwrap();
    let wall = out["wall_ms_per_frame"].as_f64().unwrap();
    assert!(cpu > 0.0 && wall >= cpu, "{out}");
}

#[test]
fn run_reports_load_time_and_post_passes() {
    let tmp = TempDir::new().unwrap();
    let pack = build_demo(tmp.path());
    let shot = tmp.path().join("shot.png");
    let (code, out) = sariema(&[
        "run",
        arg(&pack),
        "--headless",
        "--frames",
        "1",
        "--screenshot",
        arg(&shot),
        "--width",
        "320",
        "--height",
        "180",
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(out["load_room_ms"].as_f64().unwrap() > 0.0, "{out}");
    // The composite pass always runs.
    assert!(
        out["frame_stats"]["post_passes"].as_u64().unwrap() >= 1,
        "{out}"
    );
}

#[test]
fn max_background_passes_at_the_ratio_and_fails_below_it() {
    let tmp = TempDir::new().unwrap();
    let pack = build_demo(tmp.path());
    let shot = tmp.path().join("shot.png");
    let run = |extra: &[&str]| {
        let mut args = vec![
            "run",
            arg(&pack),
            "--headless",
            "--frames",
            "1",
            "--screenshot",
            arg(&shot),
            "--width",
            "320",
            "--height",
            "180",
            "--quality",
            "low",
        ];
        args.extend(extra);
        sariema(&args)
    };
    let (code, out) = run(&[]);
    assert_eq!(code, 0, "{out}");
    let background = out["background_ratio"].as_f64().unwrap();
    assert!(background > 0.01, "{out}");

    let (code, out) = run(&["--max-background", &background.to_string()]);
    assert_eq!(code, 0, "{out}");

    let (code, out) = run(&["--max-background", &(background - 0.01).to_string()]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_ROOM_VOID");
}

#[test]
fn unknown_flag_is_error() {
    cargo_bin_cmd!("sariema")
        .args(["build", "x", "--rom", "y"])
        .assert()
        .failure();
    let (code, out) = sariema(&["build", "x", "--rom", "y"]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_USAGE");
}

#[test]
fn json_after_double_dash_is_an_argument_not_the_flag() {
    let cases: [&[&str]; 2] = [
        &["inspect", "--", "--json"],
        &["inspect", "--rom", "--", "--json"],
    ];
    for args in cases {
        let out = cargo_bin_cmd!("sariema").args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}: {out:?}");
    }
}

#[test]
fn help_and_version_are_json_objects_in_json_mode() {
    let (code, out) = sariema(&["run", "--help"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["ok"], true);
    assert!(
        out["help"].as_str().unwrap().contains("--max-background"),
        "{out}"
    );
    let (code, out) = sariema(&["--version"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(
        out,
        json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })
    );
}

#[test]
fn missing_file_reports_e_io_in_json() {
    let (code, out) = sariema(&["inspect", "nada.srma"]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_IO");
    assert!(out["error"]["message"].to_string().contains("nada.srma"));
}

#[test]
fn corrupt_pack_reports_e_pack_bad_magic() {
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("corrupt.srma");
    std::fs::write(&pack, b"PNG? no, nothing at all").unwrap();
    let (code, out) = sariema(&["inspect", arg(&pack)]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_PACK_BAD_MAGIC");
}

#[test]
fn bad_project_reports_e_schema() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("sariema.json"),
        r#"{ "name": "broken", "rooms": { "demo": "rooms/demo.json" } }"#,
    )
    .unwrap();
    let pack = tmp.path().join("demo.srma");
    let (code, out) = sariema(&["build", arg(tmp.path()), "--room", "demo", "-o", arg(&pack)]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_SCHEMA");
    assert!(!pack.exists());
}

#[test]
fn test_command_passes_then_fails_after_tamper() {
    let tmp = TempDir::new().unwrap();
    build_demo(tmp.path());
    let golden = tmp.path().join("demo.golden.png");
    let actual = tmp.path().join("demo.actual.png");

    let (code, out) = sariema(&["test", arg(tmp.path()), "--update"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["updated"], json!(["demo.srma"]));
    assert_eq!(image::image_dimensions(&golden).unwrap(), (960, 540));

    let (code, out) = sariema(&["test", arg(tmp.path())]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["passed"], json!(["demo.srma"]));
    assert!(!actual.exists());

    let mut reference = image::open(&golden).unwrap().to_rgba8();
    for y in 100..300 {
        for x in 300..500 {
            reference.put_pixel(x, y, image::Rgba([255, 0, 255, 255]));
        }
    }
    reference.save(&golden).unwrap();
    let (code, out) = sariema(&["test", arg(tmp.path())]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_GOLDEN_MISMATCH");
    assert!(out["error"]["message"].to_string().contains("demo.srma"));
    assert_eq!(image::image_dimensions(&actual).unwrap(), (960, 540));

    let (code, out) = sariema(&["test", arg(tmp.path()), "--update"]);
    assert_eq!(code, 0, "{out}");
    let (code, out) = sariema(&["test", arg(tmp.path())]);
    assert_eq!(code, 0, "{out}");
    assert!(!actual.exists(), "a pass leaves no stale render behind");
}

#[test]
fn reference_of_another_size_is_a_mismatch() {
    let tmp = TempDir::new().unwrap();
    build_demo(tmp.path());
    image::RgbaImage::new(100, 100)
        .save(tmp.path().join("demo.golden.png"))
        .unwrap();
    let (code, out) = sariema(&["test", arg(tmp.path())]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_GOLDEN_MISMATCH");
    assert!(
        out["error"]["message"].to_string().contains("100×100"),
        "{out}"
    );
    let actual = tmp.path().join("demo.actual.png");
    assert_eq!(image::image_dimensions(actual).unwrap(), (960, 540));
}

#[test]
fn test_command_needs_a_pack_with_a_reference() {
    let tmp = TempDir::new().unwrap();
    let (code, out) = sariema(&["test", arg(tmp.path())]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_USAGE");

    let pack = build_demo(tmp.path());
    let (code, out) = sariema(&["test", arg(tmp.path())]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_USAGE");

    let (code, out) = sariema(&["test", arg(tmp.path()), "--update"]);
    assert_eq!(code, 0, "{out}");
    std::fs::copy(&pack, tmp.path().join("other.srma")).unwrap();
    let (code, out) = sariema(&["test", arg(tmp.path())]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out["passed"], json!(["demo.srma"]));
    assert_eq!(out["skipped"], json!(["other.srma"]));
}

#[test]
fn update_writes_nothing_when_a_pack_fails() {
    let tmp = TempDir::new().unwrap();
    build_demo(tmp.path());
    std::fs::write(tmp.path().join("zz.srma"), b"not a pack").unwrap();
    let (code, out) = sariema(&["test", arg(tmp.path()), "--update"]);
    assert_eq!(code, 1);
    assert_eq!(error_code(&out), "E_PACK_BAD_MAGIC");
    assert!(!tmp.path().join("demo.golden.png").exists());
}

#[test]
fn json_mode_prints_nothing_but_one_json_object() {
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("demo.srma");
    let shot = tmp.path().join("shot.png");
    let demo = demo();
    let commands: [&[&str]; 7] = [
        &["build", arg(&demo), "--room", "demo", "-o", arg(&pack)],
        &["inspect", arg(&pack)],
        &[
            "run",
            arg(&pack),
            "--headless",
            "--frames",
            "1",
            "--screenshot",
            arg(&shot),
            "--width",
            "64",
            "--height",
            "36",
        ],
        &["test", arg(tmp.path())],
        &["inspect", "nada.srma"],
        &["run", "--help"],
        &["--version"],
    ];
    for args in commands {
        let out = cargo_bin_cmd!("sariema")
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let values: Vec<Value> = serde_json::Deserializer::from_slice(&out.stdout)
            .into_iter()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(values.len(), 1, "{args:?}: {values:?}");
        assert!(values[0].is_object(), "{args:?}: {values:?}");
    }
}
