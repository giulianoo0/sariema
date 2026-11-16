// SPDX-License-Identifier: Apache-2.0

//! A cutscene's walks take the time their length and speed give, at the
//! speed asked in world units whatever the role's speed scale: the walk to
//! a start, and every `walk` step.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_core::STEP_SECONDS;
use sariema_format::script::{Command, Event, name_id};
use sariema_runtime::input::Actions;

use common::{engine, entity, native, send, spawn_hero, wired};

const FROM: [f32; 3] = [1.5, 0.0, -0.6];

fn walk(speed: f32) -> String {
    format!(
        r#"{{ "name": "stroll",
        "roles": {{ "hero": {{ "start": {{ "position": [1.5, 0, -0.6], "yaw": -90 }} }} }},
        "steps": [
          {{ "call": "go" }},
          {{ "walk": {{ "role": "hero", "to": [4.5, 0, -0.6], "speed": {speed} }} }},
          {{ "call": "after" }} ] }}"#
    )
}

struct Played {
    /// Each step's root.
    roots: Vec<Vec3>,
    /// The step each call reached the game.
    calls: Vec<(u32, u64)>,
}

/// The hero spawned at `at`, its speed scaled by `scale`, plays `cutscene`.
fn play(cutscene: &str, at: [f32; 3], scale: f32, steps: u64) -> Played {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::CutsceneCall { call, .. } = e {
                log.borrow_mut().push((*call, n));
            }
        }
        if n == 0 {
            spawn_hero(out, at, -90.0);
        }
        if n == 1 {
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
            send(
                out,
                Command::CutscenePlay {
                    cutscene: name_id("stroll"),
                },
            );
        }
    });
    let mut engine = engine(logic, wired(), &[cutscene]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut roots = Vec::new();
    for _ in 0..steps {
        engine.step(&actions);
        roots.push(engine.state().world.node(entity(0)).unwrap().local.position);
    }
    let calls = calls.take();
    Played { roots, calls }
}

fn call(p: &Played, name: &str) -> u64 {
    (p.calls.iter())
        .find(|(c, _)| *c == name_id(name))
        .map(|(_, n)| *n)
        .unwrap_or_else(|| panic!("no call {name}: {:?}", p.calls))
}

/// The fastest the root went in a step, in m/s.
fn fastest(roots: &[Vec3]) -> f32 {
    (roots.windows(2))
        .map(|w| (w[1] - w[0]).with_y(0.0).length() / STEP_SECONDS)
        .fold(0.0, f32::max)
}

#[test]
fn a_walk_takes_its_length_over_its_speed() {
    let p = play(&walk(1.5), FROM, 1.0, 200);
    let took = call(&p, "after") - call(&p, "go");
    let due = (3.0 / 1.5 / STEP_SECONDS).round() as u64;
    assert!(
        took.abs_diff(due) <= 2,
        "3 m at 1.5 m/s is {due} steps; the walk took {took}"
    );
}

#[test]
fn a_walk_goes_at_its_speed_whatever_the_roles_speed_scale() {
    let p = play(&walk(1.2), FROM, 2.25, 300);
    let speed = fastest(&p.roots[call(&p, "go") as usize..]);
    assert!(
        (speed - 1.2).abs() < 0.02,
        "1.2 m/s asked, {speed} m/s walked at speed scale 2.25"
    );
    let took = call(&p, "after") - call(&p, "go");
    let due = (3.0 / 1.2 / STEP_SECONDS).round() as u64;
    assert!(took.abs_diff(due) <= 2, "{took} steps against {due}");
}

#[test]
fn the_walk_to_a_start_goes_at_its_setting_whatever_the_roles_speed_scale() {
    let p = play(&walk(1.2), [4.5, 0.0, -0.6], 2.25, 200);
    let go = call(&p, "go") as usize;
    let speed = fastest(&p.roots[2..go]);
    assert!(
        (speed - 1.5).abs() < 0.02,
        "cutscene.start_walk_speed is 1.5 m/s; the walk to the start went {speed} m/s"
    );
}

#[test]
fn a_start_off_only_in_height_from_the_floor_begins_at_once() {
    for (y, x) in [(0.06, 0.0), (0.2, 0.0), (0.06, 0.1)] {
        let json = format!(
            r#"{{ "name": "stroll",
            "roles": {{ "hero": {{ "start": {{ "position": [{x}, {y}, -0.6], "yaw": -90 }} }} }},
            "steps": [ {{ "call": "go" }} ] }}"#,
            x = 1.5 + x
        );
        let p = play(&json, FROM, 1.0, 60);
        let go = call(&p, "go");
        let eased = if x > 0.0 { 15 } else { 0 };
        assert!(
            go.abs_diff(2 + eased) <= 1,
            "start {y} m up and {x} m aside: the steps began at {go}"
        );
        let root = p.roots[59];
        assert!(
            root.y < 0.02,
            "standing on the floor, not at the start's height: {root}"
        );
    }
}
