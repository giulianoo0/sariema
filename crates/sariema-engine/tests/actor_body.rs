// SPDX-License-Identifier: Apache-2.0

//! `set_collider` on an actor switches its body: a hidden actor whose body
//! is off is no invisible obstacle, the hero walks through where it
//! stands without moving it, in its room and in the next one; with its
//! body on, the hero pushes it aside.

mod common;

use glam::Vec3;
use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::script::{Command, Event, Kind, RoomTransition, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, block, collision, entity, native, send, spawn_hero, wired};

const HIDDEN: [f32; 3] = [3.0, 0.0, 0.0];

fn room(name: &str) -> RoomAssets {
    let floor = block("floor", [-2.0, -0.1, -2.0], [12.0, 0.0, 2.0]);
    RoomAssets::collision(name, collision(name, vec![floor]))
}

/// Where the hidden actor and the hero end after the hero walks +x at
/// 1.5 m/s for four seconds through where the hidden one stands, its body
/// on or off; with `switch`, in the second room, entered after the body
/// was set.
fn walk_through(body: bool, switch: bool) -> (Vec3, Vec3) {
    let logic = native(move |n, events, out| match n {
        _ if events.iter().any(|e| matches!(e, Event::Rejected { .. })) => {
            panic!("{events:?}")
        }
        0 => {
            spawn_hero(out, [0.0, 0.0, 0.0], -90.0);
            send(
                out,
                Command::Spawn {
                    entity: entity(1),
                    kind: Kind::Actor,
                    asset: name_id("hero"),
                },
            );
            send(
                out,
                Command::SetTransform {
                    entity: entity(1),
                    position: HIDDEN,
                    yaw: 0.0,
                },
            );
            send(
                out,
                Command::SetVisible {
                    entity: entity(1),
                    visible: false,
                },
            );
        }
        1 => send(
            out,
            Command::SetCollider {
                entity: entity(1),
                enabled: body,
            },
        ),
        2 if switch => send(
            out,
            Command::RoomSwitch {
                room: name_id("next"),
                transition: RoomTransition::Cut,
                duration: 0.0,
            },
        ),
        3 => send(
            out,
            Command::MoveIntent {
                entity: entity(0),
                velocity: [1.5, 0.0, 0.0],
            },
        ),
        _ => {}
    });
    let mut assets = assets();
    assets.rooms = vec![room("first"), room("next")];
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..244 {
        engine.step(&actions);
    }
    let at = |i: u32| engine.state().world.node(entity(i)).unwrap().local.position;
    (at(1), at(0))
}

#[test]
fn a_hidden_actor_with_its_body_off_is_no_obstacle() {
    for switch in [false, true] {
        let (hidden, hero) = walk_through(false, switch);
        assert_eq!(
            (hidden.x, hidden.z),
            (HIDDEN[0], HIDDEN[2]),
            "switch {switch}"
        );
        assert!(
            hero.x > 5.5 && hero.z.abs() < 1e-3,
            "switch {switch}: {hero}"
        );
        let (pushed, _) = walk_through(true, switch);
        assert!(
            pushed.distance(Vec3::from(HIDDEN)) > 0.1,
            "switch {switch}: {pushed}"
        );
    }
}
