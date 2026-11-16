// SPDX-License-Identifier: Apache-2.0

//! An actor whose body is off is in no other actor's way: others walk
//! through it without pushing it, and a hinged leaf swings through it. It
//! still stands on the floor and stops at walls.

mod common;

use std::f32::consts::FRAC_PI_2;

use common::{Owned, Room, actor, hold, settle};
use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{Intent, Pose, Settings};

#[test]
fn an_actor_walks_through_one_whose_body_is_off_and_pushes_one_whose_body_is_on() {
    for solid in [false, true] {
        let mut room = Room::default();
        room.floor(20.0)
            .wall("east", Vec2::new(6.0, 0.0), Vec3::new(0.2, 2.0, 6.0), 0.0);
        let mut world = room.world(Settings::default());
        let walker = actor(&mut world, Vec3::ZERO);
        let other = actor(&mut world, Vec3::new(2.0, 0.0, 0.0));
        settle(&mut world, walker);
        settle(&mut world, other);
        world.apply(Intent::SetSolid {
            actor: other,
            solid,
        });
        let stood = world.actor(other).position;
        let mut events = Vec::new();
        let walked = hold(&mut world, walker, Vec2::X, 120, &mut events);
        let moved = world.actor(other).position.distance(stood);
        if solid {
            assert!(moved > 0.1, "a body that is on was not pushed: {moved}");
        } else {
            assert_eq!(world.actor(other).position, stood);
            assert!(walked.last().unwrap().x > 3.0, "{:?}", walked.last());
        }
        world.apply(Intent::Move {
            actor: other,
            stick: Vec2::X,
        });
        for _ in 0..240 {
            world.step(&mut events);
        }
        let at = world.actor(other).position;
        let skin = Settings::default().collision.skin;
        assert!(
            at.x <= 5.9 - 0.3 && at.y <= skin + 1e-6,
            "solid {solid}: {at}"
        );
    }
}

#[test]
fn a_leaf_swings_through_an_actor_whose_body_is_off() {
    let pivot = Vec3::new(-0.5, 0.0, 0.0);
    let mut room = Room::default();
    room.floor(20.0).platform(
        "leaf",
        Owned::Box {
            center: Vec3::new(0.49, 1.0, 0.0),
            half: Vec3::new(0.47, 1.0, 0.025),
            rotation: Mat3::IDENTITY,
        },
        Pose {
            translation: pivot,
            yaw: FRAC_PI_2,
        },
    );
    let mut world = room.world(Settings::default());
    let leaf = world.collider_by_name("leaf").unwrap();
    let id = actor(&mut world, Vec3::new(0.0, 0.0, -0.45));
    settle(&mut world, id);
    world.apply(Intent::SetSolid {
        actor: id,
        solid: false,
    });
    let mut events = Vec::new();
    for k in 1..=60 {
        let yaw = FRAC_PI_2 * (1.0 - k as f32 / 60.0);
        let turned = Pose {
            translation: pivot,
            yaw,
        };
        assert_eq!(world.try_platform_pose(leaf, turned), Ok(()), "step {k}");
        world.step(&mut events);
    }
}
