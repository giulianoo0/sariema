// SPDX-License-Identifier: Apache-2.0

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_format::script::{Command, Entity, Kind, name_id};
use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

type Script = Box<dyn FnMut(u64, &mut Vec<u8>)>;

/// Runs `steps` steps of a game that spawns actor 0 at the origin and then
/// follows `script`, with the cutscenes given as JSON. Returns every event
/// the game saw, as `step: debug`.
fn run(cutscenes: &[&str], steps: u64, mut script: Script) -> Vec<(u64, String)> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let logic = native(move |step, events, out| {
        log.borrow_mut()
            .extend(events.iter().map(|e| (step, format!("{e:?}"))));
        if step == 0 {
            send(
                out,
                Command::Spawn {
                    entity: entity(0),
                    kind: Kind::Actor,
                    asset: 0,
                },
            );
        }
        script(step, out);
    });
    let cutscenes = cutscenes
        .iter()
        .map(|c| compile_json(c).unwrap().0)
        .collect();
    let settings = Settings::default();
    let mut sim = Simulation::new(settings.clone(), cutscenes, logic, 1);
    let actions = Actions::for_settings(&settings.input);
    for _ in 0..steps {
        sim.step(&actions);
    }
    seen.take()
}

fn at(events: &[(u64, String)], needle: &str) -> Vec<u64> {
    events
        .iter()
        .filter(|(_, e)| e.contains(needle))
        .map(|(s, _)| *s)
        .collect()
}

fn call(name: &str) -> String {
    format!("call: {} }}", name_id(name))
}

fn play(cutscene: &str) -> Command<'static> {
    Command::CutscenePlay {
        cutscene: name_id(cutscene),
    }
}

const WAIT_FOR: &str = r#"{ "name": "w", "roles": {}, "steps": [
    { "wait_for": { "signal": "go", "timeout": 1 } }, { "call": "after" } ] }"#;

#[test]
fn wait_for_ends_on_a_later_signal_or_its_timeout_never_an_earlier_signal() {
    let signal = Command::Signal {
        signal: name_id("go"),
    };
    let early = run(
        &[WAIT_FOR],
        200,
        Box::new(move |step, out| match step {
            1 => send(out, signal),
            2 => send(out, play("w")),
            _ => {}
        }),
    );
    let timed_out = at(&early, &call("after"));
    assert_eq!(timed_out.len(), 1);
    assert!(
        (61..=64).contains(&timed_out[0]),
        "a signal before the wait does not count; the 1 s timeout ends it: {timed_out:?}"
    );

    let late = run(
        &[WAIT_FOR],
        200,
        Box::new(move |step, out| match step {
            2 => send(out, play("w")),
            20 => send(out, signal),
            _ => {}
        }),
    );
    let released = at(&late, &call("after"));
    assert!(
        released.len() == 1 && (20..=22).contains(&released[0]),
        "{released:?}"
    );
}

#[test]
fn a_parallel_block_ends_when_its_longest_branch_does() {
    let parallel = r#"{ "name": "p", "roles": {}, "steps": [
        { "parallel": [ [ { "wait": 0.5 }, { "call": "short" } ], [ { "wait": 1.5 } ], [] ] },
        { "call": "after" } ] }"#;
    let events = run(
        &[parallel],
        200,
        Box::new(|step, out| {
            if step == 1 {
                send(out, play("p"))
            }
        }),
    );
    let short = at(&events, &call("short"))[0];
    let after = at(&events, &call("after"))[0];
    assert_eq!(
        after - short,
        60,
        "1.5 s after the start, 1 s after the short branch"
    );
}

#[test]
fn refused_plays_say_why_and_change_nothing() {
    let needs_hero = r#"{ "name": "h", "roles": { "hero": { "start": { "position": [0, 0, 0] } } },
        "steps": [ { "wait": 1 } ] }"#;
    let events = run(
        &[needs_hero, WAIT_FOR],
        40,
        Box::new(|step, out| match step {
            1 => send(out, play("nowhere")),
            2 => send(out, play("h")),
            3 => send(out, play("w")),
            4 => send(out, play("w")),
            _ => {}
        }),
    );
    let rejected: Vec<(u64, &str)> = events
        .iter()
        .filter(|(_, e)| e.starts_with("Rejected"))
        .map(|(s, e)| (*s, e.split('"').nth(1).unwrap()))
        .collect();
    assert_eq!(
        rejected,
        [
            (2, "E_CUTSCENE_UNKNOWN"),
            (3, "E_CUTSCENE_UNBOUND"),
            (5, "E_CUTSCENE_BUSY")
        ]
    );
    assert_eq!(at(&events, "CutsceneStarted").len(), 1);
}

#[test]
fn a_role_without_a_start_is_spawned_then_despawned_by_the_cutscene() {
    let ghost = Entity {
        index: 7,
        generation: 0,
    };
    let appear = r#"{ "name": "a", "roles": { "ghost": { "actor": "ghost" } }, "steps": [
        { "fade": { "to": 1, "duration": 0.2 } },
        { "spawn": { "role": "ghost", "position": [2, 0, -1], "yaw": 90 } },
        { "fade": { "to": 0, "duration": 0.2 } },
        { "wait": 0.5 },
        { "despawn": "ghost" } ] }"#;
    let events = run(
        &[appear],
        120,
        Box::new(move |step, out| {
            if step == 1 {
                send(
                    out,
                    Command::CutsceneBind {
                        role: name_id("ghost"),
                        entity: ghost,
                    },
                );
                send(out, play("a"));
            }
        }),
    );
    let despawned = at(
        &events,
        "Despawned { entity: Entity { index: 7, generation: 0 } }",
    );
    assert_eq!(despawned.len(), 1, "{events:?}");
    assert!(at(&events, "CutsceneEnded").contains(&despawned[0]));
    assert!(
        !events.iter().any(|(_, e)| e.starts_with("Rejected")),
        "{events:?}"
    );
}
