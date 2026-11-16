// SPDX-License-Identifier: Apache-2.0

//! The animator's pose handed to the other modules: a traversal's contact
//! lands the socket it names on its point, and a camera rig anchored to a
//! socket aims at it.

mod common;

use glam::{Quat, Vec3};
use sariema_engine::{Engine, EngineSettings};
use sariema_format::collision::Contact;
use sariema_format::script::{Anchor, Command, Ease, Entity, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::{Logic, Simulation};

use common::{assets, entity, native, send, spawn_hero, wired};

const CONTACT: [f32; 3] = [3.5, 1.6, -3.2];

/// Where `e`'s socket `name` is in the world at the end of the last step.
fn socket<L: Logic>(engine: &Engine<L>, e: Entity, name: &str) -> Vec3 {
    let (_, model, animator) = engine
        .animation()
        .actors()
        .find(|(a, ..)| *a == e)
        .expect("an animated actor");
    let rig = &engine.animation().models[model].rig;
    let local = animator
        .socket(rig, rig.socket(name).unwrap())
        .w_axis
        .truncate();
    let node = engine.state().world.node(e).unwrap();
    let rotation = Quat::from_rotation_y(node.local.yaw.to_radians());
    node.local.position + rotation * (local * node.local.scale)
}

#[test]
fn a_traversal_contact_lands_the_socket_it_names_on_its_point() {
    let mut assets = assets();
    let collision = assets.collision.as_mut().unwrap();
    collision.traversals[0].contacts = vec![Contact {
        name: "grip".into(),
        point: CONTACT,
    }];
    let logic = native(|n, _, out| match n {
        0 => spawn_hero(out, [4.5, 0.0, -1.6], 0.0),
        2 => send(
            out,
            Command::Traverse {
                entity: entity(0),
                traversal: name_id("window"),
                duration: 1.6,
            },
        ),
        _ => {}
    });
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut nearest = (f32::INFINITY, 0);
    for _ in 0..110 {
        engine.step(&actions);
        let d = socket(&engine, entity(0), "grip").distance(Vec3::from(CONTACT));
        if d < nearest.0 {
            nearest = (d, engine.steps());
        }
    }
    let (d, step) = nearest;
    assert!(
        d < 0.02,
        "the grip came within {d} m of the contact, at step {step}"
    );
    assert!((10..100).contains(&step), "mid-traversal: step {step}");
}

#[test]
fn a_rig_anchored_to_a_socket_aims_at_it() {
    let logic = native(|n, _, out| {
        if n == 0 {
            spawn_hero(out, [2.5, 0.0, -1.25], 30.0);
            send(
                out,
                Command::RigFollow {
                    rig: 1,
                    subject: entity(0),
                    distance: 3.0,
                    pitch: -15.0,
                    yaw: 20.0,
                    height: 0.0,
                    fov: 45.0,
                },
            );
            send(
                out,
                Command::RigAnchor {
                    rig: 1,
                    subject: entity(0),
                    anchor: Anchor::Socket,
                    name: name_id("grip"),
                },
            );
            send(
                out,
                Command::CameraUse {
                    rig: 1,
                    blend: 0.0,
                    ease: Ease::Linear,
                },
            );
        }
    });
    let mut engine = common::engine(logic, wired(), &[]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..60 {
        engine.step(&actions);
    }
    let grip = socket(&engine, entity(0), "grip");
    let origin = engine.state().world.node(entity(0)).unwrap().local.position;
    let pose = engine.state().camera.pose().expect("the rig is in use");
    let forward = pose.rotation * Vec3::NEG_Z;
    let angle = |at: Vec3| forward.angle_between(at - pose.position).to_degrees();
    assert!(
        angle(grip) < 0.5,
        "the camera aims {}° off the grip",
        angle(grip)
    );
    assert!(angle(origin) > 5.0, "the grip is apart from the origin");
}
