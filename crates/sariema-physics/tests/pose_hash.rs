// SPDX-License-Identifier: Apache-2.0

//! A hold is part of the state: a held actor keeps its pose until a
//! movement releases it, so two worlds alike but for a hold hash apart. A
//! pose that is not held (a traversal under way, a tween) hashes as it did
//! before holds existed.

mod common;

use common::{Room, actor, settle};
use glam::Vec3;
use sariema_physics::{ActorId, Intent, Settings, World};

/// One actor standing on a floor, then posed where it stands, held or
/// not, and stepped once.
fn worlds() -> [World; 3] {
    let mut room = Room::default();
    room.floor(10.0);
    [None, Some(false), Some(true)].map(|pose| {
        let mut world = room.world(Settings::default());
        let id = actor(&mut world, Vec3::ZERO);
        settle(&mut world, id);
        let a = world.actor(id);
        let (position, yaw) = (a.position, a.yaw);
        match pose {
            Some(true) => world.apply(Intent::Hold {
                actor: id,
                position,
                yaw,
            }),
            Some(false) => world.apply(Intent::Pose {
                actor: id,
                position,
                yaw,
            }),
            None => {}
        }
        world.step(&mut Vec::new());
        world
    })
}

#[test]
fn a_held_actor_hashes_apart_from_the_same_actor_standing() {
    let [standing, _, held] = &worlds();
    let at = |w: &World| w.actor(ActorId(0)).position;
    assert_eq!(at(standing), at(held), "the two stand alike");
    assert_ne!(standing.state_hash(), held.state_hash());
}

#[test]
fn a_pose_that_is_not_held_hashes_as_standing() {
    let [standing, posed, _] = &worlds();
    assert_eq!(standing.state_hash(), posed.state_hash());
}
