// SPDX-License-Identifier: Apache-2.0

//! The compound collider, teleports and triggers.

mod common;

use common::{Owned, Room, actor, actor_with, hold, settle};
use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{ActorCollider, EventKind, Head, Intent, Settings, Shape, TriggerId};

fn big_head() -> ActorCollider {
    ActorCollider {
        radius: 0.25,
        height: 0.9,
        head: Some(Head {
            radius: 0.38,
            height: 1.15,
            forward: 0.0,
        }),
    }
}

#[test]
fn a_wall_leaning_towards_the_hero_stops_the_head_not_the_feet() {
    // A slab whose top leans 20° towards +z, the side the actor comes from.
    let lean = 20f32.to_radians();
    let rotation = Mat3::from_rotation_x(lean);
    let mut room = Room::default();
    room.floor(20.0).add(
        "leaning",
        Owned::Box {
            center: rotation * Vec3::new(0.0, 1.5, -0.1),
            half: Vec3::new(3.0, 1.5, 0.1),
            rotation,
        },
    );
    let mut world = room.world(Settings::default());
    let collider = big_head();
    let id = actor_with(&mut world, Vec3::new(0.0, 0.0, 2.5), collider, 2.1);
    settle(&mut world, id);
    let mut events = Vec::new();
    let positions = hold(&mut world, id, Vec2::new(0.0, -1.0), 120, &mut events);
    let end = *positions.last().unwrap();
    for p in &positions {
        assert!(
            world
                .actor_overlap(&collider, *p, std::f32::consts::PI)
                .is_none(),
            "inside at {p}"
        );
    }
    let mut touching = Vec::new();
    let head = end + Vec3::Y * 1.15;
    world.overlap(head, head, 0.38 + 0.02, &mut touching);
    assert!(
        touching
            .iter()
            .any(|c| world.collider_name(*c) == "leaning"),
        "head not at the wall: {end}"
    );
    let (a, b) = (end + Vec3::Y * 0.25, end + Vec3::Y * 0.65);
    world.overlap(a, b, 0.25 + 0.2, &mut touching);
    assert!(
        !touching
            .iter()
            .any(|c| world.collider_name(*c) == "leaning"),
        "feet near the wall: {end}"
    );
}

#[test]
fn standing_beside_a_diagonal_root_no_part_is_inside_it() {
    let mut room = Room::default();
    room.floor(20.0).add(
        "root",
        Owned::Capsule {
            a: Vec3::new(-3.0, -0.1, -1.0),
            b: Vec3::new(2.0, 1.4, 0.5),
            radius: 0.3,
        },
    );
    let mut world = room.world(Settings::default());
    let collider = big_head();
    for (start, stick) in [
        (Vec3::new(0.5, 0.0, 2.0), Vec2::new(0.0, -1.0)),
        (Vec3::new(-2.0, 0.0, 1.5), Vec2::new(0.3, -1.0)),
        (Vec3::new(1.0, 0.0, -2.0), Vec2::new(-0.2, 1.0)),
    ] {
        let id = actor_with(&mut world, start, collider, 2.1);
        settle(&mut world, id);
        let mut events = Vec::new();
        let positions = hold(&mut world, id, stick, 150, &mut events);
        for p in &positions {
            assert!(
                world.actor_overlap(&collider, *p, 0.0).is_none(),
                "inside the root at {p}"
            );
        }
        world.apply(Intent::Teleport {
            actor: id,
            position: Vec3::new(9.0, 0.0, 9.0),
            yaw: 0.0,
        });
        world.step(&mut events);
    }
}

#[test]
fn teleport_into_a_solid_resolves_in_one_pass_or_is_rejected() {
    let mut room = Room::default();
    room.floor(30.0)
        .add(
            "crate",
            Owned::Box {
                center: Vec3::new(0.0, 0.5, 0.0),
                half: Vec3::splat(0.5),
                rotation: Mat3::IDENTITY,
            },
        )
        .add(
            "block",
            Owned::Box {
                center: Vec3::new(8.0, 2.5, 0.0),
                half: Vec3::splat(2.5),
                rotation: Mat3::IDENTITY,
            },
        );
    let mut world = room.world(Settings::default());
    let collider = ActorCollider::default();
    let id = actor(&mut world, Vec3::new(-4.0, 0.0, 0.0));
    settle(&mut world, id);
    let mut events = Vec::new();
    world.apply(Intent::Teleport {
        actor: id,
        position: Vec3::new(0.6, 0.02, 0.1),
        yaw: 1.0,
    });
    world.step(&mut events);
    let moved = world.actor(id);
    assert!(
        world
            .actor_overlap(&collider, moved.position, moved.yaw)
            .is_none(),
        "{moved:?}"
    );
    assert!((moved.position.x - 0.81).abs() < 0.02, "{moved:?}");
    assert!(events.is_empty(), "{events:?}");

    let before = world.actor(id).position;
    world.apply(Intent::Teleport {
        actor: id,
        position: Vec3::new(8.0, 1.0, 0.0),
        yaw: 0.0,
    });
    world.step(&mut events);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, EventKind::TeleportRejected);
    let after = world.actor(id).position;
    assert!(after.distance(before) < 0.02, "{before} -> {after}");
    assert!(world.actor_overlap(&collider, after, 0.0).is_none());
}

fn trigger_kinds(events: &[sariema_physics::Event]) -> Vec<EventKind> {
    events
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                EventKind::TriggerEnter(_) | EventKind::TriggerExit(_)
            )
        })
        .map(|e| e.kind)
        .collect()
}

#[test]
fn triggers_fire_once_per_entry_and_exit_including_on_teleport() {
    let mut room = Room::default();
    room.floor(40.0);
    let mut world = room.world(Settings::default());
    let door = world
        .add_trigger(
            "door",
            Shape::Box {
                center: Vec3::new(0.0, 1.0, 0.0),
                half_extents: Vec3::new(1.0, 1.0, 0.5),
                rotation: Mat3::IDENTITY,
            },
        )
        .unwrap();
    let pool = world
        .add_trigger(
            "pool",
            Shape::Sphere {
                center: Vec3::new(10.0, 0.0, 0.0),
                radius: 1.0,
            },
        )
        .unwrap();
    assert_eq!((door, pool), (TriggerId(0), TriggerId(1)));
    let id = actor(&mut world, Vec3::new(0.0, 0.0, 4.0));
    settle(&mut world, id);
    let mut events = Vec::new();
    hold(&mut world, id, Vec2::new(0.0, -1.0), 240, &mut events);
    assert_eq!(
        trigger_kinds(&events),
        [EventKind::TriggerEnter(door), EventKind::TriggerExit(door)]
    );

    for (to, expect) in [
        (
            Vec3::new(0.0, 0.0, 0.0),
            vec![EventKind::TriggerEnter(door)],
        ),
        (
            Vec3::new(10.0, 0.0, 0.0),
            vec![EventKind::TriggerExit(door), EventKind::TriggerEnter(pool)],
        ),
        (Vec3::new(10.2, 0.0, 0.0), vec![]),
        (
            Vec3::new(-5.0, 0.0, 0.0),
            vec![EventKind::TriggerExit(pool)],
        ),
    ] {
        events.clear();
        world.apply(Intent::Stop { actor: id });
        world.apply(Intent::Teleport {
            actor: id,
            position: to,
            yaw: 0.0,
        });
        for _ in 0..3 {
            world.step(&mut events);
        }
        assert_eq!(trigger_kinds(&events), expect, "teleport to {to}");
    }
}
