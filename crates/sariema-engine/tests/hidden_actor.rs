// SPDX-License-Identifier: Apache-2.0

//! A hidden actor is in no other actor's way: a walk onto its spot passes
//! through it and leaves it where it stood.

mod common;

use glam::Vec3;
use sariema_format::script::{Command, Event, Kind, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, entity, native, send, wired};
use sariema_engine::{Engine, EngineSettings};

const SPOT: [f32; 3] = [5.4, 0.0, -5.0];

#[test]
fn a_walk_onto_a_hidden_actor_passes_through_it_and_arrives() {
    let arrived = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = arrived.clone();
    let logic = native(move |n, events, out| {
        assert!(
            !events.iter().any(|e| matches!(e, Event::Rejected { .. })),
            "{events:?}"
        );
        if events
            .iter()
            .any(|e| matches!(e, Event::Arrived { entity: e } if *e == entity(0)))
        {
            seen.set(true);
        }
        match n {
            0 => {
                for (k, at) in [(0, [0.6, 0.0, -5.0]), (1, SPOT)] {
                    send(
                        out,
                        Command::Spawn {
                            entity: entity(k),
                            kind: Kind::Actor,
                            asset: name_id("hero"),
                        },
                    );
                    send(
                        out,
                        Command::SetTransform {
                            entity: entity(k),
                            position: at,
                            yaw: 0.0,
                        },
                    );
                }
            }
            2 => send(
                out,
                Command::SetVisible {
                    entity: entity(1),
                    visible: false,
                },
            ),
            3 => send(
                out,
                Command::MoveTo {
                    entity: entity(0),
                    target: SPOT,
                    speed: 1.5,
                },
            ),
            _ => {}
        }
    });
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets(), EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let at = |engine: &Engine<_>| engine.state().world.node(entity(1)).unwrap().local.position;
    for _ in 0..3 {
        engine.step(&actions);
    }
    let stood = at(&engine);
    let mut moved = 0.0f32;
    for _ in 0..400 {
        engine.step(&actions);
        moved = moved.max((at(&engine) - stood).length());
    }
    assert!(arrived.get(), "the walker arrives");
    assert!(moved < 1e-4, "the hidden actor was pushed {moved} m");
    let walker = engine.state().world.node(entity(0)).unwrap().local.position;
    assert!(
        (walker - Vec3::from(SPOT)).length() < 0.05,
        "the walker ends at {walker}"
    );
}
