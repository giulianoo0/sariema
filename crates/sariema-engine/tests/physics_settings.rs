// SPDX-License-Identifier: Apache-2.0

//! A game's physics settings reach the collision module: a stair of 0.25 m
//! risers stops a hero at the default step height of 0.2 m and is climbed
//! with the 0.26 m the game's settings give.

mod common;

use glam::Vec3;
use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::collision::{ActorBody, Collider, CollisionPack, Shape};
use sariema_format::script::Command;
use sariema_runtime::game::parse_settings;
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, entity, native, send, spawn_hero};

const RISER: f32 = 0.25;

fn block(name: &str, min: [f32; 3], max: [f32; 3]) -> Collider {
    let centre = |k: usize| (min[k] + max[k]) * 0.5;
    let half = |k: usize| (max[k] - min[k]) * 0.5;
    Collider {
        name: name.into(),
        shape: Shape::Box {
            center: [centre(0), centre(1), centre(2)],
            half_extents: [half(0), half(1), half(2)],
            rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        },
        enabled: true,
    }
}

/// A floor at height 0, then three risers up to a landing at 0.75 m,
/// climbing along +x from x 2 with 0.6 m treads.
fn stairs() -> CollisionPack {
    let mut colliders = vec![block("floor", [-2.0, -0.1, -2.0], [12.0, 0.0, 2.0])];
    for k in 1..=3 {
        let from = 2.0 + 0.6 * (k - 1) as f32;
        colliders.push(block(
            &format!("step{k}"),
            [from, 0.0, -2.0],
            [12.0, RISER * k as f32, 2.0],
        ));
    }
    CollisionPack {
        room: "stairs".into(),
        colliders,
        triggers: Vec::new(),
        actors: vec![ActorBody {
            name: "hero".into(),
            radius: 0.3,
            height: 1.0,
            head: None,
            visual_bounds: None,
        }],
        spots: Vec::new(),
        routes: Vec::new(),
        traversals: Vec::new(),
        nav_bounds: None,
        hinges: Vec::new(),
    }
}

/// Where a hero walking +x at 1.5 m/s for four seconds ends, under the
/// game settings `settings` as a game pack carries them.
fn walk_up(settings: &str) -> Vec3 {
    let logic = native(|n, _, out| match n {
        0 => spawn_hero(out, [0.0, 0.0, 0.0], -90.0),
        1 => send(
            out,
            Command::MoveIntent {
                entity: entity(0),
                velocity: [1.5, 0.0, 0.0],
            },
        ),
        _ => {}
    });
    let mut assets = assets();
    assets.rooms = vec![RoomAssets::collision("stairs", stairs())];
    let sim = Simulation::new(parse_settings(settings).unwrap(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..240 {
        engine.step(&actions);
    }
    engine.state().world.node(entity(0)).unwrap().local.position
}

#[test]
fn the_games_step_height_climbs_a_stair_the_default_stops_at() {
    let stopped = walk_up(r#"{ "motion": { "builtin": false } }"#);
    assert!(stopped.y.abs() < 0.01 && stopped.x < 2.0, "{stopped}");
    let climbed = walk_up(
        r#"{ "motion": { "builtin": false }, "physics": { "controller": { "step_height": 0.26 } } }"#,
    );
    assert!(
        (climbed.y - 3.0 * RISER).abs() < 0.01 && climbed.x > 4.0,
        "{climbed}"
    );
}
