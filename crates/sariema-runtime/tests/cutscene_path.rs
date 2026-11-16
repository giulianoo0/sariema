// SPDX-License-Identifier: Apache-2.0

//! A cutscene's `path` step: the role's root moves only within the spans
//! of its clip, the clips after it follow in a row, and the step ends when
//! the last has played through; skipped, the role lands where playing
//! would have left it.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_core::STEP_SECONDS;
use sariema_format::script::{Command, Kind, name_id};
use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::cutscene::check::check;
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

const VAULT: f32 = 1.0;
const LAND: f32 = 0.5;

const OVER: &str = r#"{ "name": "over",
    "roles": { "hero": { "start": { "position": [0, 0, 0] } } },
    "steps": [
      { "path": { "role": "hero", "clip": "vault", "spans": [
          { "at": [0.2, 0.5], "to": [0, 0.9, -1] },
          { "at": [0.5, 0.6], "rate": 0.4 },
          { "at": [0.6, 0.8], "to": [0, 0, -2], "ease": "in_quad", "arc": 0.1 } ],
        "then": [ { "clip": "land" } ] } },
      { "call": "after" } ] }"#;

struct Played {
    roots: Vec<Vec3>,
    clips: Vec<Option<(u32, f32)>>,
    after: Option<u64>,
}

fn play(cutscene: &str, skip_at: Option<u64>, steps: u64) -> Played {
    let after = Rc::new(RefCell::new(None));
    let got = after.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if format!("{e:?}").contains(&format!("call: {} }}", name_id("after"))) {
                got.borrow_mut().get_or_insert(n);
            }
        }
        if n == 0 {
            send(
                out,
                Command::Spawn {
                    entity: entity(0),
                    kind: Kind::Actor,
                    asset: 0,
                },
            );
            send(
                out,
                Command::CutsceneBind {
                    role: name_id("hero"),
                    entity: entity(0),
                },
            );
            send(
                out,
                Command::CutscenePlay {
                    cutscene: name_id("over"),
                },
            );
        }
        if Some(n) == skip_at {
            send(out, Command::CutsceneSkip {});
        }
    });
    let cutscenes = vec![compile_json(cutscene).unwrap().0];
    let settings = Settings::default();
    let mut sim = Simulation::new(settings.clone(), cutscenes, logic, 1);
    sim.learn_clip(0, name_id("vault"), VAULT);
    sim.learn_clip(0, name_id("land"), LAND);
    let actions = Actions::for_settings(&settings.input);
    let (mut roots, mut clips) = (Vec::new(), Vec::new());
    for _ in 0..steps {
        sim.step(&actions);
        roots.push(sim.state().world.node(entity(0)).unwrap().local.position);
        clips.push(sim.state().clip_time(entity(0)));
    }
    let after = *after.borrow();
    Played {
        roots,
        clips,
        after,
    }
}

#[test]
fn a_path_step_moves_the_root_in_its_spans_and_ends_when_its_clips_are_through() {
    let p = play(OVER, None, 200);
    let started = (p.clips.iter()).position(|c| c.is_some_and(|c| c.0 == name_id("vault")));
    let started = started.expect("the vault plays");
    let held = (0.2 / STEP_SECONDS) as usize;
    assert!(
        p.roots[started..started + held]
            .iter()
            .all(|r| *r == Vec3::ZERO),
        "still while it anticipates"
    );
    let landed = p.roots[started + 60];
    assert!(
        landed.distance(Vec3::new(0.0, 0.0, -2.0)) < 1e-5,
        "{landed}"
    );
    let land = (p.clips.iter()).position(|c| c.is_some_and(|c| c.0 == name_id("land")));
    let land = land.expect("the land clip follows");
    let world = 0.5 + 0.1 / 0.4 + (VAULT - 0.6);
    assert_eq!(land - started, (world / STEP_SECONDS).ceil() as usize);
    let after = p.after.expect("the step ended") as usize;
    let through = land + (LAND / STEP_SECONDS).ceil() as usize;
    assert!(
        after.abs_diff(through + 1) <= 1,
        "the step ends once the last clip played through: {after} against {through}"
    );
}

#[test]
fn a_skipped_path_lands_where_playing_would_have() {
    let played = play(OVER, None, 200);
    let skipped = play(OVER, Some(20), 200);
    let ended = skipped.after.expect("the skip runs the rest") as usize;
    assert!(ended < 60, "under the skip's fade: {ended}");
    assert_eq!(skipped.roots[ended - 1], played.roots[199]);
    let (clip, t) = skipped.clips[ended - 1].unwrap();
    assert_eq!((clip, t >= LAND), (name_id("land"), true));
}

#[test]
fn a_path_too_fast_or_moving_a_role_another_branch_moves_fails_the_check() {
    let fast = r#"{ "name": "fast",
        "roles": { "hero": { "start": { "position": [0, 0, 0] } } },
        "steps": [ { "path": { "role": "hero", "clip": "vault", "spans": [
            { "at": [0.1, 0.2], "to": [0, 0, -1] },
            { "at": [0.2, 0.25], "to": [0, 0, -9] } ] } } ] }"#;
    let race = r#"{ "name": "race",
        "roles": { "hero": { "start": { "position": [0, 0, 0] } } },
        "steps": [ { "parallel": [
            [ { "path": { "role": "hero", "clip": "vault", "spans": [ { "at": [0.1, 0.5], "to": [0, 0, -1] } ] } } ],
            [ { "walk": { "role": "hero", "to": [1, 0, 0] } } ] ] } ] }"#;
    let settings = Settings::default();
    for (json, code) in [(fast, "E_CUTSCENE_SPEED"), (race, "E_CUTSCENE_RACE")] {
        let (cutscene, _) = compile_json(json).unwrap();
        let errors = check(&cutscene, &settings);
        assert!(errors.iter().any(|e| e.code == code), "{code}: {errors:?}");
    }
    let (over, _) = compile_json(OVER).unwrap();
    assert_eq!(check(&over, &settings), []);
}

#[test]
fn the_sill_trip_example_passes_the_rules() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cutscenes/sill_trip.json"
    );
    let (cutscene, _) = compile_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(check(&cutscene, &Settings::default()), []);
}
