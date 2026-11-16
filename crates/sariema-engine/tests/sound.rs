// SPDX-License-Identifier: Apache-2.0

//! A sound played at an entity is heard from the camera as the entity
//! moves: louder near, panned to its side.

mod common;

use sariema_format::script::{Bus, Command, Ease, name_id};
use sariema_runtime::input::Actions;

use common::{engine, entity, native, send, spawn_hero, wired};

#[test]
fn a_sound_tied_to_an_entity_follows_it() {
    let logic = native(|n, _, out| match n {
        0 => {
            spawn_hero(out, [0.8, 0.0, -1.0], 0.0);
            send(
                out,
                Command::RigFixed {
                    rig: 1,
                    position: [1.0, 1.5, 3.0],
                    look_at: [1.0, 0.5, -1.0],
                    fov: 50.0,
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
        2 => send(
            out,
            Command::PlaySound {
                sound: name_id("wind"),
                bus: Bus::Ambience,
                emitter: entity(0),
                volume: 1.0,
            },
        ),
        3 => send(
            out,
            Command::MoveTo {
                entity: entity(0),
                target: [5.0, 0.0, -1.0],
                speed: 1.5,
            },
        ),
        _ => {}
    });
    let mut engine = engine(logic, wired(), &[]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let heard = |engine: &sariema_engine::Engine<_>| {
        engine
            .sound()
            .unwrap()
            .emitters()
            .find(|e| e.0 == entity(0))
            .map(|(_, gain, pan)| (gain, pan))
    };
    for _ in 0..4 {
        engine.step(&actions);
    }
    let (near_gain, left_pan) = heard(&engine).expect("the sound follows the hero");
    for _ in 0..200 {
        engine.step(&actions);
    }
    let x = engine
        .state()
        .world
        .node(entity(0))
        .unwrap()
        .local
        .position
        .x;
    assert!(x > 4.0, "the hero walked east to x {x}");
    let (far_gain, right_pan) = heard(&engine).unwrap();
    assert!(
        left_pan < -0.05 && right_pan > left_pan + 0.5,
        "pan went from {left_pan} to {right_pan}"
    );
    assert!(
        far_gain < near_gain,
        "gain went from {near_gain} to {far_gain}"
    );
}
