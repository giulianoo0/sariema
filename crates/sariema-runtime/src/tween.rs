// SPDX-License-Identifier: Apache-2.0

//! Tweens and springs on numeric properties. At most one animation drives a
//! property at a time; a new one starts from the property's current value
//! (and a spring keeps the velocity it had), so interrupting never jumps.

use sariema_core::STEP_SECONDS;
use sariema_format::script::{Ease, Property};

use crate::ease::ease;
use crate::scenery::Scenery;
use crate::spring::Spring;
use crate::store::Store;
use crate::world::{Entity, Reject, World};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub entity: Entity,
    pub property: Property,
    pub var: u32,
}

impl Target {
    fn same(&self, other: &Target) -> bool {
        self.property == other.property
            && if self.named() {
                self.var == other.var
            } else {
                self.entity == other.entity
            }
    }

    /// Whether `var` names what it drives, instead of `entity`.
    pub fn named(&self) -> bool {
        matches!(self.property, Property::Var | Property::Scene)
    }
}

/// What a tween or spring drives, besides entities.
pub struct Values<'a> {
    pub store: &'a mut Store,
    pub scenery: &'a mut Scenery,
}

pub fn read(world: &World, store: &Store, scenery: &Scenery, t: &Target) -> Result<f32, Reject> {
    match t.property {
        Property::Var => return Ok(store.var(t.var)),
        Property::Scene => return scenery.param(t.var).map_err(|_| Reject::Stale),
        _ => {}
    }
    let local = world.node(t.entity).ok_or(Reject::Stale)?.local;
    Ok(match t.property {
        Property::PositionX => local.position.x,
        Property::PositionY => local.position.y,
        Property::PositionZ => local.position.z,
        Property::Yaw => local.yaw,
        Property::Pitch => local.pitch,
        Property::Roll => local.roll,
        Property::Scale => local.scale,
        Property::Var | Property::Scene => unreachable!(),
    })
}

pub fn write(world: &mut World, values: &mut Values, t: &Target, v: f32) -> Result<(), Reject> {
    match t.property {
        Property::Var => {
            values.store.vars.insert(t.var, v);
            return Ok(());
        }
        Property::Scene => {
            return values
                .scenery
                .set_param(t.var, v)
                .map_err(|_| Reject::Stale);
        }
        _ => {}
    }
    let local = &mut world.node_mut(t.entity)?.local;
    match t.property {
        Property::PositionX => local.position.x = v,
        Property::PositionY => local.position.y = v,
        Property::PositionZ => local.position.z = v,
        Property::Yaw => local.yaw = v,
        Property::Pitch => local.pitch = v,
        Property::Roll => local.roll = v,
        Property::Scale => local.scale = v,
        Property::Var | Property::Scene => unreachable!(),
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drive {
    Tween {
        from: f32,
        to: f32,
        steps: u32,
        elapsed: u32,
        ease: Ease,
    },
    Spring {
        to: f32,
        velocity: f32,
        spring: Spring,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Anim {
    id: u32,
    target: Target,
    drive: Drive,
    /// The value written last step, for the velocity a spring inherits.
    last: f32,
    /// Kept where it is for this step.
    held: bool,
}

impl Anim {
    /// Moves the drive on a step: the value to write and whether it is the last.
    fn advance(&mut self, rest: f32) -> (f32, bool) {
        match &mut self.drive {
            Drive::Tween {
                from,
                to,
                steps,
                elapsed,
                ease: curve,
            } => {
                *elapsed += 1;
                if *elapsed >= *steps {
                    (*to, true)
                } else {
                    let t = *elapsed as f32 / *steps as f32;
                    (*from + (*to - *from) * ease(*curve, t), false)
                }
            }
            Drive::Spring {
                to,
                velocity,
                spring,
            } => {
                let (x, v) = spring.step(self.last, *velocity, *to);
                *velocity = v;
                if (x - *to).abs() <= rest && v.abs() <= rest {
                    (*to, true)
                } else {
                    (x, false)
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Tweens {
    list: Vec<Anim>,
}

/// Steps a duration in seconds lasts: at least one.
pub fn steps_of(seconds: f32) -> u32 {
    ((seconds / STEP_SECONDS).round() as u32).max(1)
}

impl Tweens {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn take(&mut self, id: u32, target: &Target) -> Option<Anim> {
        let at = self
            .list
            .iter()
            .position(|a| a.id == id || a.target.same(target))?;
        Some(self.list.remove(at))
    }

    pub fn tween(
        &mut self,
        (world, store, scenery): (&World, &Store, &Scenery),
        id: u32,
        target: Target,
        to: f32,
        duration: f32,
        ease: Ease,
    ) -> Result<(), Reject> {
        let from = read(world, store, scenery, &target)?;
        self.take(id, &target);
        self.list.push(Anim {
            id,
            target,
            drive: Drive::Tween {
                from,
                to,
                steps: steps_of(duration),
                elapsed: 0,
                ease,
            },
            last: from,
            held: false,
        });
        Ok(())
    }

    pub fn spring(
        &mut self,
        (world, store, scenery): (&World, &Store, &Scenery),
        id: u32,
        target: Target,
        to: f32,
        frequency: f32,
        damping: f32,
    ) -> Result<(), Reject> {
        let now = read(world, store, scenery, &target)?;
        let velocity = self.velocity(id, &target, now);
        self.take(id, &target);
        self.list.push(Anim {
            id,
            target,
            drive: Drive::Spring {
                to,
                velocity,
                spring: Spring::new(frequency, damping, STEP_SECONDS),
            },
            last: now,
            held: false,
        });
        Ok(())
    }

    /// The velocity a spring on `target`, or with `id`, starts with: the one
    /// a spring there had, or the rate the animation there moved at.
    pub fn velocity(&self, id: u32, target: &Target, now: f32) -> f32 {
        match self
            .list
            .iter()
            .find(|a| a.id == id || a.target.same(target))
        {
            Some(Anim {
                drive: Drive::Spring { velocity, .. },
                ..
            }) => *velocity,
            Some(old) => (now - old.last) / STEP_SECONDS,
            None => 0.0,
        }
    }

    /// What the animation of `target` writes in the next [`Tweens::step`],
    /// if one drives it.
    pub fn peek(&self, target: &Target, rest: f32) -> Option<f32> {
        let mut anim = *self.list.iter().find(|a| a.target.same(target))?;
        Some(anim.advance(rest).0)
    }

    /// Keeps the animation of `target` where it is for the next step: it
    /// writes nothing then and goes on from there after, so it ends a step
    /// later; a spring loses its velocity.
    pub fn hold(&mut self, target: &Target) {
        if let Some(a) = self.list.iter_mut().find(|a| a.target.same(target)) {
            a.held = true;
            if let Drive::Spring { velocity, .. } = &mut a.drive {
                *velocity = 0.0;
            }
        }
    }

    pub fn active(&self, id: u32) -> bool {
        self.list.iter().any(|a| a.id == id)
    }

    pub fn cancel(&mut self, id: u32) {
        self.list.retain(|a| a.id != id);
    }

    /// Drops every animation of `entity`'s placement; whether there was any.
    pub fn forget(&mut self, entity: Entity) -> bool {
        let before = self.list.len();
        self.list
            .retain(|a| a.target.named() || a.target.entity != entity);
        self.list.len() != before
    }

    /// Advances every animation one step; `done` receives each finished id.
    /// A spring finishes once both its offset and its velocity are within
    /// `rest`, landing exactly on its target.
    pub fn step(
        &mut self,
        world: &mut World,
        values: &mut Values,
        rest: f32,
        mut done: impl FnMut(u32),
    ) {
        let mut kept = 0;
        for i in 0..self.list.len() {
            let mut anim = self.list[i];
            if anim.held {
                anim.held = false;
                self.list[kept] = anim;
                kept += 1;
                continue;
            }
            let (value, finished) = anim.advance(rest);
            anim.last = value;
            let alive = write(world, values, &anim.target, value).is_ok();
            if finished && alive {
                done(anim.id);
            }
            if !finished && alive {
                self.list[kept] = anim;
                kept += 1;
            }
        }
        self.list.truncate(kept);
    }
}
