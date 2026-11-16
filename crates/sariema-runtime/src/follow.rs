// SPDX-License-Identifier: Apache-2.0

//! Followers: an entity keeps another's world position plus an offset,
//! closing the gap with a half-life, so retargeting never jumps.

use glam::Vec3;
use sariema_core::STEP_SECONDS;

use crate::world::{Entity, Reject, World};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Follower {
    entity: Entity,
    target: Entity,
    offset: Vec3,
    /// Fraction of the remaining gap closed each step.
    gain: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Followers {
    list: Vec<Follower>,
}

/// Fraction of the gap a follower with `lag` closes each step; 1 is rigid.
pub fn gain(lag: f32) -> f32 {
    if lag > 0.0 {
        1.0 - 0.5f32.powf(STEP_SECONDS / lag)
    } else {
        1.0
    }
}

impl Followers {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The followers of `target`, each with the fraction of a move of
    /// `target` it makes in the next step.
    pub fn of(&self, target: Entity) -> impl Iterator<Item = (Entity, f32)> + '_ {
        self.list
            .iter()
            .filter(move |f| f.target == target)
            .map(|f| (f.entity, f.gain))
    }

    /// `lag` is the half-life of the gap in seconds; 0 or less is rigid.
    pub fn follow(
        &mut self,
        world: &World,
        entity: Entity,
        target: Entity,
        offset: Vec3,
        lag: f32,
    ) -> Result<(), Reject> {
        world.check(entity)?;
        world.check(target)?;
        if entity == target {
            return Err(Reject::Cycle);
        }
        self.unfollow(entity);
        self.list.push(Follower {
            entity,
            target,
            offset,
            gain: gain(lag),
        });
        Ok(())
    }

    /// Whether `entity` was following anything.
    pub fn unfollow(&mut self, entity: Entity) -> bool {
        let before = self.list.len();
        self.list.retain(|f| f.entity != entity);
        self.list.len() != before
    }

    pub fn forget(&mut self, entity: Entity) {
        self.list
            .retain(|f| f.entity != entity && f.target != entity);
    }

    /// In the order the follows were made, so a chain settles in one step.
    pub fn step(&self, world: &mut World) {
        for f in &self.list {
            let (Some(target), Some(me)) =
                (world.current_pose(f.target), world.current_pose(f.entity))
            else {
                continue;
            };
            let goal = target.position + f.offset;
            let at = me.position + (goal - me.position) * f.gain;
            let _ = world.set_world_position(f.entity, at);
        }
    }
}
