// SPDX-License-Identifier: Apache-2.0

//! An actor held where a kinematic move ended is part of the world: it
//! hashes apart from the same actor standing there, and `locate` says so.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use sariema_format::script::{Command, Event, Kind, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

/// The world hash after an actor is put at the same place, held or not,
/// and what `locate` said of it.
fn put(hold: bool) -> (u64, bool) {
    let held = Rc::new(Cell::new(None));
    let heard = held.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::Located { held, .. } = *e {
                heard.set(Some(held));
            }
        }
        match n {
            0 => send(
                out,
                Command::Spawn {
                    entity: entity(0),
                    kind: Kind::Actor,
                    asset: name_id("hero"),
                },
            ),
            1 => send(
                out,
                Command::Teleport {
                    entity: entity(0),
                    position: [1.0, 1.2, 0.0],
                    yaw: 0.0,
                    force: true,
                    hold,
                },
            ),
            2 => send(out, Command::Locate { entity: entity(0) }),
            _ => {}
        }
    });
    let mut sim = Simulation::new(Settings::default(), Vec::new(), logic, 1);
    let actions = Actions::for_settings(&sim.state().settings.input);
    for _ in 0..4 {
        sim.step(&actions);
    }
    (sim.state().world_hash(), held.get().expect("located"))
}

#[test]
fn a_held_actor_hashes_apart_and_is_located_held() {
    let (standing, held_there) = put(false);
    let (held, held_now) = put(true);
    assert!(!held_there);
    assert!(held_now);
    assert_ne!(standing, held);
}
