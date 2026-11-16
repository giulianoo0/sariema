// SPDX-License-Identifier: Apache-2.0

//! The compound collider, teleports and triggers.

mod common;

use common::{Owned, Room, actor, actor_with, hold, settle};
use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{
    ActorCollider, ActorDesc, EventKind, Head, Intent, Settings, Shape, TriggerId, World,
};

fn big_head() -> ActorCollider {
    ActorCollider {
        radius: 0.25,
        height: 0.9,
        head: Some(Head {
            radius: 0.38,
            height: 1.15,
            forward: 0.1,
        }),
    }
}

/// A head 0.3 m ahead of the body's axis, as `ActorCollider::fit` gives a
/// character whose head juts forward.
fn forward_head() -> ActorCollider {
    ActorCollider {
        radius: 0.3,
        height: 1.0,
        head: Some(Head {
            radius: 0.3,
            height: 1.1,
            forward: 0.3,
        }),
    }
}

/// A wall whose south face is 0.32 m north of an actor at the origin facing
/// east, its head clear of the wall.
fn facing_east_by_a_wall() -> (World, sariema_physics::ActorId) {
    let mut room = Room::default();
    room.floor(20.0)
        .wall("wall", Vec2::new(0.0, -0.42), Vec3::new(6.0, 2.0, 0.2), 0.0);
    let mut world = room.world(Settings::default());
    let id = world
        .add_actor(ActorDesc {
            position: Vec3::ZERO,
            yaw: std::f32::consts::FRAC_PI_2,
            collider: forward_head(),
            ..ActorDesc::default()
        })
        .unwrap();
    (world, id)
}

/// The deepest the actor's collider is inside the room over `steps` steps.
fn deepest_over(world: &mut World, id: sariema_physics::ActorId, steps: usize) -> f32 {
    let collider = world.actor_collider(id);
    let mut events = Vec::new();
    let mut deepest = 0.0f32;
    for _ in 0..steps {
        world.step(&mut events);
        let a = world.actor(id);
        if let Some((_, separation)) = world.actor_overlap(&collider, a.position, a.yaw) {
            deepest = deepest.min(separation);
        }
    }
    deepest
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
    world.apply(Intent::Move {
        actor: id,
        stick: Vec2::new(0.0, -1.0),
    });
    for _ in 0..120 {
        world.step(&mut events);
        let a = world.actor(id);
        assert!(
            world.actor_overlap(&collider, a.position, a.yaw).is_none(),
            "inside at {a:?}"
        );
    }
    let a = world.actor(id);
    let end = a.position;
    let mut touching = Vec::new();
    let head = end + Vec3::Y * 1.15 + sariema_physics::forward(a.yaw) * 0.1;
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

#[test]
fn a_teleport_clears_the_head_where_it_will_face() {
    let mut room = Room::default();
    room.floor(20.0)
        .wall("wall", Vec2::new(0.0, -1.0), Vec3::new(4.0, 2.0, 0.2), 0.0);
    let mut world = room.world(Settings::default());
    let collider = ActorCollider {
        radius: 0.3,
        height: 1.0,
        head: Some(Head {
            radius: 0.3,
            height: 1.1,
            forward: 0.3,
        }),
    };
    let id = actor_with(&mut world, Vec3::new(0.0, 0.0, 3.0), collider, 2.1);
    let north = std::f32::consts::PI;
    world.apply(Intent::Teleport {
        actor: id,
        position: Vec3::new(0.0, 0.0, -0.5),
        yaw: north,
    });
    world.step(&mut Vec::new());
    let a = world.actor(id);
    assert!(
        world
            .actor_overlap(&collider, a.position, a.yaw)
            .is_none_or(|(_, separation)| separation > -1e-3),
        "{a:?}"
    );
}

#[test]
fn a_posed_actor_goes_through_a_sill_untouched_then_walks_with_collision() {
    let mut room = Room::default();
    room.floor(20.0).add(
        "sill",
        Owned::Box {
            center: Vec3::new(0.0, 0.45, -1.0),
            half: Vec3::new(1.0, 0.45, 0.2),
            rotation: Mat3::IDENTITY,
        },
    );
    let mut world = room.world(Settings::default());
    let id = actor(&mut world, Vec3::new(0.0, 0.0, 0.5));
    let bystander = actor(&mut world, Vec3::new(0.2, 0.0, 0.5));
    let north = std::f32::consts::PI;
    let mut events = Vec::new();
    world.step(&mut events);
    let beside = world.actor(bystander).position;
    let pose = |world: &mut sariema_physics::World, position: Vec3| {
        world.apply(Intent::Pose {
            actor: id,
            position,
            yaw: north,
        })
    };

    pose(&mut world, Vec3::new(0.2, 0.0, 0.5));
    world.step(&mut events);
    assert_eq!(world.actor(id).position, Vec3::new(0.2, 0.0, 0.5));
    let moved = world.actor(bystander).position - beside;
    assert!(moved.length() < 1e-4, "the bystander was pushed by {moved}");
    for z in [-0.6, -1.0, -1.4] {
        pose(&mut world, Vec3::new(0.0, 0.5, z));
        world.step(&mut events);
        assert_eq!(world.actor(id).position, Vec3::new(0.0, 0.5, z));
    }
    world.step(&mut events);
    assert_eq!(world.actor(id).position, Vec3::new(0.0, 0.5, -1.4));

    pose(&mut world, Vec3::new(0.0, 0.0, -1.6));
    world.step(&mut events);
    hold(&mut world, id, Vec2::new(0.0, -1.0), 30, &mut events);
    let walked = world.actor(id);
    assert!(walked.grounded && walked.position.z < -2.4, "{walked:?}");
    hold(&mut world, id, Vec2::new(0.0, 1.0), 120, &mut events);
    let back = world.actor(id);
    assert!(
        back.position.z < -1.49,
        "walked back into the sill: {back:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(
            e.kind,
            EventKind::Landed { .. } | EventKind::TeleportRejected
        )),
        "{events:?}"
    );
}

#[test]
fn turning_towards_a_wall_never_swings_the_head_into_it() {
    for stick in [
        Vec2::new(0.0, -1.0),
        Vec2::new(1.0, -1.0).normalize(),
        Vec2::new(15f32.to_radians().cos(), -15f32.to_radians().sin()),
    ] {
        let (mut world, id) = facing_east_by_a_wall();
        world.apply(Intent::Move { actor: id, stick });
        let deepest = deepest_over(&mut world, id, 120);
        assert!(deepest > -1e-3, "stick {stick}: head {deepest} m inside");
    }
}

#[test]
fn turning_to_face_the_wall_on_arrival_keeps_the_head_out() {
    let (mut world, id) = facing_east_by_a_wall();
    world.apply(Intent::MoveTo {
        actor: id,
        target: Vec3::new(0.3, 0.0, 0.0),
        face: Some(std::f32::consts::PI),
    });
    let deepest = deepest_over(&mut world, id, 120);
    assert!(deepest > -1e-3, "head {deepest} m inside");
    let a = world.actor(id);
    assert!(
        (a.yaw.abs() - std::f32::consts::PI).abs() < 0.05,
        "never turned to the wall: {a:?}"
    );
}
