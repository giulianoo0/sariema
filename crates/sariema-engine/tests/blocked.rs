// SPDX-License-Identifier: Apache-2.0

//! A cutscene walk the collision module cannot make ends with `blocked`
//! where the role stood, played or skipped alike.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_format::script::{Command, Event, name_id};
use sariema_runtime::input::Actions;

use common::{engine, entity, native, send, spawn_hero, wired};

const START: [f32; 3] = [0.75, 0.0, -0.4];

/// The far half of the room, behind the window: no path leads there.
const FAR: &str = r#"{ "name": "far", "roles": { "hero": { "start": { "position": [0.75, 0, -0.4], "yaw": 0 } } },
    "steps": [ { "wait": 1 }, { "walk": { "role": "hero", "to": [0.75, 0, -4.5], "speed": 1.2 } }, { "call": "after" } ] }"#;

fn play(skip: bool) -> (Vec<(u64, String)>, Vec3) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if matches!(
                e,
                Event::Blocked { .. }
                    | Event::Arrived { .. }
                    | Event::CutsceneCall { .. }
                    | Event::CutsceneEnded { .. }
                    | Event::Rejected { .. }
            ) {
                log.borrow_mut().push((n, format!("{e:?}")));
            }
        }
        match n {
            0 => spawn_hero(out, START, 0.0),
            1 => {
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
                        cutscene: name_id("far"),
                    },
                );
            }
            2 if skip => send(out, Command::CutsceneSkip {}),
            _ => {}
        }
    });
    let mut engine = engine(logic, wired(), &[FAR]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..240 {
        engine.step(&actions);
    }
    let at = engine.state().world.node(entity(0)).unwrap().local.position;
    drop(engine);
    (seen.take(), at)
}

#[test]
fn a_physics_blocked_cutscene_walk_ends_with_blocked_played_or_skipped() {
    for skip in [false, true] {
        let (events, at) = play(skip);
        let count = |what: &str| events.iter().filter(|(_, e)| e.starts_with(what)).count();
        assert_eq!(
            (count("Blocked"), count("Arrived"), count("Rejected")),
            (1, 0, 0),
            "skip {skip}: {events:?}"
        );
        assert_eq!(count("CutsceneEnded"), 1, "skip {skip}: {events:?}");
        assert!(
            at.distance(Vec3::from(START)) < 0.05,
            "skip {skip}: ended at {at}, not where the hero stood"
        );
    }
}
