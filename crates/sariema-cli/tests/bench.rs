// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::TempDir;

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

fn demo_pack(dir: &Path) -> PathBuf {
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo");
    let pack = dir.join("demo.srma");
    let (code, out) = sariema(&[
        "build",
        demo.to_str().unwrap(),
        "--room",
        "demo",
        "-o",
        pack.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{out}");
    pack
}

fn bench(pack: &Path, extra: &[&str]) -> Value {
    let mut args = vec![
        "bench",
        pack.to_str().unwrap(),
        "--frames",
        "6",
        "--warmup",
        "2",
        "--runs",
        "2",
        "--width",
        "320",
        "--height",
        "180",
    ];
    args.extend(extra);
    let (code, out) = sariema(&args);
    assert_eq!(code, 0, "{out}");
    out
}

#[test]
fn bench_reports_stages_counts_and_load() {
    let tmp = TempDir::new().unwrap();
    let out = bench(&demo_pack(tmp.path()), &[]);
    assert_eq!(out["ok"], true);
    assert_eq!(out["room"], "demo");
    for stage in ["frame", "cull", "bin", "record", "post", "finish", "submit"] {
        let s = &out["cpu_ms"][stage];
        let (p50, p90) = (s["median"].as_f64().unwrap(), s["p90"].as_f64().unwrap());
        assert!(p50 >= 0.0 && p90 >= p50, "{stage}: {s}");
    }
    assert_eq!(
        out["wall_ms_per_frame"]["runs"].as_array().unwrap().len(),
        2
    );
    let per_frame = &out["per_frame"];
    assert!(per_frame["draws"].as_u64().unwrap() > 0, "{per_frame}");
    assert!(per_frame["passes"].as_u64().unwrap() >= 2, "{per_frame}");
    assert!(
        per_frame["bytes_uploaded"].as_u64().unwrap() > 0,
        "{per_frame}"
    );
    assert!(per_frame["allocations"]["median"].is_u64(), "{per_frame}");
    for stage in [
        "read",
        "decode",
        "device",
        "pipelines",
        "upload",
        "first_frame_cpu",
    ] {
        assert!(out["load_ms"][stage].as_f64().unwrap() >= 0.0, "{stage}");
    }
    assert!(out["load_average"].is_array() || out["load_average"].is_null());
    let gpu = &out["gpu"];
    match gpu["timestamps"].as_str() {
        Some("unsupported") => assert!(gpu["passes"].is_null()),
        _ => {
            let passes = gpu["passes"].as_array().unwrap();
            assert_eq!(passes[0]["label"], "scene", "{gpu}");
            assert_eq!(passes.last().unwrap()["label"], "composite", "{gpu}");
            assert!(gpu["frame_ms"]["median"].as_f64().unwrap() > 0.0, "{gpu}");
        }
    }
}

#[test]
fn like_for_like_flags_turn_samples_shadows_and_occlusion_off() {
    let tmp = TempDir::new().unwrap();
    let pack = demo_pack(tmp.path());
    let full = bench(&pack, &["--quality", "high"]);
    let bare = bench(
        &pack,
        &[
            "--quality",
            "high",
            "--msaa",
            "1",
            "--no-shadows",
            "--no-ao",
        ],
    );
    assert_eq!(bare["settings"]["msaa"], 1);
    assert_eq!(bare["settings"]["shadows"], false);
    assert_eq!(bare["settings"]["ao"], false);
    assert!(
        full["settings"]["msaa"].as_u64().unwrap() > 1,
        "{}",
        full["settings"]
    );
    let passes = |v: &Value| v["per_frame"]["post_passes"].as_u64().unwrap();
    assert!(
        passes(&bare) < passes(&full),
        "the occlusion's passes are gone"
    );
}

#[test]
fn bench_errors_are_stable_codes() {
    let (code, out) = sariema(&["bench", "nada.srma"]);
    assert_eq!(code, 1);
    assert_eq!(out["error"]["code"], "E_IO");
    for args in [
        &["bench", "x.srma", "--frames", "0"][..],
        &["bench", "x.srma", "--runs", "0"],
        &["bench", "x.srma", "--msaa", "0"],
        &["bench", "x.srma", "--shadows"],
    ] {
        let (code, out) = sariema(args);
        assert_eq!(code, 1, "{args:?}");
        assert_eq!(out["error"]["code"], "E_USAGE", "{args:?}");
    }
}
