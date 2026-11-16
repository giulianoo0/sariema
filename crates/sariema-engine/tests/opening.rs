// SPDX-License-Identifier: Apache-2.0

//! A cutscene the game plays as it starts walks its hero under the
//! collision module, though the hero, its placement and the walk all come
//! in the same step.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_format::script::{Command, Event, name_id};
use sariema_runtime::input::Actions;

use common::{engine, entity, native, send, spawn_hero, wired};

const OPENING: &str = r#"{ "name": "opening", "roles": { "hero": { "start": { "position": [4.5, 0, -1], "yaw": 0 } } },
    "steps": [ { "walk": { "role": "hero", "to": [4.5, 0, -1.6], "speed": 1.2 } } ] }"#;

#[test]
fn a_cutscene_played_with_the_first_commands_walks_its_hero_to_the_mark() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if matches!(
                e,
                Event::Arrived { .. } | Event::Blocked { .. } | Event::CutsceneEnded { .. }
            ) {
                log.borrow_mut().push(format!("{e:?}"));
            }
        }
        if n == 0 {
            spawn_hero(out, [4.5, 0.0, -1.0], 0.0);
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
                    cutscene: name_id("opening"),
                },
            );
        }
    });
    let mut engine = engine(logic, wired(), &[OPENING]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..120 {
        engine.step(&actions);
    }
    let at = engine.state().world.node(entity(0)).unwrap().local.position;
    drop(engine);
    let seen = seen.take();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(seen[0].starts_with("Arrived"), "{seen:?}");
    assert!(seen[1].starts_with("CutsceneEnded"), "{seen:?}");
    assert!((at.z - -1.6).abs() < 0.03, "{at}");
}
