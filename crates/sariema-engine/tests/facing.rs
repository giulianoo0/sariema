// SPDX-License-Identifier: Apache-2.0

//! Yaw 0 faces −Z: an actor's model, whose front is glTF's +Z, is drawn
//! facing the way its yaw says, and so is everything its skeleton places in
//! the world, such as what it holds at a socket.

mod common;

use std::path::Path;

use glam::{Mat4, Vec3};
use sariema_engine::{Assets, Engine, EngineSettings};
use sariema_format::actor::ActorPack;
use sariema_format::script::{Command, Event, Kind, name_id};
use sariema_import::{ActorSource, build_actor};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send, spawn_hero, wired};

const ITEM: u32 = 3;

/// The `climb` hero with a socket half a metre in front of its shoulder,
/// on glTF's +Z.
fn hero() -> ActorPack {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let source = r#"{
        "name": "hero",
        "model": "../demo/actors/arm.glb",
        "sockets": { "front": { "joint": "shoulder", "translation": [0.0, 1.0, 0.5] } }
    }"#;
    build_actor(
        &ActorSource::from_json(source).unwrap(),
        &examples.join("collision"),
    )
    .unwrap()
}

/// The hero at the origin facing `yaw`, holding an empty at its front
/// socket, after a few steps.
fn played(yaw: f32) -> Engine<impl sariema_runtime::sim::Logic> {
    let logic = native(move |n, seen, out| {
        if seen.iter().any(|e| matches!(e, Event::Started { .. })) {
            spawn_hero(out, [0.0; 3], yaw);
            send(
                out,
                Command::Spawn {
                    entity: entity(ITEM),
                    kind: Kind::Empty,
                    asset: 0,
                },
            );
        }
        if n == 2 {
            send(
                out,
                Command::Hold {
                    entity: entity(ITEM),
                    actor: entity(0),
                    socket: name_id("front"),
                    offset: [0.0; 3],
                    keep: false,
                },
            );
        }
    });
    let assets = Assets {
        actors: vec![("hero".into(), hero())],
        ..common::assets()
    };
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..10 {
        engine.step(&actions);
    }
    engine
}

/// The vertex furthest along glTF's +Z in the bind pose: the front of the model.
fn front_vertex(pack: &ActorPack) -> (Vec3, [u16; 4], [u16; 4]) {
    (pack.meshes.iter())
        .flat_map(|m| &m.vertices)
        .map(|v| (Vec3::from(v.pos), v.joints, v.weights))
        .max_by(|a, b| a.0.z.total_cmp(&b.0.z))
        .unwrap()
}

fn skinned(
    transform: Mat4,
    palette: &[Mat4],
    (pos, joints, weights): (Vec3, [u16; 4], [u16; 4]),
) -> Vec3 {
    let model = (joints.iter().zip(weights))
        .map(|(&j, w)| palette[j as usize].transform_point3(pos) * (f32::from(w) / 65535.0))
        .sum::<Vec3>();
    transform.transform_point3(model)
}

#[test]
fn yaw_0_draws_the_model_s_front_towards_minus_z_and_yaw_90_towards_minus_x() {
    let pack = hero();
    let front = front_vertex(&pack);
    assert!(front.0.z > 0.01, "the model has a front: {}", front.0);
    for (yaw, ahead) in [(0.0, Vec3::NEG_Z), (90.0, Vec3::NEG_X)] {
        let mut engine = played(yaw);
        let actor = engine.actors(1.0).next().unwrap();
        let at = skinned(actor.transform, actor.palette, front);
        let origin = actor.transform.w_axis.truncate();
        assert!(
            (at - origin).dot(ahead) > 0.01,
            "yaw {yaw}: the model's front is drawn at {at}, not towards {ahead} of {origin}"
        );
    }
}

#[test]
fn what_an_actor_holds_at_a_socket_on_its_front_is_on_the_side_its_yaw_faces() {
    for (yaw, ahead) in [(0.0, Vec3::NEG_Z), (90.0, Vec3::NEG_X)] {
        let engine = played(yaw);
        let world = &engine.state().world;
        let item = world.pose(entity(ITEM)).unwrap().position;
        let hero = world.pose(entity(0)).unwrap().position;
        assert!(
            (item - hero).dot(ahead) > 0.4,
            "yaw {yaw}: held at {item}, not half a metre towards {ahead} of the hero at {hero}"
        );
    }
}
