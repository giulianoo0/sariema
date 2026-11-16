// SPDX-License-Identifier: Apache-2.0

//! Moving platforms, actors pushing each other, and the queries.

mod common;

use common::{Owned, Room, actor, hold, settle};
use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{ActorDesc, ColliderId, Intent, Pose, Settings, yaw_matrix};

#[test]
fn platforms_carry_an_actor_without_drift() {
    let start = Pose {
        translation: Vec3::new(0.0, 1.0, 0.0),
        yaw: 0.0,
    };
    let mut room = Room::default();
    room.floor(40.0).platform(
        "disc",
        Owned::Box {
            center: Vec3::ZERO,
            half: Vec3::new(1.5, 0.1, 1.5),
            rotation: Mat3::IDENTITY,
        },
        start,
    );
    let mut world = room.world(Settings::default());
    let disc = ColliderId(1);
    let id = actor(&mut world, Vec3::new(0.7, 1.2, 0.3));
    settle(&mut world, id);
    assert_eq!(world.actor(id).ground, Some(disc));
    let local = |world: &sariema_physics::World| {
        let pose = world.platform_pose(disc).unwrap();
        yaw_matrix(pose.yaw).transpose() * (world.actor(id).position - pose.translation)
    };
    let rest = local(&world);
    let mut events = Vec::new();
    for k in 1..=600 {
        let t = k as f32 / 60.0;
        world.set_platform_pose(
            disc,
            Pose {
                translation: Vec3::new(
                    3.0 * (0.5 * t).sin(),
                    1.0 + 0.8 * (0.7 * t).sin(),
                    2.0 * (0.3 * t).sin(),
                ),
                yaw: 0.9 * t,
            },
        );
        world.step(&mut events);
        let drift = local(&world).distance(rest);
        assert!(drift < 1e-4, "step {k}: drifted {drift}");
        assert!(world.actor(id).grounded, "step {k}");
    }
    assert!((world.actor(id).yaw - sariema_physics::wrap_angle(0.9 * 10.0)).abs() < 1e-3);
}

#[test]
fn actors_push_each_other_apart_and_an_immovable_one_stays() {
    let mut room = Room::default();
    room.floor(20.0);
    let mut world = room.world(Settings::default());
    let a = actor(&mut world, Vec3::new(0.0, 0.0, 0.0));
    let b = actor(&mut world, Vec3::new(0.2, 0.0, 0.0));
    let wall = world
        .add_actor(ActorDesc {
            position: Vec3::new(3.0, 0.0, 0.0),
            mass: f32::INFINITY,
            ..ActorDesc::default()
        })
        .unwrap();
    settle(&mut world, a);
    let (pa, pb) = (world.actor(a).position, world.actor(b).position);
    assert!(pa.distance(pb) >= 0.6 - 1e-4, "{pa} {pb}");
    assert!(
        (pa.x + pb.x - 0.2).abs() < 1e-4,
        "equal masses share the push: {pa} {pb}"
    );

    let mut events = Vec::new();
    hold(&mut world, b, Vec2::new(1.0, 0.0), 120, &mut events);
    let (pb, pw) = (world.actor(b).position, world.actor(wall).position);
    assert_eq!((pw.x, pw.z), (3.0, 0.0));
    assert!(pw.x - pb.x >= 0.6 - 0.04, "{pb} {pw}");
}

#[test]
fn queries_see_what_is_there() {
    let mut room = Room::default();
    room.floor(20.0).add(
        "crate",
        Owned::Box {
            center: Vec3::new(0.0, 0.5, -3.0),
            half: Vec3::splat(0.5),
            rotation: Mat3::IDENTITY,
        },
    );
    let world = room.world(Settings::default());
    let crate_id = world.collider_by_name("crate").unwrap();

    let ray = world
        .raycast(Vec3::new(0.0, 0.5, 0.0), Vec3::NEG_Z, 10.0)
        .unwrap();
    assert_eq!(ray.collider, crate_id);
    assert!(
        (ray.distance - 2.5).abs() < 1e-5 && ray.normal == Vec3::Z,
        "{ray:?}"
    );
    assert!(
        world
            .raycast(Vec3::new(0.0, 0.5, 0.0), Vec3::Z, 10.0)
            .is_none()
    );

    let cast = world
        .shapecast(
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.0, 1.5, 0.0),
            0.3,
            Vec3::new(0.0, 0.0, -5.0),
        )
        .unwrap();
    assert_eq!(cast.collider, crate_id);
    assert!((cast.fraction * 5.0 - 2.2).abs() < 1e-4, "{cast:?}");

    let mut found = Vec::new();
    world.overlap(
        Vec3::new(0.0, 0.5, -2.3),
        Vec3::new(0.0, 0.5, -2.3),
        0.25,
        &mut found,
    );
    assert_eq!(found, [crate_id]);
    world.overlap(
        Vec3::new(0.0, 0.5, -1.0),
        Vec3::new(0.0, 0.5, -1.0),
        0.25,
        &mut found,
    );
    assert!(found.is_empty());

    let near = world.closest_point(Vec3::new(0.0, 0.5, -3.0), 2.0).unwrap();
    assert_eq!(near.collider, crate_id);
    assert!(
        (near.distance + 0.5).abs() < 1e-5,
        "inside the crate: {near:?}"
    );
}

#[test]
fn camera_clearance_reports_occluders_and_a_pull_in_distance() {
    let mut room = Room::default();
    room.floor(30.0)
        .add(
            "root",
            Owned::Capsule {
                a: Vec3::new(-3.0, 1.5, 2.0),
                b: Vec3::new(3.0, 1.5, 2.0),
                radius: 0.4,
            },
        )
        .add(
            "pillar",
            Owned::Box {
                center: Vec3::new(0.0, 1.5, 5.0),
                half: Vec3::new(0.3, 1.5, 0.3),
                rotation: Mat3::IDENTITY,
            },
        );
    let world = room.world(Settings::default());
    let target = Vec3::new(0.0, 1.5, 0.0);
    let camera = Vec3::new(0.0, 1.5, 8.0);
    let mut occluders = Vec::new();
    let pull = world.camera_clearance(target, camera, 0.2, &mut occluders);
    let names: Vec<&str> = occluders
        .iter()
        .map(|o| world.collider_name(o.collider))
        .collect();
    assert_eq!(names, ["root", "pillar"]);
    assert!((occluders[0].distance - 1.4).abs() < 1e-3, "{occluders:?}");
    assert!((pull - (1.4 - 0.01)).abs() < 1e-3, "{pull}");
    let open = world.camera_clearance(target, Vec3::new(0.0, 1.5, -8.0), 0.2, &mut occluders);
    assert!(occluders.is_empty() && (open - 8.0).abs() < 1e-5);
}

#[test]
fn disabling_a_collider_lets_actors_through_and_drops_them() {
    let mut room = Room::default();
    room.floor(20.0)
        .wall("gate", Vec2::new(0.0, 0.0), Vec3::new(6.0, 2.0, 0.2), 0.0)
        .add(
            "trapdoor",
            Owned::Box {
                center: Vec3::new(5.0, 1.0, 5.0),
                half: Vec3::new(1.0, 0.1, 1.0),
                rotation: Mat3::IDENTITY,
            },
        );
    let mut world = room.world(Settings::default());
    let id = actor(&mut world, Vec3::new(0.0, 0.0, 2.0));
    let rider = actor(&mut world, Vec3::new(5.0, 1.2, 5.0));
    settle(&mut world, id);
    assert!((world.actor(rider).position.y - 1.11).abs() < 1e-3);
    let mut events = Vec::new();
    hold(&mut world, id, Vec2::new(0.0, -1.0), 60, &mut events);
    assert!(world.actor(id).position.z > 0.3);
    for name in ["gate", "trapdoor"] {
        world.apply(Intent::SetColliderEnabled {
            collider: world.collider_by_name(name).unwrap(),
            enabled: false,
        });
    }
    hold(&mut world, id, Vec2::new(0.0, -1.0), 60, &mut events);
    assert!(world.actor(id).position.z < -0.5, "{:?}", world.actor(id));
    assert!(
        world.actor(rider).position.y < 0.02,
        "{:?}",
        world.actor(rider)
    );
}
