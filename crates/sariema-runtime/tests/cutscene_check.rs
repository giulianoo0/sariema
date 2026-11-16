// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::cutscene::check::{CheckError, check};

fn errors(fixture: &str) -> Vec<CheckError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cutscenes")
        .join(fixture);
    let (cutscene, _) = compile_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    check(&cutscene)
}

fn codes(fixture: &str) -> Vec<&'static str> {
    errors(fixture).iter().map(|e| e.code).collect()
}

#[test]
fn a_teleport_of_an_on_screen_actor_fails_naming_cutscene_and_step() {
    let e = errors("teleport_visible.json");
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].code, "E_CUTSCENE_TELEPORT");
    assert_eq!(e[0].cutscene, "jump");
    assert_eq!(e[0].at, "steps[1].teleport");
    assert!(
        e[0].to_string()
            .starts_with("cutscene jump, steps[1].teleport: role hero"),
        "{}",
        e[0]
    );
}

#[test]
fn the_same_teleport_wrapped_in_a_full_fade_passes() {
    assert_eq!(codes("teleport_under_fade.json"), Vec::<&str>::new());
}

#[test]
fn a_declared_cover_must_hold() {
    assert_eq!(codes("teleport_half_fade.json"), ["E_CUTSCENE_COVER"]);
    assert_eq!(codes("teleport_after_blend.json"), ["E_CUTSCENE_COVER"]);
    assert!(codes("teleport_on_cut.json").is_empty());
    assert_eq!(
        codes("fade_in_other_branch.json"),
        ["E_CUTSCENE_COVER"],
        "another branch may lift the fade"
    );
}

#[test]
fn a_hidden_actor_may_be_placed_anywhere() {
    assert!(codes("teleport_hidden.json").is_empty());
}

#[test]
fn hiding_covers_a_teleport_only_once_a_step_has_passed_hidden() {
    let e = errors("teleport_hidden_same_instant.json");
    assert_eq!(
        e.iter()
            .map(|e| (e.code, e.at.as_str()))
            .collect::<Vec<_>>(),
        [("E_CUTSCENE_TELEPORT", "steps[1].teleport")]
    );
    assert!(
        e[0].to_string().contains("wait"),
        "the message says how to fix it: {}",
        e[0]
    );
    assert_eq!(
        codes("teleport_hidden_in_branch.json"),
        Vec::<&str>::new(),
        "a parallel waits as long as its longest branch; teleporting again needs no new wait"
    );
}

#[test]
fn every_role_declares_its_start_unless_spawned() {
    let e = errors("no_start.json");
    assert_eq!(e.len(), 1);
    assert_eq!(
        (e[0].code, e[0].at.as_str()),
        ("E_CUTSCENE_START", "steps[0].walk")
    );
    assert!(codes("spawned.json").is_empty());
}

#[test]
fn parallel_branches_must_not_move_the_same_role() {
    assert_eq!(codes("race.json"), ["E_CUTSCENE_RACE"]);
}

#[test]
fn unknown_roles_and_missing_choices_are_compile_errors() {
    let bad = r#"{"name":"x","roles":{"hero":{"start":{"position":[0,0,0]}}},"steps":[{"walk":{"role":"heroo","to":[0,0,0]}}]}"#;
    let e = compile_json(bad).unwrap_err();
    assert_eq!(
        (e.code, e.at.as_str()),
        ("E_CUTSCENE_ROLE", "steps[0].walk")
    );
    assert!(e.message.contains("hero"));
    let bad = r#"{"name":"x","roles":{},"steps":[{"shot":{"blend":0}}]}"#;
    assert_eq!(compile_json(bad).unwrap_err().code, "E_SCHEMA");
    let bad = r#"{"name":"x","roles":{},"steps":[{"shot":{"fixed":{"position":[0,0,0],"look_at":[0,0,1]}}}]}"#;
    assert_eq!(
        compile_json(bad).unwrap_err().code,
        "E_SCHEMA",
        "a shot must say its blend"
    );
    let bad = r#"{"name":"x","roles":{},"steps":[{"teleprot":{}}]}"#;
    assert_eq!(compile_json(bad).unwrap_err().code, "E_SCHEMA");
}
