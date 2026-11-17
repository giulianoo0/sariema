// SPDX-License-Identifier: Apache-2.0

//! A scripted move onto a spot another actor holds always ends: the walker
//! that cannot push it out of the way is blocked rather than left walking.

mod common;

use common::{Room, actor};
use glam::{Vec2, Vec3};
use sariema_physics::{ActorId, BlockReason, EventKind, Intent, Settings, World};

const TARGET: Vec3 = Vec3::new(0.0, 0.0, -1.0);

/// A walker 3 m from `TARGET`, where another actor stands with a wall
/// right behind it.
fn against_a_wall(settings: Settings) -> (World, ActorId, ActorId) {
    let mut room = Room::default();
    room.floor(14.0).wall(
        "behind",
        Vec2::new(0.0, -1.5),
        Vec3::new(6.0, 2.5, 0.2),
        0.0,
    );
    let mut world = room.world(settings);
    let walker = actor(&mut world, Vec3::new(0.0, 0.0, 2.0));
    let other = actor(&mut world, TARGET);
    (world, walker, other)
}

/// How the walk ends, and the step it does.
fn walk(world: &mut World, walker: ActorId, steps: u32) -> Option<(EventKind, u32)> {
    world.apply(Intent::MoveTo {
        actor: walker,
        target: TARGET,
        face: None,
    });
    let mut events = Vec::new();
    (0..steps).find_map(|step| {
        events.clear();
        world.step(&mut events);
        (events.iter())
            .find(|e| {
                e.actor == walker && matches!(e.kind, EventKind::Arrived | EventKind::Blocked(_))
            })
            .map(|e| (e.kind, step))
    })
}

#[test]
fn a_walk_onto_an_actor_held_against_a_wall_ends_blocked() {
    let settings = Settings::default();
    let stuck_steps = settings.move_to.stuck_steps;
    let (mut world, walker, _) = against_a_wall(settings);
    // About 2.4 m at 2.1 m/s to reach the other actor, then the window.
    let within = 70 + 2 * stuck_steps;
    let end = walk(&mut world, walker, 1800);
    assert!(
        matches!(end, Some((EventKind::Blocked(BlockReason::Stuck), step)) if step <= within),
        "{end:?}, not blocked by step {within}, walker at {}",
        world.actor(walker).position
    );
}
