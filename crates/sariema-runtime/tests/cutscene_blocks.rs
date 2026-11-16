// SPDX-License-Identifier: Apache-2.0

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use std::time::Duration;

use sariema_core::STEP_SECONDS;
use sariema_format::script::{Anchor, Command, Entity, Kind, name_id};
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

const START_AWAY: &str = r#"{ "name": "away", "roles": { "hero": { "start": { "position": [3, 0, 0] } } },
    "steps": [ { "show": "hero" }, { "call": "began" } ] }"#;

fn bind_and_play(out: &mut Vec<u8>) {
    send(
        out,
        Command::CutsceneBind {
            role: name_id("hero"),
            entity: entity(0),
        },
    );
    send(out, play("away"));
}

#[test]
fn an_actor_hidden_in_the_step_its_cutscene_starts_is_not_placed_at_once() {
    let cutscenes = vec![compile_json(START_AWAY).unwrap().0];
    let logic = native(|step, _, out| match step {
        0 => send(
            out,
            Command::Spawn {
                entity: entity(0),
                kind: Kind::Actor,
                asset: 0,
            },
        ),
        1 => {
            send(
                out,
                Command::SetVisible {
                    entity: entity(0),
                    visible: false,
                },
            );
            bind_and_play(out);
        }
        _ => {}
    });
    let settings = Settings::default();
    let actions = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, cutscenes, logic, 1);
    sim.step(&actions);
    sim.step(&actions);
    let x = sim.state().world.node(entity(0)).unwrap().local.position.x;
    assert!(x < 0.1, "it was on screen when the step began; x = {x}");
}

#[test]
fn a_start_walk_ends_on_a_reported_arrival_when_the_runtime_does_not_move_actors() {
    let cutscenes = vec![compile_json(START_AWAY).unwrap().0];
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    let logic = native(move |step, events, out| {
        log.borrow_mut()
            .extend(events.iter().map(|e| (step, format!("{e:?}"))));
        match step {
            0 => send(
                out,
                Command::Spawn {
                    entity: entity(0),
                    kind: Kind::Actor,
                    asset: 0,
                },
            ),
            1 => bind_and_play(out),
            _ => {}
        }
    });
    let mut settings = Settings::default();
    settings.motion.builtin = false;
    let actions = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, cutscenes, logic, 1);
    for step in 0..30 {
        if step == 10 {
            sim.report(sariema_format::script::Event::Arrived { entity: entity(0) });
        }
        sim.step(&actions);
    }
    drop(sim);
    let began = at(&calls.take(), &call("began"));
    assert_eq!(began.len(), 1, "the body started once placing ended");
    assert!(began[0] > 10, "not before the arrival: {began:?}");
}

const ROLE_AT_ORIGIN: &str = r#""roles": { "hero": { "start": { "position": [0, 0, 0] } } }"#;

/// A simulation whose game spawns actor 0 at the origin and, at step 1,
/// plays `cutscene` with it as `hero`.
fn playing(cutscene: String) -> Simulation<impl sariema_runtime::sim::Logic> {
    let compiled = compile_json(&cutscene).unwrap().0;
    let name = compiled.name.clone();
    let logic = native(move |step, _, out| match step {
        0 => send(
            out,
            Command::Spawn {
                entity: entity(0),
                kind: Kind::Actor,
                asset: 0,
            },
        ),
        1 => {
            send(
                out,
                Command::CutsceneBind {
                    role: name_id("hero"),
                    entity: entity(0),
                },
            );
            send(out, play(&name));
        }
        _ => {}
    });
    Simulation::new(Settings::default(), vec![compiled], logic, 1)
}

fn steps(sim: &mut Simulation<impl sariema_runtime::sim::Logic>, n: u32) {
    let actions = Actions::for_settings(&Settings::default().input);
    for _ in 0..n {
        sim.step(&actions);
    }
}

#[test]
fn a_cut_can_wait_for_a_clip_marker() {
    let cutscene = format!(
        r#"{{ "name": "beat", {ROLE_AT_ORIGIN}, "steps": [
            {{ "wait_for": {{ "clip_marker": {{ "role": "hero", "clip": "trip", "marker": "catch" }} }} }},
            {{ "shot": {{ "follow": "hero", "blend": 0 }} }},
            {{ "wait": 5 }} ] }}"#
    );
    let mut sim = playing(cutscene);
    let marker = |name: &str| sariema_format::script::Event::ClipMarker {
        entity: entity(0),
        clip: name_id("trip"),
        marker: name_id(name),
    };
    steps(&mut sim, 10);
    sim.report(marker("lift"));
    steps(&mut sim, 5);
    assert!(sim.state().camera.active().is_none(), "another marker");
    sim.report(marker("catch"));
    steps(&mut sim, 1);
    assert_eq!(
        sim.state().camera.active(),
        Some(sariema_runtime::cutscene::RIG),
        "the cut lands in the step the marker is delivered"
    );
}

#[test]
fn slow_motion_runs_fewer_steps_per_real_second_and_clips_follow_the_steps() {
    let cutscene = format!(
        r#"{{ "name": "slow", {ROLE_AT_ORIGIN}, "steps": [
            {{ "time_scale": {{ "scale": 0.4 }} }},
            {{ "clip": {{ "role": "hero", "clip": "trip", "speed": 0.5 }} }},
            {{ "wait": 1 }} ] }}"#
    );
    let mut sim = playing(cutscene);
    steps(&mut sim, 3);
    assert_eq!(sim.state().time_scale(), 0.4);
    assert_eq!(
        sim.state().clip_time(entity(0)),
        Some((name_id("trip"), 0.0))
    );
    let frame = Duration::from_nanos(16_666_667);
    let mut ran = 0;
    for _ in 0..100 {
        ran += sim.frame_with(frame, |_, _| {}).steps;
    }
    assert_eq!(ran, 40, "100 frames at 0.4× owe 40 steps; leftovers carry");
    let (_, t) = sim.state().clip_time(entity(0)).unwrap();
    assert!(
        (t - 40.0 * STEP_SECONDS * 0.5).abs() < 1e-5,
        "the clip is at {t} s, its steps' time"
    );
    for _ in 0..200 {
        sim.frame_with(frame, |_, _| {});
    }
    assert_eq!(
        sim.state().time_scale(),
        1.0,
        "the end of the cutscene gives back the scale it found"
    );
}

#[test]
fn a_shot_can_aim_at_a_bone_and_an_impact_can_shake_the_camera() {
    let cutscene = format!(
        r#"{{ "name": "close", {ROLE_AT_ORIGIN}, "steps": [
            {{ "shot": {{ "follow": "hero", "bone": "foot_l", "distance": 1.2, "blend": 0 }} }},
            {{ "camera_shake": {{ "amplitude": 0.05, "frequency": 14, "duration": 0.3 }} }},
            {{ "wait": 1 }} ] }}"#
    );
    let mut sim = playing(cutscene);
    steps(&mut sim, 3);
    let camera = &sim.state().camera;
    assert_eq!(
        camera.anchors().collect::<Vec<_>>(),
        [(entity(0), Anchor::Bone, name_id("foot_l"))]
    );
    assert!(camera.shaking());
}

#[test]
fn in_slow_motion_the_clip_time_drawn_grows_every_frame() {
    let cutscene = format!(
        r#"{{ "name": "slow", {ROLE_AT_ORIGIN}, "steps": [
            {{ "time_scale": {{ "scale": 0.4 }} }},
            {{ "clip": {{ "role": "hero", "clip": "trip", "speed": 0.5 }} }},
            {{ "wait": 1 }} ] }}"#
    );
    let mut sim = playing(cutscene);
    steps(&mut sim, 3);
    let frame = Duration::from_nanos(16_666_667);
    let mut drawn = Vec::new();
    for _ in 0..60 {
        sim.frame_with(frame, |_, _| {});
        let (_, t) = sim.state().clip_time_at(entity(0), sim.alpha()).unwrap();
        drawn.push(t);
    }
    let drawn = &drawn[5..];
    for pair in drawn.windows(2) {
        assert!(pair[1] > pair[0], "a still frame: {drawn:?}");
    }
    let (_, last) = sim.state().clip_time(entity(0)).unwrap();
    assert!(
        (drawn[drawn.len() - 1] - last).abs() <= STEP_SECONDS * 0.5,
        "drawn within a step of the clip's own time"
    );
}
