// SPDX-License-Identifier: Apache-2.0

//! What the engine asks of the world besides walking: turning in place, a
//! cap on how fast anything turns, and actors that come and go.

mod common;

use glam::Vec3;
use sariema_physics::{EventKind, Intent, Settings, Shape, wrap_angle};

use common::{Room, actor, settle};

fn room() -> Room {
    let mut room = Room::default();
    room.floor(20.0);
    room
}

#[test]
fn face_turns_an_actor_in_place_no_faster_than_max_yaw_speed() {
    let mut settings = Settings::default();
    settings.controller.max_yaw_speed = std::f32::consts::PI;
    let mut world = room().world(settings);
    let id = actor(&mut world, Vec3::ZERO);
    settle(&mut world, id);
    let at = world.actor(id).position;
    world.apply(Intent::Face {
        actor: id,
        yaw: std::f32::consts::FRAC_PI_2,
    });
    let mut events = Vec::new();
    let most = std::f32::consts::PI / 60.0;
    for _ in 0..60 {
        world.step(&mut events);
        let a = world.actor(id);
        let turned = wrap_angle(a.yaw - a.previous_yaw).abs();
        assert!(turned <= most * 1.0001, "turned {turned} rad in a step");
        assert!(a.position.distance(at) < 1e-6, "moved to {}", a.position);
    }
    let yaw = world.actor(id).yaw;
    assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 0.01, "{yaw}");
}

#[test]
fn an_inactive_actor_is_left_out_of_steps_and_leaves_its_triggers() {
    let mut world = room().world(Settings::default());
    world
        .add_trigger(
            "pad",
            Shape::Sphere {
                center: Vec3::new(0.0, 0.5, 0.0),
                radius: 1.0,
            },
        )
        .unwrap();
    let a = actor(&mut world, Vec3::ZERO);
    let b = actor(&mut world, Vec3::new(3.0, 0.0, 0.0));
    let mut events = Vec::new();
    world.step(&mut events);
    assert!(
        events
            .iter()
            .any(|e| e.actor == a && matches!(e.kind, EventKind::TriggerEnter(_)))
    );
    events.clear();
    world.set_actor_active(a, false, &mut events);
    assert!(
        events
            .iter()
            .any(|e| e.actor == a && matches!(e.kind, EventKind::TriggerExit(_)))
    );
    world.apply(Intent::MoveTo {
        actor: b,
        target: Vec3::ZERO,
        face: None,
    });
    let still = world.actor(a).position;
    for _ in 0..240 {
        events.clear();
        world.step(&mut events);
        assert!(
            events.iter().all(|e| e.actor != a),
            "an inactive actor reported {events:?}"
        );
    }
    assert_eq!(world.actor(a).position, still, "nothing pushed it");
    assert!(
        world.actor(b).position.distance(Vec3::ZERO) < 0.05,
        "the other walked through where it stood: {}",
        world.actor(b).position
    );
    world.set_actor_active(a, true, &mut events);
    assert!(world.actor_active(a));
}
