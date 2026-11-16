// SPDX-License-Identifier: Apache-2.0

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use std::time::Duration;

use glam::Vec3;

use sariema_core::STEP_SECONDS;
use sariema_format::script::{Anchor, Command, Entity, Kind, Property, name_id};
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
    let held = drawn.iter().take_while(|&&t| t == 0.0).count();
    assert!(
        held >= 1 && held as f32 <= 1.0 / 0.4,
        "the clip shows its first frame until the step after it began, and no longer: {drawn:?}"
    );
    let drawn = &drawn[held - 1..];
    for pair in drawn.windows(2) {
        assert!(pair[1] > pair[0], "a still frame: {drawn:?}");
    }
    let (_, last) = sim.state().clip_time(entity(0)).unwrap();
    assert!(
        (drawn[drawn.len() - 1] - last).abs() <= STEP_SECONDS * 0.5,
        "drawn within a step of the clip's own time"
    );
}

const DASH: &str = r#"{ "name": "dash", "roles": { "hero": { "start": { "position": [0, 0, 0] } } },
    "steps": [ { "walk": { "role": "hero", "to": [10, 0, 0], "speed": 30 } }, { "call": "after" } ] }"#;

/// Plays `cutscene` from step 1 with actor 0, spawned at the origin, as
/// `hero`, its speed scaled by `scale`; skips it at step `skip`, and adds
/// `extra(step, out)` to the game's commands. The events seen and where the
/// hero ends.
fn hero_plays(
    cutscene: &str,
    builtin: bool,
    scale: f32,
    skip: Option<u64>,
    extra: fn(u64, &mut Vec<u8>),
) -> (Vec<(u64, String)>, Vec3) {
    let mut settings = Settings::default();
    settings.motion.builtin = builtin;
    hero_plays_with(settings, cutscene, scale, skip, extra)
}

fn hero_plays_with(
    settings: Settings,
    cutscene: &str,
    scale: f32,
    skip: Option<u64>,
    extra: fn(u64, &mut Vec<u8>),
) -> (Vec<(u64, String)>, Vec3) {
    let compiled = compile_json(cutscene).unwrap().0;
    let name = compiled.name.clone();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
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
            1 => {
                send(
                    out,
                    Command::SetSpeedScale {
                        entity: entity(0),
                        scale,
                    },
                );
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
        }
        if skip == Some(step) {
            send(out, Command::CutsceneSkip {});
        }
        extra(step, out);
    });
    let actions = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![compiled], logic, 1);
    for _ in 0..400 {
        sim.step(&actions);
    }
    let at = sim.state().world.current_pose(entity(0)).unwrap().position;
    drop(sim);
    (seen.take(), at)
}

fn nothing(_: u64, _: &mut Vec<u8>) {}

#[test]
fn a_walk_too_fast_to_be_seen_is_refused_and_ends_alike_played_or_skipped() {
    let mut wrong = Vec::new();
    for builtin in [true, false] {
        for skip in [false, true] {
            let (events, end) = hero_plays(DASH, builtin, 1.0, skip.then_some(1), nothing);
            let x = end.x;
            let case = format!("builtin {builtin}, skip {skip}");
            if at(&events, "E_TELEPORT_VISIBLE").is_empty() {
                wrong.push(format!("{case}: not refused"));
            }
            if at(&events, &call("after")).len() != 1 {
                wrong.push(format!("{case}: the walk never ended"));
            }
            if at(&events, "CutsceneEnded").len() != 1 {
                wrong.push(format!("{case}: the cutscene never ended"));
            }
            if x != 0.0 {
                wrong.push(format!("{case}: the role went to x {x}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

const BRISK: &str = r#"{ "name": "brisk", "roles": { "hero": { "start": { "position": [0, 0, 0] } } },
    "steps": [ { "walk": { "role": "hero", "to": [10, 0, 0], "speed": 15 } }, { "call": "after" } ] }"#;

#[test]
fn a_walk_scaled_past_the_visible_speed_is_refused_alike_played_or_skipped() {
    let mut wrong = Vec::new();
    for builtin in [true, false] {
        for skip in [None, Some(1)] {
            let (events, end) = hero_plays(BRISK, builtin, 2.0, skip, nothing);
            let case = format!("builtin {builtin}, skip {skip:?}");
            if at(&events, "E_TELEPORT_VISIBLE").len() != 1 {
                wrong.push(format!("{case}: not refused once"));
            }
            if at(&events, &call("after")).len() != 1 {
                wrong.push(format!("{case}: the walk never ended"));
            }
            if end.x != 0.0 {
                wrong.push(format!("{case}: the role went to x {}", end.x));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_start_walk_slows_to_what_may_be_seen_and_ends_alike_played_or_skipped() {
    let mut wrong = Vec::new();
    for skip in [None, Some(3)] {
        let (events, end) = hero_plays(START_AWAY, true, 20.0, skip, nothing);
        let case = format!("skip {skip:?}");
        if !at(&events, "Rejected").is_empty() {
            wrong.push(format!("{case}: refused at {:?}", at(&events, "Rejected")));
        }
        if at(&events, &call("began")).len() != 1 {
            wrong.push(format!("{case}: the body never began"));
        }
        if end.distance(Vec3::new(3.0, 0.0, 0.0)) > 1e-5 {
            wrong.push(format!("{case}: the hero ended at {end}, not at its start"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_start_walk_slowed_by_the_speed_scale_is_never_refused_for_rounding() {
    let mut wrong = Vec::new();
    let cases: [(f32, &[f32]); 2] = [
        (1.5, &[16.89, 17.03, 17.45]),
        (10.0, &[2.09, 2.22, 2.25, 2.45]),
    ];
    for (start_walk_speed, scales) in cases {
        for &scale in scales {
            let mut settings = Settings::default();
            settings.cutscene.start_walk_speed = start_walk_speed;
            let (events, end) = hero_plays_with(settings, START_AWAY, scale, None, nothing);
            let case = format!("start_walk_speed {start_walk_speed}, scale {scale}");
            if !at(&events, "Rejected").is_empty() {
                wrong.push(format!("{case}: refused at {:?}", at(&events, "Rejected")));
            }
            if end.distance(Vec3::new(3.0, 0.0, 0.0)) > 1e-5 {
                wrong.push(format!("{case}: the hero ended at {end}, not at its start"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

const STROLL: &str = r#"{ "name": "stroll", "roles": { "hero": { "start": { "position": [0, 0, 0] } } },
    "steps": [ { "walk": { "role": "hero", "to": [3, 0, 0], "speed": 1 } }, { "call": "after" } ] }"#;

/// Spins the hero at step 30, mid-walk, faster than it may be seen to turn.
fn spin(step: u64, out: &mut Vec<u8>) {
    if step == 30 {
        send(
            out,
            Command::Spring {
                tween: 9,
                entity: entity(0),
                property: Property::Yaw,
                var: 0,
                to: 170.0,
                frequency: 40.0,
                damping: 1.0,
            },
        );
    }
}

#[test]
fn a_role_put_back_mid_walk_ends_its_walk_in_both_modes() {
    let mut wrong = Vec::new();
    for builtin in [true, false] {
        for skip in [None, Some(1)] {
            let (events, _) = hero_plays(STROLL, builtin, 1.0, skip, spin);
            let case = format!("builtin {builtin}, skip {skip:?}");
            if at(&events, &call("after")).len() != 1 {
                wrong.push(format!("{case}: the walk never ended"));
            }
            if at(&events, "CutsceneEnded").len() != 1 {
                wrong.push(format!("{case}: the cutscene never ended"));
            }
            if skip.is_none() && at(&events, "Blocked").len() != 1 {
                wrong.push(format!("{case}: blocked at {:?}", at(&events, "Blocked")));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A room whose exposure starts at 1 and fog density at 0.
fn lit_room() -> sariema_runtime::scenery::RoomScene {
    use sariema_format::scenery::{Param, RoomIndex, Target};
    let param = |name: &str, value, target| Param {
        name: name.into(),
        value,
        target,
    };
    let index = RoomIndex {
        params: vec![
            param("look.exposure", 1.0, Target::Exposure),
            param("fog.density", 0.0, Target::FogDensity),
        ],
        ..RoomIndex::default()
    };
    sariema_runtime::scenery::RoomScene::new("hall", index)
}

/// Plays `cutscene` in [`lit_room`] with actor 0 as `hero` from step 1,
/// skipping it at `skip_at`; returns the exposure after each step and the
/// steps the game heard each call at.
fn scene_run(cutscene: &str, steps: u64, skip_at: Option<u64>) -> (Vec<f32>, Vec<(u64, String)>) {
    let compiled = compile_json(cutscene).unwrap().0;
    let name = compiled.name.clone();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
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
            _ if Some(step) == skip_at => send(out, Command::CutsceneSkip {}),
            _ => {}
        }
    });
    let mut sim = Simulation::new(Settings::default(), vec![compiled], logic, 1);
    sim.set_rooms(vec![lit_room()]);
    let actions = Actions::for_settings(&Settings::default().input);
    let mut exposure = Vec::new();
    for _ in 0..steps {
        sim.step(&actions);
        exposure.push(sim.state().scenery.param(name_id("look.exposure")).unwrap());
    }
    let fog = sim.state().scenery.param(name_id("fog.density")).unwrap();
    assert_eq!(fog, 0.1, "the fog ends where the cutscene set it");
    let events = seen.take();
    (exposure, events)
}

#[test]
fn a_scene_step_tweens_the_room_without_waiting() {
    let cutscene = format!(
        r#"{{ "name": "dusk", {ROLE_AT_ORIGIN}, "steps": [
            {{ "scene": {{ "set": {{ "look.exposure": 3.0, "fog.density": 0.1 }},
                           "duration": 1, "ease": "linear" }} }},
            {{ "call": "after" }},
            {{ "wait": 2 }} ] }}"#
    );
    let (exposure, events) = scene_run(&cutscene, 120, None);
    let first = exposure
        .iter()
        .position(|&e| e != 1.0)
        .expect("never moved");
    assert!(
        (exposure[first] - (1.0 + 2.0 / 60.0)).abs() < 1e-5,
        "{}",
        exposure[first]
    );
    let half = exposure[first + 29];
    assert!((half - 2.0).abs() < 1e-5, "half way: {half}");
    assert_eq!(exposure[first + 59], 3.0);
    assert!(
        exposure
            .windows(2)
            .all(|w| w[1] - w[0] <= 2.0 / 60.0 + 1e-5)
    );
    let after = at(&events, &call("after"));
    assert_eq!(after.len(), 1, "{events:?}");
    assert!(
        after[0] <= first as u64 + 2,
        "the call came at step {}, the tween started at {first}",
        after[0]
    );
}

#[test]
fn a_skipped_cutscene_lands_its_scene_steps_under_the_fade() {
    let cutscene = format!(
        r#"{{ "name": "night", {ROLE_AT_ORIGIN}, "steps": [
            {{ "scene": {{ "set": {{ "look.exposure": 3.0 }}, "duration": 4 }} }},
            {{ "wait": 1 }},
            {{ "scene": {{ "set": {{ "fog.density": 0.1 }}, "duration": 2 }} }},
            {{ "wait": 4 }} ] }}"#
    );
    let (played, _) = scene_run(&cutscene, 330, None);
    let (skipped, _) = scene_run(&cutscene, 330, Some(20));
    assert!(
        played[150] < 3.0,
        "still easing when played: {}",
        played[150]
    );
    assert_eq!(skipped[150], 3.0, "landed once skipped");
    assert_eq!((played[329], skipped[329]), (3.0, 3.0));
}
