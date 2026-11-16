// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::TempDir;

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../examples/{name}"))
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

fn manifest(dir: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(dir.join("sariema-web.json")).unwrap()).unwrap()
}

#[test]
fn a_project_becomes_a_folder_the_web_package_loads() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("game");
    let (code, done) = sariema(&[
        "web",
        example("demo").to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{done}");
    let m = manifest(&out);
    assert_eq!(m["start"], "demo");
    assert_eq!(
        m["rooms"]["demo"],
        json!({ "astc": "demo.srma", "rgba8": "demo.rgba8.srma" })
    );
    assert_eq!(
        m["actors"]["arm"],
        json!({ "astc": "arm.srmk", "rgba8": "arm.rgba8.srmk" })
    );
    assert!(m.get("sounds").is_none(), "{m}");
    for file in ["demo.srma", "demo.rgba8.srma", "arm.srmk", "arm.rgba8.srmk"] {
        let bytes = std::fs::metadata(out.join(file)).unwrap().len();
        assert!(bytes > 0, "{file}");
    }
    assert_eq!(
        done["rooms"]["demo"]["rgba8"]["bytes"],
        std::fs::metadata(out.join("demo.rgba8.srma"))
            .unwrap()
            .len()
    );
}

#[test]
fn a_project_with_audio_gets_its_sound_bank() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("game");
    let (code, done) = sariema(&[
        "web",
        example("audio").to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{done}");
    let m = manifest(&out);
    assert_eq!(m["sounds"], "sounds.srms");
    assert_eq!(m["rooms"], json!({}));
    assert!(m.get("start").is_none(), "{m}");
    assert!(std::fs::metadata(out.join("sounds.srms")).unwrap().len() > 0);
}

#[test]
fn a_start_room_outside_the_build_is_refused() {
    let tmp = TempDir::new().unwrap();
    let (code, done) = sariema(&[
        "web",
        example("demo").to_str().unwrap(),
        "-o",
        tmp.path().to_str().unwrap(),
        "--start",
        "elsewhere",
    ]);
    assert_eq!(code, 1, "{done}");
    assert_eq!(done["error"]["code"], "E_USAGE");
}
