// SPDX-License-Identifier: Apache-2.0

//! `locate` under collision answers with where the collision module left
//! the actor, walking or held against a wall, the same on every run: what
//! a save needs to bring the hero back where it stood.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::script::{Command, Event};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, block, collision, entity, native, send, spawn_hero, wired};

/// Steps the game asks at: walking, then pressed against the wall.
const ASKED: [u64; 2] = [30, 200];

/// A `located` answer: the tick it came at, the place and the yaw.
type Answer = (u64, Vec3, f32);

/// The answers, and the hero's place and yaw after every step.
fn run() -> (Vec<Answer>, Vec<(Vec3, f32)>) {
    let answers = Rc::new(RefCell::new(Vec::new()));
    let log = answers.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            match *e {
                Event::Located { position, yaw, .. } => {
                    log.borrow_mut().push((n, Vec3::from(position), yaw))
                }
                Event::Rejected { .. } => panic!("{e:?}"),
                _ => {}
            }
        }
        match n {
            0 => spawn_hero(out, [0.0, 0.0, 0.0], 0.0),
            1 => send(
                out,
                Command::MoveIntent {
                    entity: entity(0),
                    velocity: [1.5, 0.0, 0.0],
                },
            ),
            n if ASKED.contains(&n) => send(out, Command::Locate { entity: entity(0) }),
            _ => {}
        }
    });
    let mut assets = assets();
    let colliders = vec![
        block("floor", [-2.0, -0.1, -2.0], [12.0, 0.0, 2.0]),
        block("wall", [3.0, 0.0, -2.0], [3.2, 2.0, 2.0]),
    ];
    assets.rooms = vec![RoomAssets::collision(
        "walled",
        collision("walled", colliders),
    )];
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut placed = Vec::new();
    for _ in 0..=ASKED[1] + 1 {
        engine.step(&actions);
        let node = engine.state().world.node(entity(0)).unwrap();
        placed.push((node.local.position, node.local.yaw));
    }
    drop(engine);
    (answers.take(), placed)
}

#[test]
fn locate_gives_where_the_collision_module_left_the_actor() {
    let (answers, placed) = run();
    let expected: Vec<Answer> = ASKED
        .iter()
        .map(|&n| (n + 1, placed[n as usize].0, placed[n as usize].1))
        .collect();
    assert_eq!(answers, expected);
    let (walking, held) = (answers[0].1, answers[1].1);
    assert!(walking.x > 0.5 && walking.x < 2.5, "{walking}");
    assert!((held.x - (3.0 - 0.3)).abs() < 0.02, "{held}");
    assert_eq!(run().0, answers);
}
