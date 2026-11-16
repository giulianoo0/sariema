// SPDX-License-Identifier: Apache-2.0

mod common;

use common::{Owned, Room, actor, settle};
use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{
    ActorCollider, ActorId, BlockReason, EventKind, Intent, Settings, World, wrap_angle,
};

/// A 12 m square room cut in two by a wall along z = 0 with a 0.8 m door at
/// x = 2, a 0.15 m dais, a crate, and a closed pen nobody can enter.
pub fn fixture() -> Room {
    let mut room = Room::default();
    room.floor(14.0)
        .wall(
            "north",
            Vec2::new(0.0, -6.1),
            Vec3::new(12.4, 2.5, 0.2),
            0.0,
        )
        .wall("south", Vec2::new(0.0, 6.1), Vec3::new(12.4, 2.5, 0.2), 0.0)
        .wall(
            "west",
            Vec2::new(-6.1, 0.0),
            Vec3::new(12.4, 2.5, 0.2),
            std::f32::consts::FRAC_PI_2,
        )
        .wall(
            "east",
            Vec2::new(6.1, 0.0),
            Vec3::new(12.4, 2.5, 0.2),
            std::f32::consts::FRAC_PI_2,
        )
        .wall(
            "split_w",
            Vec2::new(-2.2, 0.0),
            Vec3::new(7.6, 2.5, 0.2),
            0.0,
        )
        .wall(
            "split_e",
            Vec2::new(4.2, 0.0),
            Vec3::new(3.6, 2.5, 0.2),
            0.0,
        )
        .add(
            "dais",
            Owned::Box {
                center: Vec3::new(-3.5, 0.075, -3.5),
                half: Vec3::new(1.5, 0.075, 1.5),
                rotation: Mat3::IDENTITY,
            },
        )
        .add(
            "crate",
            Owned::Box {
                center: Vec3::new(1.0, 0.5, 3.0),
                half: Vec3::splat(0.5),
                rotation: Mat3::from_rotation_y(0.4),
            },
        )
        .wall("pen_n", Vec2::new(4.0, -4.0), Vec3::new(2.0, 1.5, 0.1), 0.0)
        .wall("pen_s", Vec2::new(4.0, -2.0), Vec3::new(2.0, 1.5, 0.1), 0.0)
        .wall(
            "pen_w",
            Vec2::new(3.0, -3.0),
            Vec3::new(2.0, 1.5, 0.1),
            std::f32::consts::FRAC_PI_2,
        )
        .wall(
            "pen_e",
            Vec2::new(5.0, -3.0),
            Vec3::new(2.0, 1.5, 0.1),
            std::f32::consts::FRAC_PI_2,
        );
    room
}

/// Runs until the scripted move ends; checks every step against the actor's
/// travel limits and returns the event that ended it.
fn walk_to(world: &mut World, id: ActorId, target: Vec3, face: f32) -> EventKind {
    world.apply(Intent::MoveTo {
        actor: id,
        target,
        face: Some(face),
    });
    let collider = ActorCollider::default();
    let travel = 2.1 / 60.0;
    let mut events = Vec::new();
    for _ in 0..1800 {
        events.clear();
        world.step(&mut events);
        let a = world.actor(id);
        let d = a.position - a.previous_position;
        assert!(
            Vec2::new(d.x, d.z).length() <= travel + 1e-4,
            "jumped {d} to {}",
            a.position
        );
        assert!(d.y.abs() <= 0.2 + 1e-3, "jumped {d} to {}", a.position);
        assert!(
            world.actor_overlap(&collider, a.position, a.yaw).is_none(),
            "inside at {}",
            a.position
        );
        if let Some(end) = events
            .iter()
            .find(|e| matches!(e.kind, EventKind::Arrived | EventKind::Blocked(_)))
        {
            return end.kind;
        }
    }
    panic!("no end to the move towards {target}: {:?}", world.actor(id));
}

#[test]
fn move_to_reaches_every_spot_without_jumps() {
    let mut world = fixture().world(Settings::default());
    let id = actor(&mut world, Vec3::new(-5.0, 0.0, 5.0));
    settle(&mut world, id);
    let spots = [
        (Vec3::new(5.0, 0.0, 5.0), 0.0),
        (Vec3::new(2.0, 0.0, -1.0), std::f32::consts::PI),
        (Vec3::new(-3.5, 0.16, -3.5), 1.0),
        (Vec3::new(1.0, 0.0, 4.0), -2.0),
        (Vec3::new(5.4, 0.0, -5.4), 2.5),
        (Vec3::new(-5.0, 0.0, 5.0), 0.0),
    ];
    for (target, face) in spots {
        assert_eq!(
            walk_to(&mut world, id, target, face),
            EventKind::Arrived,
            "towards {target}"
        );
        let a = world.actor(id);
        assert!(
            Vec2::new(a.position.x - target.x, a.position.z - target.z).length() <= 0.02 + 1e-4,
            "{a:?}"
        );
        assert!(wrap_angle(a.yaw - face).abs() <= 0.05, "{a:?}");
        assert!(!a.moving_to);
    }
}

#[test]
fn an_unreachable_spot_is_blocked() {
    let mut world = fixture().world(Settings::default());
    let id = actor(&mut world, Vec3::new(-5.0, 0.0, 5.0));
    settle(&mut world, id);
    let start = world.actor(id).position;
    assert_eq!(
        walk_to(&mut world, id, Vec3::new(4.0, 0.0, -3.0), 0.0),
        EventKind::Blocked(BlockReason::NoPath)
    );
    assert_eq!(world.actor(id).position, start);
    let mut path = Vec::new();
    assert!(!world.find_path(id, start, Vec3::new(4.0, 0.0, -3.0), &mut path));
    assert!(world.find_path(id, start, Vec3::new(5.0, 0.0, -5.0), &mut path));
    assert_eq!(*path.last().unwrap(), Vec3::new(5.0, 0.0, -5.0));
}

#[test]
fn a_door_closed_after_planning_ends_in_blocked() {
    let mut room = fixture();
    room.add(
        "door_leaf",
        Owned::Box {
            center: Vec3::new(2.0, 1.0, 0.0),
            half: Vec3::new(0.45, 1.0, 0.05),
            rotation: Mat3::IDENTITY,
        },
    );
    let mut world = room.world(Settings::default());
    let leaf = world.collider_by_name("door_leaf").unwrap();
    world.apply(Intent::SetColliderEnabled {
        collider: leaf,
        enabled: false,
    });
    let id = actor(&mut world, Vec3::new(2.0, 0.0, 5.0));
    settle(&mut world, id);
    world.apply(Intent::MoveTo {
        actor: id,
        target: Vec3::new(2.0, 0.0, -4.0),
        face: None,
    });
    let mut events = Vec::new();
    world.step(&mut events);
    world.apply(Intent::SetColliderEnabled {
        collider: leaf,
        enabled: true,
    });
    let mut end = None;
    for _ in 0..1200 {
        world.step(&mut events);
        if let Some(e) = events
            .iter()
            .find(|e| matches!(e.kind, EventKind::Arrived | EventKind::Blocked(_)))
        {
            end = Some(e.kind);
            break;
        }
    }
    assert_eq!(end, Some(EventKind::Blocked(BlockReason::Stuck)));
    assert!(world.actor(id).position.z > 0.0);
    let mut path = Vec::new();
    assert!(!world.find_path(
        id,
        world.actor(id).position,
        Vec3::new(2.0, 0.0, -4.0),
        &mut path
    ));
}
