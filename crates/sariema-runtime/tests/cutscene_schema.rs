// SPDX-License-Identifier: Apache-2.0

use std::fs;
use std::path::{Path, PathBuf};

use sariema_format::script::{Advance, Bus, ClipMode, Ease, EnumSpec};
use sariema_runtime::cutscene::author::compile_json;
use serde_json::Value;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn schema() -> Value {
    serde_json::from_str(&fs::read_to_string(repo().join("schemas/cutscene.schema.json")).unwrap())
        .unwrap()
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn every_cutscene_in_the_repository_matches_the_schema() {
    let schema = schema();
    let mut files: Vec<PathBuf> =
        fs::read_dir(repo().join("crates/sariema-runtime/tests/fixtures/cutscenes"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
    files.push(repo().join("examples/hello-game/cutscenes/meet.json"));
    files.push(repo().join("crates/sariema-cli/templates/new/cutscenes/intro.json"));
    for file in files {
        if let Err(e) = jsonschema::validate(&schema, &json(&file)) {
            panic!("{}: {}: {e}", file.display(), e.instance_path());
        }
    }
}

#[test]
fn the_schema_and_the_compiler_refuse_the_same_mistakes() {
    let schema = schema();
    for text in [
        r#"{ "name": "a", "roles": {}, "steps": [ { "shot": { "fixed": { "position": [0,0,0], "look_at": [0,0,1] } } } ] }"#,
        r#"{ "name": "a", "roles": {}, "steps": [ { "jump": "hero" } ] }"#,
        r#"{ "name": "a", "roles": {}, "steps": [ { "wait_for": { "timeout": 1 } } ] }"#,
        r#"{ "name": "a", "roles": { "x": {} }, "steps": [ { "line": { "speaker": "x", "text": "hi", "advance": "auto" } } ] }"#,
        r#"{ "name": "a", "roles": {}, "steps": [ { "shot": { "fixed": { "position": [0,0,0], "look_at": [0,0,1] }, "blend": 0, "ease": "bouncy" } } ] }"#,
        r#"{ "name": "a", "roles": { "x": {} }, "steps": [ { "wait_for": { "clip_marker": { "role": "x", "clip": "c" } } } ] }"#,
        r#"{ "name": "a", "roles": { "x": {} }, "steps": [ { "shot": { "frame": ["x", "x"], "bone": "head", "blend": 0 } } ] }"#,
        r#"{ "name": "a", "roles": {}, "steps": [ { "time_scale": { "blend": 1 } } ] }"#,
        r#"{ "name": "a", "roles": {}, "steps": [ { "shot": { "fixed": { "position": [0,0,0], "look_at": [0,0,1] }, "socket": "hand", "blend": 0 } } ] }"#,
        r#"{ "name": "a", "roles": {}, "steps": [ { "scene": { "duration": 1 } } ] }"#,
        r#"{ "name": "a", "roles": {}, "steps": [ { "scene": { "set": { "look.exposure": 2 }, "duration": 1, "ease": "bouncy" } } ] }"#,
        r#"{ "name": "a", "roles": {}, "steps": [ { "scene": { "set": { "look.exposure": "bright" }, "duration": 1 } } ] }"#,
        r#"{ "name": "a", "roles": {}, "steps": [ { "scene": { "set": { "look.exposure": 2 }, "duration": -1 } } ] }"#,
    ] {
        let value: Value = serde_json::from_str(text).unwrap();
        assert!(
            jsonschema::validate(&schema, &value).is_err(),
            "schema took {text}"
        );
        assert!(compile_json(text).is_err(), "compiler took {text}");
    }
}

fn names(spec: &EnumSpec) -> Vec<&str> {
    spec.variants.iter().map(|v| v.name).collect()
}

#[test]
fn the_schemas_enums_are_the_tables() {
    let schema = schema();
    let step = |key: &str| -> Value {
        schema["$defs"]["step"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|s| s["properties"].get(key).cloned())
            .unwrap()
    };
    let list = |v: &Value| -> Vec<String> {
        v["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(list(&schema["$defs"]["ease"]), names(&Ease::SPEC));
    assert_eq!(
        list(&step("clip")["properties"]["mode"]),
        names(&ClipMode::SPEC)
    );
    assert_eq!(list(&step("sound")["properties"]["bus"]), names(&Bus::SPEC));
    assert_eq!(
        list(&step("line")["properties"]["advance"]),
        names(&Advance::SPEC)
    );
}

#[test]
fn a_scene_step_names_its_parameters_and_eases_in_and_out_by_default() {
    use sariema_format::script::cutscene::Block;
    use sariema_format::script::name_id;
    let text = r#"{ "name": "dusk", "roles": {}, "steps": [
        { "scene": { "set": { "look.exposure": 0.6, "fog.density": 0.05 }, "duration": 2 } } ] }"#;
    let value: Value = serde_json::from_str(text).unwrap();
    jsonschema::validate(&schema(), &value).unwrap();
    let (cutscene, names) = compile_json(text).unwrap();
    assert_eq!(
        cutscene.body,
        [Block::Scene {
            values: vec![
                (name_id("fog.density"), 0.05),
                (name_id("look.exposure"), 0.6)
            ],
            duration: 2.0,
            ease: Ease::InOutSine as u8,
        }]
    );
    assert!(names.iter().any(|n| n == "look.exposure"), "{names:?}");
}
