// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::TempDir;

#[test]
#[ignore = "opens a window"]
fn window_runs_frames_and_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("demo.srma");
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo");
    cargo_bin_cmd!("sariema")
        .arg("build")
        .arg(&demo)
        .args(["--room", "demo", "-o"])
        .arg(&pack)
        .assert()
        .success();
    let out = cargo_bin_cmd!("sariema")
        .arg("run")
        .arg(&pack)
        .args([
            "--frames", "30", "--width", "320", "--height", "180", "--json",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    let value: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["frames"], 30, "{value}");
    assert!(value["ms_per_frame"].as_f64().unwrap() > 0.0, "{value}");
    // The composite pass always runs.
    assert!(
        value["frame_stats"]["post_passes"].as_u64().unwrap() >= 1,
        "{value}"
    );
}
