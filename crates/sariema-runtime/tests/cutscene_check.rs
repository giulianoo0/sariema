// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::cutscene::check::{CheckError, check};
use sariema_runtime::settings::Settings;

fn errors(fixture: &str) -> Vec<CheckError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cutscenes")
        .join(fixture);
    let (cutscene, _) = compile_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    check(&cutscene, &Settings::default())
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
fn the_window_entry_example_passes_the_rules() {
    assert_eq!(codes("window_entry.json"), Vec::<&str>::new());
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

#[test]
fn a_hinge_step_names_a_cast_role_and_a_span_that_ends_after_it_starts() {
    let step = |role: &str, span: &str| {
        format!(
            r#"{{"name":"x","roles":{{"hero":{{"start":{{"position":[0,0,0]}}}}}},"steps":[{{"hinge":{{"node":"a.door_leaf","role":"{role}","clip":"push","span":{span},"to":90}}}}]}}"#
        )
    };
    let (cutscene, _) = compile_json(&step("hero", "[0.2, 0.6]")).unwrap();
    assert_eq!(check(&cutscene, &Settings::default()), Vec::new());
    let e = compile_json(&step("heroo", "[0.2, 0.6]")).unwrap_err();
    assert_eq!(
        (e.code, e.at.as_str()),
        ("E_CUTSCENE_ROLE", "steps[0].hinge")
    );
    for span in ["[0.6, 0.2]", "[-0.1, 0.2]"] {
        let e = compile_json(&step("hero", span)).unwrap_err();
        assert_eq!(
            (e.code, e.at.as_str()),
            ("E_SCHEMA", "steps[0].hinge"),
            "{span}"
        );
    }
}

#[test]
fn a_hinge_step_must_name_a_hinged_node_of_some_room() {
    use sariema_format::script::name_id;
    use sariema_runtime::cutscene::check::check_hinges;
    let text = r#"{"name":"x","roles":{"hero":{"start":{"position":[0,0,0]}}},"steps":[
        {"clip":{"role":"hero","clip":"push"}},
        {"parallel":[[{"wait":0.1}],[{"hinge":{"node":"a.door_lef","role":"hero","clip":"push","span":[0.2,0.6],"to":90}}]]}]}"#;
    let (cutscene, names) = compile_json(text).unwrap();
    let rooms = |id: u32| id == name_id("hinge.a.door_leaf");
    let e = check_hinges(&cutscene, &rooms, &names);
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(
        (e[0].code, e[0].at.as_str()),
        ("E_CUTSCENE_HINGE", "steps[1].parallel[1][0].hinge")
    );
    assert!(e[0].message.contains("a.door_lef"), "{}", e[0]);
    let fixed = text.replace("door_lef\"", "door_leaf\"");
    let (cutscene, names) = compile_json(&fixed).unwrap();
    assert_eq!(check_hinges(&cutscene, &rooms, &names), Vec::new());
}

#[test]
fn parallel_branches_must_not_tween_the_same_scene_parameter() {
    let text = r#"{ "name": "dusk", "roles": {}, "steps": [ { "parallel": [
        [ { "scene": { "set": { "look.exposure": 2, "fog.density": 0.1 }, "duration": 2 } } ],
        [ { "wait": 1 }, { "scene": { "set": { "look.exposure": 3 }, "duration": 1 } } ],
        [ { "scene": { "set": { "look.tint.r": 0.9 }, "duration": 1 } } ] ] } ] }"#;
    let (cutscene, _) = compile_json(text).unwrap();
    let e = check(&cutscene, &Settings::default());
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].code, "E_CUTSCENE_RACE");
    assert!(e[0].message.contains("branches 0 and 1"), "{}", e[0]);
}
