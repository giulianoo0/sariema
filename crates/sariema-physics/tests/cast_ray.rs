// SPDX-License-Identifier: Apache-2.0

//! A ray against the room's colliders and the actors' bodies, as filtered.

mod common;

use common::{Room, actor, settle};
use glam::{Vec2, Vec3};
use sariema_physics::{RayFilter, RayTarget, Settings, World};

/// A floor and a wall across x = 3 (2.9 to 3.1), and an actor at x = 1.5.
fn room() -> (World, sariema_physics::ActorId) {
    let mut room = Room::default();
    room.floor(14.0).wall(
        "east",
        Vec2::new(3.0, 0.0),
        Vec3::new(6.0, 2.5, 0.2),
        std::f32::consts::FRAC_PI_2,
    );
    let mut world = room.world(Settings::default());
    let id = actor(&mut world, Vec3::new(1.5, 0.0, 0.0));
    settle(&mut world, id);
    (world, id)
}

const FROM: Vec3 = Vec3::new(0.0, 0.5, 0.0);

#[test]
fn a_ray_hits_an_actor_first_and_the_wall_behind_it_when_it_is_excluded() {
    let (world, id) = room();
    let hit = world.cast_ray(FROM, Vec3::X, 10.0, RayFilter::default());
    let hit = hit.expect("the actor is in the way");
    assert_eq!(hit.target, RayTarget::Actor(id));
    assert!((hit.distance - 1.2).abs() < 1e-3, "{hit:?}");
    assert!((hit.normal - Vec3::NEG_X).length() < 1e-3, "{hit:?}");

    for filter in [
        RayFilter {
            exclude_actor: Some(id),
            ..RayFilter::default()
        },
        RayFilter {
            actors: false,
            ..RayFilter::default()
        },
    ] {
        let hit = world
            .cast_ray(FROM, Vec3::X, 10.0, filter)
            .expect("the wall");
        assert!(matches!(hit.target, RayTarget::Collider(_)), "{hit:?}");
        assert!((hit.distance - 2.9).abs() < 1e-3, "{hit:?}");
        assert!((hit.point - Vec3::new(2.9, 0.5, 0.0)).length() < 1e-3);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-3, "{hit:?}");
    }
}

#[test]
fn a_ray_misses_what_is_out_of_reach_or_filtered_out() {
    let (world, _) = room();
    assert_eq!(
        world.cast_ray(FROM, Vec3::NEG_X, 5.0, RayFilter::default()),
        None
    );
    assert_eq!(
        world.cast_ray(FROM, Vec3::X, 1.0, RayFilter::default()),
        None
    );
    let nothing = RayFilter {
        colliders: false,
        actors: false,
        ..RayFilter::default()
    };
    assert_eq!(world.cast_ray(FROM, Vec3::X, 10.0, nothing), None);
}

#[test]
fn a_ray_down_hits_the_floor_facing_up() {
    let (world, _) = room();
    let hit = world
        .cast_ray(
            Vec3::new(-2.0, 2.0, 0.0),
            Vec3::NEG_Y,
            5.0,
            RayFilter::default(),
        )
        .expect("the floor");
    assert!((hit.normal - Vec3::Y).length() < 1e-3, "{hit:?}");
    assert!(hit.point.y.abs() < 1e-3, "{hit:?}");
}
