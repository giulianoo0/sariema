// SPDX-License-Identifier: Apache-2.0

//! A pose is part of the state: it decides whether the actor falls, is
//! pushed out or stands in others' way at the next step, so two worlds
//! alike but for a pose hash apart.

mod common;

use common::{Room, actor, settle};
use glam::Vec3;
use sariema_physics::{Intent, Settings};

#[test]
fn a_posed_actor_hashes_apart_from_the_same_actor_standing() {
    let mut room = Room::default();
    room.floor(10.0);
    let worlds = [false, true].map(|posed| {
        let mut world = room.world(Settings::default());
        let id = actor(&mut world, Vec3::ZERO);
        settle(&mut world, id);
        if posed {
            let a = world.actor(id);
            world.apply(Intent::Pose {
                actor: id,
                position: a.position,
                yaw: a.yaw,
            });
        }
        world.step(&mut Vec::new());
        world
    });
    let [standing, posed] = &worlds;
    let at = |w: &sariema_physics::World| w.actor(sariema_physics::ActorId(0)).position;
    assert_eq!(at(standing), at(posed), "the two stand alike");
    assert_ne!(standing.state_hash(), posed.state_hash());
}
