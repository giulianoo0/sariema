// SPDX-License-Identifier: Apache-2.0

//! A ray against the room's colliders and the actors' bodies, as filtered.

mod common;

use common::{Owned, Room, actor, settle};
use glam::{Vec2, Vec3};
use sariema_physics::{RayFilter, RayTarget, Settings, World};

/// A floor, a wall across x = 3 (2.9 to 3.1), a post of radius 0.4 at
/// x = -3, and an actor of radius 0.3 at x = 1.5.
fn room() -> (World, sariema_physics::ActorId) {
    let mut room = Room::default();
    room.floor(14.0)
        .wall(
            "east",
            Vec2::new(3.0, 0.0),
            Vec3::new(6.0, 2.5, 0.2),
            std::f32::consts::FRAC_PI_2,
        )
        .add(
            "post",
            Owned::Capsule {
                a: Vec3::new(-3.0, 0.4, 0.0),
                b: Vec3::new(-3.0, 2.0, 0.0),
                radius: 0.4,
            },
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
        world.cast_ray(FROM, Vec3::NEG_X, 2.5, RayFilter::default()),
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

/// The answer of a ray from `origin` along `dir`, as what it hit and how far.
fn along(world: &World, origin: Vec3, dir: Vec3) -> Option<(RayTarget, f32)> {
    (world.cast_ray(origin, dir, 10.0, RayFilter::default())).map(|h| (h.target, h.distance))
}

fn near(got: Option<(RayTarget, f32)>, target: RayTarget, distance: f32) -> bool {
    got.is_some_and(|(t, d)| t == target && (d - distance).abs() < 1e-3)
}

#[test]
fn a_ray_from_inside_an_actor_ignores_it_whichever_way_it_goes() {
    let (world, id) = room();
    let inside = Vec3::new(1.6, 0.5, 0.0);
    let wall = along(&world, inside, Vec3::X).expect("the wall");
    assert!((wall.1 - 1.3).abs() < 1e-3, "{wall:?}");
    let back = along(&world, inside, Vec3::NEG_X);
    assert!(
        back.is_none_or(|(t, _)| t != RayTarget::Actor(id)),
        "{back:?}"
    );
    assert!(near(back, post(&world), 4.2), "{back:?}");
}

#[test]
fn a_ray_from_inside_a_wall_or_a_post_ignores_it_whichever_way_it_goes() {
    let (world, id) = room();
    let in_wall = Vec3::new(3.0, 0.5, 0.0);
    assert_eq!(along(&world, in_wall, Vec3::X), None);
    assert!(near(
        along(&world, in_wall, Vec3::NEG_X),
        RayTarget::Actor(id),
        1.2
    ));

    let in_post = Vec3::new(-3.1, 0.5, 0.0);
    assert_eq!(along(&world, in_post, Vec3::NEG_X), None);
    assert!(near(
        along(&world, in_post, Vec3::X),
        RayTarget::Actor(id),
        4.3
    ));
}

/// The post's collider, as a ray from outside hits it.
fn post(world: &World) -> RayTarget {
    let from = Vec3::new(-1.0, 0.5, 0.0);
    along(world, from, Vec3::NEG_X).expect("the post").0
}
