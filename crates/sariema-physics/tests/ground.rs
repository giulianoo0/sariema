// SPDX-License-Identifier: Apache-2.0

mod common;

use common::{Owned, Room, actor, hold, settle};
use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{EventKind, Settings};

/// Walks north (−z) for 1.5 s towards a box of `height` whose south face is at z = 0.
fn walk_onto_step(height: f32) -> Vec3 {
    let mut room = Room::default();
    room.floor(20.0).add(
        "step",
        Owned::Box {
            center: Vec3::new(0.0, height * 0.5, -2.0),
            half: Vec3::new(2.0, height * 0.5, 2.0),
            rotation: Mat3::IDENTITY,
        },
    );
    let mut world = room.world(Settings::default());
    let id = actor(&mut world, Vec3::new(0.0, 0.0, 1.5));
    settle(&mut world, id);
    let mut events = Vec::new();
    let positions = hold(&mut world, id, Vec2::new(0.0, -1.0), 90, &mut events);
    for w in positions.windows(2) {
        assert!(
            w[1].y - w[0].y <= 0.2 + 1e-4,
            "rose {} in one step",
            w[1].y - w[0].y
        );
    }
    *positions.last().unwrap()
}

#[test]
fn step_of_0_19_m_climbs_and_0_21_m_blocks() {
    let up = walk_onto_step(0.19);
    assert!(up.z < -1.0 && (up.y - 0.20).abs() < 0.005, "{up}");
    let blocked = walk_onto_step(0.21);
    assert!(blocked.z > 0.29 && blocked.y < 0.02, "{blocked}");
}

/// A ramp rising northwards at `deg`, its foot at z = 0 on the floor.
fn walk_up_ramp(deg: f32) -> Vec3 {
    let angle = deg.to_radians();
    let length = 6.0;
    let thickness = 0.4;
    // A box tilted about x: its top face rises from the foot line northwards.
    let rotation = Mat3::from_rotation_x(angle);
    let foot_to_centre = rotation * Vec3::new(0.0, -thickness * 0.5, -length * 0.5);
    let mut room = Room::default();
    room.floor(30.0).add(
        "ramp",
        Owned::Box {
            center: foot_to_centre,
            half: Vec3::new(2.0, thickness * 0.5, length * 0.5),
            rotation,
        },
    );
    let mut world = room.world(Settings::default());
    let id = actor(&mut world, Vec3::new(0.0, 0.0, 1.5));
    settle(&mut world, id);
    let mut events = Vec::new();
    let positions = hold(&mut world, id, Vec2::new(0.0, -1.0), 150, &mut events);
    *positions.last().unwrap()
}

#[test]
fn slope_of_39_degrees_climbs_and_41_does_not() {
    let up = walk_up_ramp(39.0);
    assert!(up.y > 1.5, "{up}");
    let blocked = walk_up_ramp(41.0);
    assert!(blocked.y < 0.05, "{blocked}");
}

#[test]
fn walking_off_a_ledge_falls_and_lands_once() {
    let mut room = Room::default();
    room.floor(20.0).add(
        "ledge",
        Owned::Box {
            center: Vec3::new(0.0, 0.5, 2.0),
            half: Vec3::new(3.0, 0.5, 2.0),
            rotation: Mat3::IDENTITY,
        },
    );
    let mut world = room.world(Settings::default());
    let id = actor(&mut world, Vec3::new(0.0, 1.0, 1.0));
    settle(&mut world, id);
    let mut events = Vec::new();
    hold(&mut world, id, Vec2::new(0.0, -1.0), 120, &mut events);
    let landed: Vec<f32> = events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::Landed { speed } => Some(speed),
            _ => None,
        })
        .collect();
    assert_eq!(landed.len(), 1, "{events:?}");
    assert!(landed[0] > 3.0, "{landed:?}");
    let end = world.actor(id);
    assert!(
        end.grounded && end.position.y < 0.02 && end.position.z < -1.0,
        "{end:?}"
    );
}

#[test]
fn walking_down_a_gentle_slope_stays_on_the_ground() {
    let angle = 20f32.to_radians();
    let rotation = Mat3::from_rotation_x(-angle);
    let mut room = Room::default();
    room.floor(40.0).add(
        "hill",
        Owned::Box {
            center: rotation * Vec3::new(0.0, -0.2, -3.0) + Vec3::new(0.0, 6.0 * angle.sin(), 0.0),
            half: Vec3::new(2.0, 0.2, 3.0),
            rotation,
        },
    );
    let mut world = room.world(Settings::default());
    let id = actor(&mut world, Vec3::new(0.0, 2.2, -0.3));
    settle(&mut world, id);
    let mut events = Vec::new();
    world.apply(sariema_physics::Intent::Move {
        actor: id,
        stick: Vec2::new(0.0, -1.0),
    });
    for _ in 0..150 {
        world.step(&mut events);
        let a = world.actor(id);
        if a.position.z > -5.5 {
            assert!(a.grounded, "{a:?}");
        }
    }
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.kind, EventKind::Landed { .. })),
        "{events:?}"
    );
}

#[test]
fn an_actor_placed_on_the_floor_stands_and_only_a_drop_lands() {
    let mut room = Room::default();
    room.floor(10.0);
    let mut world = room.world(Settings::default());
    let id = actor(&mut world, Vec3::ZERO);
    assert!(world.actor(id).grounded, "{:?}", world.actor(id));
    let mut events = Vec::new();
    let teleport = |world: &mut sariema_physics::World, y: f32| {
        world.apply(sariema_physics::Intent::Teleport {
            actor: id,
            position: Vec3::new(2.0, y, 2.0),
            yaw: 0.0,
        })
    };
    for _ in 0..10 {
        world.step(&mut events);
    }
    teleport(&mut world, 0.0);
    for _ in 0..10 {
        world.step(&mut events);
    }
    let landings = |events: &[sariema_physics::Event]| {
        events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::Landed { .. }))
            .count()
    };
    assert_eq!(landings(&events), 0, "{events:?}");
    teleport(&mut world, 1.0);
    for _ in 0..60 {
        world.step(&mut events);
    }
    assert_eq!(landings(&events), 1, "{events:?}");
}
