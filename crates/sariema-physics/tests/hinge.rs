// SPDX-License-Identifier: Apache-2.0

//! A door leaf turning on its hinge: what it lets through in each state,
//! and how it stops at an actor in the way instead of pushing it.

mod common;

use std::f32::consts::FRAC_PI_2;

use common::{Owned, Room, actor, settle};
use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{ActorId, ColliderId, Intent, Pose, Settings, World};

/// The hinge, on the doorway's west jamb.
const PIVOT: Vec3 = Vec3::new(-0.5, 0.0, 0.0);

/// A wall along x at z = 0 with a doorway from x −0.5 to 0.5, and its leaf
/// turned `yaw` about the west jamb: 0 closes the doorway, π/2 swings it
/// open to the north (−z).
fn doorway(yaw: f32) -> (World, ColliderId) {
    let mut room = Room::default();
    room.floor(20.0)
        .wall("west", Vec2::new(-3.5, 0.0), Vec3::new(6.0, 2.2, 0.2), 0.0)
        .wall("east", Vec2::new(3.5, 0.0), Vec3::new(6.0, 2.2, 0.2), 0.0)
        .platform(
            "leaf",
            Owned::Box {
                center: Vec3::new(0.49, 1.0, 0.0),
                half: Vec3::new(0.47, 1.0, 0.025),
                rotation: Mat3::IDENTITY,
            },
            Pose {
                translation: PIVOT,
                yaw,
            },
        );
    let world = room.world(Settings::default());
    let leaf = world.collider_by_name("leaf").unwrap();
    (world, leaf)
}

fn turned(yaw: f32) -> Pose {
    Pose {
        translation: PIVOT,
        yaw,
    }
}

fn clear(world: &World, id: ActorId) -> bool {
    let a = world.actor(id);
    world
        .actor_overlap(&world.actor_collider(id), a.position, a.yaw)
        .is_none()
}

/// Walks the actor north to south (or back) with the stick for `steps`.
fn walk(world: &mut World, id: ActorId, south: bool, steps: usize) -> Vec3 {
    let stick = Vec2::new(0.0, if south { 1.0 } else { -1.0 });
    world.apply(Intent::Move { actor: id, stick });
    let mut events = Vec::new();
    for _ in 0..steps {
        world.step(&mut events);
        assert!(clear(world, id), "{:?}", world.actor(id));
    }
    world.actor(id).position
}

#[test]
fn a_closed_leaf_blocks_the_doorway_and_an_open_one_lets_an_actor_through() {
    let (mut world, _) = doorway(0.0);
    let id = actor(&mut world, Vec3::new(0.0, 0.0, -2.0));
    settle(&mut world, id);
    let stopped = walk(&mut world, id, true, 180);
    assert!(stopped.z < -0.3, "went through the closed leaf: {stopped}");
    assert!(stopped.z > -0.4, "stopped short of the leaf: {stopped}");

    let (mut world, _) = doorway(FRAC_PI_2);
    let id = actor(&mut world, Vec3::new(0.0, 0.0, -2.0));
    settle(&mut world, id);
    let through = walk(&mut world, id, true, 180);
    assert!(through.z > 1.5, "the open leaf held it back: {through}");
}

#[test]
fn a_leaf_turning_onto_an_actor_in_the_doorway_stops_and_names_it() {
    let (mut world, leaf) = doorway(FRAC_PI_2);
    let id = actor(&mut world, Vec3::new(0.0, 0.0, -0.45));
    settle(&mut world, id);
    let standing = world.actor(id).position;
    let mut events = Vec::new();
    let mut stopped = None;
    for k in 1..=60 {
        let yaw = FRAC_PI_2 * (1.0 - k as f32 / 60.0);
        match world.try_platform_pose(leaf, turned(yaw)) {
            Ok(()) => assert!(stopped.is_none(), "turned on past the actor at step {k}"),
            Err(who) => {
                assert_eq!(who, id);
                stopped.get_or_insert(k);
            }
        }
        world.step(&mut events);
        assert!(clear(&world, id), "step {k}: {:?}", world.actor(id));
        assert!(
            (world.actor(id).position - standing).length() < 1e-4,
            "step {k}: pushed from {standing} to {}",
            world.actor(id).position
        );
    }
    let at = world.platform_pose(leaf).unwrap().yaw;
    assert!(stopped.is_some(), "never met the actor");
    assert!(at > 0.3 && at < 1.4, "stopped at {at} rad");

    world.apply(Intent::Teleport {
        actor: id,
        position: Vec3::new(0.0, 0.0, 2.0),
        yaw: 0.0,
    });
    world.step(&mut events);
    assert_eq!(world.try_platform_pose(leaf, turned(0.0)), Ok(()));
}

#[test]
fn a_leaf_closing_on_an_actor_walking_through_waits_for_it_and_never_pushes_it() {
    let (mut world, leaf) = doorway(FRAC_PI_2);
    let id = actor(&mut world, Vec3::new(0.0, 0.0, -0.8));
    settle(&mut world, id);
    world.apply(Intent::Move {
        actor: id,
        stick: Vec2::new(0.0, 1.0),
    });
    let mut events = Vec::new();
    let (mut turned_steps, mut waited) = (0u32, 0u32);
    let mut last = world.actor(id).position;
    for k in 1..=240 {
        let next = (turned_steps + 1).min(40);
        let yaw = FRAC_PI_2 * (1.0 - next as f32 / 40.0);
        match world.try_platform_pose(leaf, turned(yaw)) {
            Ok(()) => turned_steps = next,
            Err(who) => {
                assert_eq!(who, id);
                waited += 1;
            }
        }
        world.step(&mut events);
        let now = world.actor(id).position;
        assert!(clear(&world, id), "step {k}: {:?}", world.actor(id));
        assert!(
            now.z >= last.z - 1e-4,
            "step {k}: pushed back from {last} to {now}"
        );
        last = now;
    }
    assert!(waited > 0, "the leaf never met the actor");
    assert!(last.z > 1.0, "the actor did not get through: {last}");
    assert_eq!(turned_steps, 40, "the leaf did not finish closing");
    assert_eq!(world.platform_pose(leaf).unwrap().yaw, 0.0);
}

#[test]
fn a_leaf_set_across_an_actor_in_one_go_is_refused() {
    let (mut world, leaf) = doorway(FRAC_PI_2);
    let id = actor(&mut world, Vec3::new(0.0, 0.0, -0.45));
    settle(&mut world, id);
    assert_eq!(world.try_platform_pose(leaf, turned(-FRAC_PI_2)), Err(id));
    assert_eq!(world.platform_pose(leaf), Some(turned(FRAC_PI_2)));
}
