// SPDX-License-Identifier: Apache-2.0

//! `blocked` says why a walk ended: no path to where it was sent, or held
//! back on its way.

mod common;

use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::script::{Command, Event, Kind, Value, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, block, collision, entity, native, send, wired};

/// A floor with a wall across x = 4.5, the far side of the spot the walk
/// goes to.
fn room() -> RoomAssets {
    let colliders = vec![
        block("floor", [-2.0, -0.1, -4.0], [8.0, 0.0, 4.0]),
        block("wall", [4.6, 0.0, -4.0], [4.8, 2.0, 4.0]),
    ];
    RoomAssets::collision("walled", collision("walled", colliders))
}

/// The `reason` field of the hero's first `blocked`, after a walk to
/// `target`, with another actor standing at `other` when given.
fn reason(target: [f32; 3], other: Option<[f32; 3]>) -> Option<String> {
    let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
    let got = seen.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::Blocked { entity: who, .. } = e
                && *who == entity(0)
                && got.borrow().is_none()
            {
                let mut why = String::from("none");
                e.fields(|name, value| {
                    if let ("reason", Value::Enum(v)) = (name, value) {
                        why = v.into();
                    }
                });
                *got.borrow_mut() = Some(why);
            }
        }
        match n {
            0 => {
                let actors = [Some([0.0, 0.0, 0.0]), other];
                for (k, at) in actors.into_iter().enumerate() {
                    let Some(at) = at else { continue };
                    send(
                        out,
                        Command::Spawn {
                            entity: entity(k as u32),
                            kind: Kind::Actor,
                            asset: name_id("hero"),
                        },
                    );
                    send(
                        out,
                        Command::SetTransform {
                            entity: entity(k as u32),
                            position: at,
                            yaw: 0.0,
                        },
                    );
                }
            }
            3 => send(
                out,
                Command::MoveTo {
                    entity: entity(0),
                    target,
                    speed: 1.5,
                },
            ),
            _ => {}
        }
    });
    let mut assets = assets();
    assets.rooms = vec![room()];
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..600 {
        engine.step(&actions);
    }
    seen.borrow().clone()
}

#[test]
fn a_walk_to_a_spot_off_the_floor_is_blocked_with_no_path() {
    assert_eq!(reason([30.0, 0.0, 0.0], None).as_deref(), Some("no_path"));
}

#[test]
fn a_walk_onto_an_actor_held_against_a_wall_is_blocked_stuck() {
    assert_eq!(
        reason([4.0, 0.0, 0.0], Some([4.0, 0.0, 0.0])).as_deref(),
        Some("stuck")
    );
}
