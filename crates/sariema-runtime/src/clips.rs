// SPDX-License-Identifier: Apache-2.0

//! The time of each actor's clip, counted in steps. The animation module
//! seeks every clip to this time instead of running it on its own clock, so
//! a clip slows with the world in slow motion and never drifts from the
//! steps that wait on it; drawn between steps, it never stands still.

use sariema_core::STEP_SECONDS;
use sariema_format::script::Entity;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Playing {
    entity: Entity,
    clip: u32,
    speed: f32,
    steps: u32,
}

#[derive(Clone, Debug, Default)]
pub struct ClipClock {
    list: Vec<Playing>,
}

impl ClipClock {
    /// `clip` starts on `entity` at time 0, replacing what it played.
    pub fn play(&mut self, entity: Entity, clip: u32, speed: f32) {
        self.forget(entity);
        self.list.push(Playing {
            entity,
            clip,
            speed,
            steps: 0,
        });
    }

    pub fn forget(&mut self, entity: Entity) {
        self.list.retain(|p| p.entity != entity);
    }

    /// One more step for every clip started before this one.
    pub fn step(&mut self) {
        for p in &mut self.list {
            p.steps += 1;
        }
    }

    /// The clip `entity` plays and its time in seconds.
    pub fn time(&self, entity: Entity) -> Option<(u32, f32)> {
        self.time_at(entity, 1.0)
    }

    /// The time at `alpha` between the previous step and the last one, as
    /// poses are drawn; never before the clip began.
    pub fn time_at(&self, entity: Entity, alpha: f32) -> Option<(u32, f32)> {
        self.list.iter().find(|p| p.entity == entity).map(|p| {
            let steps = (p.steps as f32 - 1.0 + alpha).max(0.0);
            (p.clip, steps * STEP_SECONDS * p.speed)
        })
    }
}
