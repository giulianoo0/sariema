// SPDX-License-Identifier: Apache-2.0

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use sariema_render::{GOLDEN_MAX_RATIO, compare_images};
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

fn repo(path: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
        .to_str()
        .unwrap()
        .to_string()
}

/// Golden name, description and the arguments after it.
const GOLDENS: [(&str, &str, &[&str]); 5] = [
    ("ui_dialogue", "dialogue", &[]),
    (
        "ui_objective",
        "objective",
        &["--state", "play=done", "--time", "1"],
    ),
    (
        "ui_objective_mid",
        "objective",
        &["--state", "play=done", "--time", "0.3"],
    ),
    (
        "ui_controls",
        "controls",
        &[
            "--state",
            "down=1:150,280",
            "--state",
            "move=1:175,262",
            "--state",
            "down=2:788,334",
            "--time",
            "0.6",
        ],
    ),
    ("ui_gradients", "gradients", &[]),
];

fn render_golden(name: &str, desc: &str, extra: &[&str], out: &Path) -> Value {
    let desc = repo(&format!("examples/ui/{desc}.json"));
    let mut args = vec![
        "ui",
        "render",
        &desc,
        "--width",
        "844",
        "--height",
        "390",
        "--background",
        "#5B7A99",
        "-o",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let (code, v) = sariema(&args);
    assert_eq!(code, 0, "{name}: {v}");
    v
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(repo(&format!("tests/golden/{name}.png")))
}

#[test]
fn renders_match_their_goldens() {
    let tmp = TempDir::new().unwrap();
    for (name, desc, extra) in GOLDENS {
        let out = tmp.path().join(format!("{name}.png"));
        let v = render_golden(name, desc, extra, &out);
        assert_eq!(v["draw_calls"], 1, "{name}: {v}");
        let actual = image::open(&out).unwrap().to_rgba8();
        let expected = image::open(golden(name)).unwrap().to_rgba8();
        let d = compare_images(&actual, &expected).unwrap();
        assert!(
            d.differing_pixels_ratio <= GOLDEN_MAX_RATIO,
            "{name}: {d:?}"
        );
    }
}

#[test]
#[ignore = "rewrites tests/golden/ui_*.png"]
fn write_goldens() {
    for (name, desc, extra) in GOLDENS {
        render_golden(name, desc, extra, &golden(name));
    }
}

#[test]
fn reports_what_happened_and_films_a_strip() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("strip.png");
    let (code, v) = sariema(&[
        "ui",
        "render",
        &repo("examples/ui/controls.json"),
        "--width",
        "844",
        "--height",
        "390",
        "--state",
        "down=2:788,334",
        "--time",
        "1.3",
        "--filmstrip",
        "4",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(
        (v["width"].as_u64(), v["height"].as_u64()),
        (Some(4 * 844), Some(390))
    );
    assert_eq!(v["frames"], 4);
    assert_eq!(v["steps"], 78);
    let events: Vec<(&str, &str)> = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["event"].as_str().unwrap(), e["node"].as_str().unwrap()))
        .collect();
    assert_eq!(events, [("press", "action"), ("held", "action")]);
    let held = &v["events"][1];
    assert_eq!(held["step"], 72);
    assert_eq!(image::open(&out).unwrap().width(), 4 * 844);
}

#[test]
fn missing_glyphs_are_listed_by_node_name() {
    let tmp = TempDir::new().unwrap();
    let desc = tmp.path().join("cjk.json");
    let font = repo("crates/sariema-ui/fonts/Nunito.ttf");
    let texts: Vec<String> = ["f", "e", "d", "c", "b", "a"]
        .iter()
        .map(|n| format!(r#"{{ "name": "{n}", "text": "漢字" }}"#))
        .collect();
    fs::write(
        &desc,
        format!(
            r#"{{ "fonts": {{ "body": {{ "file": {font:?} }} }}, "nodes": [ {} ] }}"#,
            texts.join(", ")
        ),
    )
    .unwrap();
    let out = tmp.path().join("cjk.png");
    let (code, v) = sariema(&[
        "ui",
        "render",
        desc.to_str().unwrap(),
        "--width",
        "200",
        "--height",
        "100",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{v}");
    let names: Vec<&str> = v["missing_glyphs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["node"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["a", "b", "c", "d", "e", "f"]);
}

#[test]
fn errors_name_their_cause() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("x.png");
    let out = out.to_str().unwrap();
    let objective = repo("examples/ui/objective.json");
    let base = [
        "ui", "render", &objective, "--width", "844", "--height", "390", "-o", out,
    ];
    let with = |extra: &[&str]| {
        let mut a: Vec<&str> = base.to_vec();
        a.extend_from_slice(extra);
        sariema(&a)
    };
    let code_of = |(code, v): (i32, Value)| {
        assert_eq!(code, 1, "{v}");
        assert_eq!(v["ok"], false);
        v["error"]["code"].as_str().unwrap().to_string()
    };
    assert_eq!(code_of(with(&["--state", "nope.x=1"])), "E_UI_UNKNOWN");
    assert_eq!(code_of(with(&["--state", "play=nope"])), "E_UI_UNKNOWN");
    assert_eq!(code_of(with(&["--state", "phrase.colour=1"])), "E_USAGE");
    assert_eq!(code_of(with(&["--state", "phrase.x=far"])), "E_USAGE");
    assert_eq!(code_of(with(&["--frobnicate"])), "E_USAGE");
    let bad = tmp.path().join("bad.json");
    fs::write(&bad, r##"{ "nodes": [ { "colour": "#fff" } ] }"##).unwrap();
    let (code, v) = sariema(&[
        "ui",
        "render",
        bad.to_str().unwrap(),
        "--width",
        "10",
        "--height",
        "10",
        "-o",
        out,
    ]);
    assert_eq!(code_of((code, v.clone())), "E_SCHEMA", "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("/nodes/0"));
    for gradient in [
        r##"{ "type": "conic", "from": "#fff", "to": "#000" }"##,
        r##"{ "type": "radial", "radius": [0, 1], "from": "#fff", "to": "#000" }"##,
    ] {
        let desc = format!(r#"{{ "nodes": [ {{ "gradient": {gradient} }} ] }}"#);
        fs::write(&bad, desc).unwrap();
        let (code, v) = sariema(&[
            "ui",
            "render",
            bad.to_str().unwrap(),
            "--width",
            "10",
            "--height",
            "10",
            "-o",
            out,
        ]);
        assert_eq!(code_of((code, v.clone())), "E_SCHEMA", "{gradient}: {v}");
    }
    fs::write(&bad, r#"{ "fonts": { "f": { "file": "missing.ttf" } } }"#).unwrap();
    let (code, v) = sariema(&[
        "ui",
        "render",
        bad.to_str().unwrap(),
        "--width",
        "10",
        "--height",
        "10",
        "-o",
        out,
    ]);
    assert_eq!(code_of((code, v)), "E_IO");
}
