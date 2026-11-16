// SPDX-License-Identifier: Apache-2.0

//! Highlights: a room node or an actor glowing in its own colour, eased in
//! and out without a jump and pulsing slowly while it is on.

use sariema_core::STEP_SECONDS;
use sariema_format::script::Entity;

use super::FxReject;
use super::ramp::Ramp;
use crate::hash::StateHasher;
use crate::world::World;

/// What glows: an entity (an actor, or a room node's entity), or node
/// `node` of room `room`, both by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlowTarget {
    Entity(Entity),
    Node { room: usize, node: usize },
}

/// How much a target glows in a frame: over the whole of it and at its rim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub target: GlowTarget,
    pub fill: f32,
    pub rim: f32,
}

#[derive(Clone, Debug)]
struct Lit {
    target: GlowTarget,
    fill: Ramp,
    rim: Ramp,
    /// Seconds of one pulse; 0 for none.
    pulse: f32,
    /// Steps since it was first lit.
    age: u32,
}

pub(crate) struct Glows {
    lit: Vec<Lit>,
    capacity: usize,
    depth: f32,
}

impl Glows {
    pub fn new(capacity: u32, depth: f32) -> Glows {
        Glows {
            lit: Vec::with_capacity(capacity as usize),
            capacity: capacity as usize,
            depth,
        }
    }

    pub fn set(
        &mut self,
        target: GlowTarget,
        (fill, rim): (f32, f32),
        pulse: f32,
        fade: f32,
    ) -> Result<(), FxReject> {
        let k = match self.lit.iter().position(|g| g.target == target) {
            Some(k) => k,
            None if fill == 0.0 && rim == 0.0 => return Ok(()),
            None if self.lit.len() < self.capacity => {
                self.lit.push(Lit {
                    target,
                    fill: Ramp::default(),
                    rim: Ramp::default(),
                    pulse,
                    age: 0,
                });
                self.lit.len() - 1
            }
            None => return Err(FxReject::Full),
        };
        let g = &mut self.lit[k];
        g.fill.to(fill, fade);
        g.rim.to(rim, fade);
        g.pulse = pulse;
        Ok(())
    }

    /// Eases every glow a step; one faded out, or on an entity gone, ends.
    pub fn step(&mut self, world: &World) {
        for g in &mut self.lit {
            g.fill.step();
            g.rim.step();
            g.age += 1;
        }
        self.lit.retain(|g| {
            let gone = matches!(g.target, GlowTarget::Entity(e) if !world.is_alive(e));
            !gone && !(g.fill.out() && g.rim.out())
        });
    }

    /// Every target glowing `alpha` of the way from the last step to this one.
    pub fn at(&self, alpha: f32) -> impl Iterator<Item = Glow> + '_ {
        self.lit.iter().map(move |g| {
            let pulse = if g.pulse > 0.0 {
                let seconds = (g.age as f64 - 1.0 + alpha as f64).max(0.0) * STEP_SECONDS as f64;
                let t = (seconds / g.pulse as f64).fract() as f32;
                1.0 - self.depth * 0.5 * (1.0 - libm::cosf(core::f32::consts::TAU * t))
            } else {
                1.0
            };
            Glow {
                target: g.target,
                fill: g.fill.at(alpha) * pulse,
                rim: g.rim.at(alpha) * pulse,
            }
        })
    }

    pub fn len(&self) -> usize {
        self.lit.len()
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        for g in self.at(1.0) {
            match g.target {
                GlowTarget::Entity(e) => {
                    h.u32(e.index);
                    h.u32(e.generation);
                }
                GlowTarget::Node { room, node } => {
                    h.u32(room as u32);
                    h.u32(node as u32);
                }
            }
            h.f32(g.fill);
            h.f32(g.rim);
        }
    }
}
