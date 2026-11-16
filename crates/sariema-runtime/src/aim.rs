// SPDX-License-Identifier: Apache-2.0

//! Where each spot light the game aimed points: a point in the frame of an
//! entity, in metres of the world whatever its scale, so a lantern in the
//! hand lights ahead of the one holding it as they walk and turn; or a
//! point of the world.

use glam::Vec3;

use crate::world::{Entity, World};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Aim {
    light: Entity,
    /// The entity whose frame `target` is in; NONE for the world.
    at: Entity,
    target: Vec3,
}

#[derive(Clone, Debug, Default)]
pub struct Aims {
    list: Vec<Aim>,
}

impl Aims {
    /// `light` points at `target` in `at`'s frame from now on.
    pub fn aim(&mut self, light: Entity, at: Entity, target: Vec3) {
        self.list.retain(|a| a.light != light);
        self.list.push(Aim { light, at, target });
    }

    /// Forgets the aim of `e` and the aims in its frame, which then keep
    /// pointing where they last did.
    pub fn forget(&mut self, e: Entity) {
        self.list.retain(|a| a.light != e && a.at != e);
    }

    /// Where `light` points in the world, drawn at `alpha` between the last
    /// two steps; `None` when the game never aimed it.
    pub fn target(&self, world: &World, light: Entity, alpha: f32) -> Option<Vec3> {
        let aim = self.list.iter().find(|a| a.light == light)?;
        if aim.at.is_none() {
            return Some(aim.target);
        }
        let frame = world.interpolated(aim.at, alpha)?;
        Some(frame.position + frame.rotation * aim.target)
    }
}
