// SPDX-License-Identifier: Apache-2.0

//! Particles: emitters the game starts, stops and bursts, each a fixed pool
//! of particles simulated on the step with the emitter's own seeded
//! generator, so the same commands give the same particles on every host.
//! A host draws [`Fx::sprites`] at the frame's `alpha`. Nothing here is
//! part of the state hash: particles never change what the game sees.

mod effect;

use glam::{Quat, Vec3};
use sariema_core::STEP_SECONDS;
use sariema_format::script::{Entity, PointList};

pub use effect::{Effect, FxSettings, Shape};

use crate::hash::StateHasher;
use crate::rng::Pcg32;
use crate::tween::steps_of;
use crate::world::World;

/// Why an fx command was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxReject {
    /// No effect of that name in `fx.effects`.
    Unknown,
    /// Every one of `fx.emitters` is in use.
    Full,
    /// More points than `fx.path_points`.
    Points,
}

impl FxReject {
    pub const ALL: [FxReject; 3] = [FxReject::Unknown, FxReject::Full, FxReject::Points];

    pub const fn code(self) -> &'static str {
        match self {
            FxReject::Unknown => "E_FX_UNKNOWN",
            FxReject::Full => "E_FX_FULL",
            FxReject::Points => "E_FX_POINTS",
        }
    }
}

/// A particle as a host draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sprite {
    pub position: Vec3,
    /// Metres across.
    pub size: f32,
    /// Linear RGB times the effect's intensity, and opacity.
    pub color: [f32; 4],
    /// Radians, about the axis it faces along.
    pub rotation: f32,
    pub additive: f32,
    pub shape: Shape,
    /// Lies on the ground facing up.
    pub flat: bool,
}

#[derive(Clone, Copy, Debug)]
struct Particle {
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    age: f32,
    life: f32,
    size: f32,
    rotation: f32,
    spin: f32,
    /// Unit horizontal direction of the sway, and its phase.
    drift: [f32; 2],
    phase: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Use {
    Free,
    Running(u32),
    /// Stopped or a burst: its particles live out, no more are born.
    Draining,
}

#[derive(Clone, Debug)]
struct Slot {
    using: Use,
    effect: usize,
    entity: Entity,
    offset: Vec3,
    path: usize,
    rng: Pcg32,
    owed: f32,
    /// Steps to the next puff; `u32::MAX` for none.
    puff_in: u32,
    /// Particles to be born at the next step.
    pending: u32,
    /// Steps left of a fade, and its length.
    fade: Option<(u32, u32)>,
    particles: Vec<Particle>,
}

struct Compiled {
    effect: Effect,
    id: u32,
    pool: usize,
    damping: f32,
    every: u32,
    /// The cosine of `spread`, and the turn from +Y to `direction`.
    cone: f32,
    aim: Quat,
}

pub struct Fx {
    effects: Vec<Compiled>,
    slots: Vec<Slot>,
    /// `path_points` points per slot.
    points: Vec<Vec3>,
    path_points: usize,
}

impl Default for Fx {
    fn default() -> Fx {
        Fx::new(&FxSettings::default())
    }
}

impl Fx {
    /// Every slot and its pool allocated now, so no step allocates.
    pub fn new(settings: &FxSettings) -> Fx {
        let effects: Vec<Compiled> = (settings.effects.iter())
            .map(|e| {
                let direction = Vec3::from(e.direction).try_normalize().unwrap_or(Vec3::Y);
                Compiled {
                    id: sariema_format::script::name_id(&e.name),
                    pool: e.pool.unwrap_or(settings.pool) as usize,
                    damping: libm::expf(-e.drag * STEP_SECONDS),
                    every: if e.every > 0.0 {
                        steps_of(e.every).max(1)
                    } else {
                        u32::MAX
                    },
                    cone: libm::cosf(e.spread.to_radians()),
                    aim: Quat::from_rotation_arc(Vec3::Y, direction),
                    effect: e.clone(),
                }
            })
            .collect();
        let most = effects.iter().map(|c| c.pool).max().unwrap_or(0);
        let path_points = settings.path_points as usize;
        let slot = Slot {
            using: Use::Free,
            effect: 0,
            entity: Entity::NONE,
            offset: Vec3::ZERO,
            path: 0,
            rng: Pcg32::new(0, 0),
            owed: 0.0,
            puff_in: u32::MAX,
            pending: 0,
            fade: None,
            particles: Vec::new(),
        };
        let slots = (0..settings.emitters)
            .map(|_| Slot {
                particles: Vec::with_capacity(most),
                ..slot.clone()
            })
            .collect();
        Fx {
            effects,
            slots,
            points: vec![Vec3::ZERO; path_points * settings.emitters as usize],
            path_points,
        }
    }

    fn effect(&self, id: u32) -> Result<usize, FxReject> {
        self.effects
            .iter()
            .position(|c| c.id == id)
            .ok_or(FxReject::Unknown)
    }

    fn take_slot(&mut self, running: Option<u32>) -> Result<usize, FxReject> {
        let same =
            running.and_then(|id| self.slots.iter().position(|s| s.using == Use::Running(id)));
        let k = same
            .or_else(|| self.slots.iter().position(|s| s.using == Use::Free))
            .ok_or(FxReject::Full)?;
        let slot = &mut self.slots[k];
        slot.particles.clear();
        slot.fade = None;
        slot.owed = 0.0;
        slot.pending = 0;
        slot.puff_in = u32::MAX;
        Ok(k)
    }

    /// Starts emitter `id` of effect `effect`, replacing a running one of
    /// that id; `offset` and `points` are from `entity`'s placement, or in
    /// the world when it is NONE.
    pub fn start(
        &mut self,
        id: u32,
        effect: u32,
        (entity, offset, points): (Entity, Vec3, PointList<'_>),
        seed: u32,
    ) -> Result<(), FxReject> {
        let e = self.effect(effect)?;
        if points.len() > self.path_points {
            return Err(FxReject::Points);
        }
        let k = self.take_slot(Some(id))?;
        let start = k * self.path_points;
        for (to, p) in self.points[start..].iter_mut().zip(points.iter()) {
            *to = Vec3::from(p);
        }
        let c = &self.effects[e];
        let slot = &mut self.slots[k];
        slot.using = Use::Running(id);
        slot.effect = e;
        slot.entity = entity;
        slot.offset = offset;
        slot.path = points.len();
        slot.rng = Pcg32::new(u64::from(seed), u64::from(id));
        if c.effect.puff > 0 {
            slot.pending = c.effect.puff;
            slot.puff_in = c.every;
        }
        Ok(())
    }

    /// Stops emitter `id`; its particles fade out over `fade` seconds.
    pub fn stop(&mut self, id: u32, fade: f32) {
        let Some(slot) = self.slots.iter_mut().find(|s| s.using == Use::Running(id)) else {
            return;
        };
        slot.using = Use::Draining;
        slot.pending = 0;
        let steps = steps_of(fade);
        if steps == 0 {
            slot.particles.clear();
            slot.using = Use::Free;
        } else {
            slot.fade = Some((steps, steps));
        }
    }

    /// `count` particles of `effect` at once, at most its pool.
    pub fn burst(
        &mut self,
        effect: u32,
        (entity, offset): (Entity, Vec3),
        count: u32,
        seed: u32,
    ) -> Result<(), FxReject> {
        let e = self.effect(effect)?;
        let k = self.take_slot(None)?;
        let slot = &mut self.slots[k];
        slot.using = Use::Draining;
        slot.effect = e;
        slot.entity = entity;
        slot.offset = offset;
        slot.path = 0;
        slot.rng = Pcg32::new(u64::from(seed), u64::from(effect));
        slot.pending = count;
        Ok(())
    }

    /// One step: particles age, move and die, then the emitters' new ones
    /// are born where their entity stands now.
    pub fn step(&mut self, world: &World) {
        let dt = STEP_SECONDS;
        for k in 0..self.slots.len() {
            if self.slots[k].using == Use::Free {
                continue;
            }
            let c = &self.effects[self.slots[k].effect];
            let slot = &mut self.slots[k];
            let gravity = Vec3::from(c.effect.gravity) * dt;
            let mut i = 0;
            while i < slot.particles.len() {
                let p = &mut slot.particles[i];
                p.previous = p.position;
                p.age += dt;
                if p.age >= p.life {
                    slot.particles.swap_remove(i);
                    continue;
                }
                p.velocity = (p.velocity + gravity) * c.damping;
                let sway = c.effect.drift
                    * libm::sinf(core::f32::consts::TAU * c.effect.drift_hz * p.age + p.phase);
                let sideways = Vec3::new(p.drift[0], 0.0, p.drift[1]) * sway;
                p.position += (p.velocity + sideways) * dt;
                p.rotation += p.spin * dt;
                i += 1;
            }
            if let Some((left, _)) = &mut slot.fade {
                *left -= 1;
                if *left == 0 {
                    slot.particles.clear();
                }
            }
            if let Use::Running(_) = slot.using {
                slot.owed += c.effect.rate * dt;
                let born = slot.owed as u32;
                slot.owed -= born as f32;
                slot.pending += born;
                if slot.puff_in != u32::MAX {
                    slot.puff_in -= 1;
                    if slot.puff_in == 0 {
                        slot.pending += c.effect.puff;
                        slot.puff_in = c.every;
                    }
                }
            }
            let born = std::mem::take(&mut slot.pending);
            let anchor = anchor(world, slot.entity);
            if born > 0
                && let Some(anchor) = anchor
            {
                let path = &self.points[k * self.path_points..][..slot.path];
                for _ in 0..born.min((c.pool - slot.particles.len().min(c.pool)) as u32) {
                    let p = spawn(c, &mut slot.rng, anchor, slot.offset, path);
                    slot.particles.push(p);
                }
            }
            let done = slot.particles.is_empty()
                && (slot.using == Use::Draining || (anchor.is_none() && !slot.entity.is_none()));
            if done {
                slot.using = Use::Free;
                slot.fade = None;
            }
        }
    }

    /// The live particles as drawn `alpha` of the way from the last step to
    /// this one, appended to `out`.
    pub fn sprites(&self, alpha: f32, out: &mut Vec<Sprite>) {
        let back = (1.0 - alpha) * STEP_SECONDS;
        for slot in self.slots.iter().filter(|s| s.using != Use::Free) {
            let c = &self.effects[slot.effect];
            let e = &c.effect;
            let fade = match slot.fade {
                Some((left, total)) => ((left as f32 + 1.0 - alpha) / total as f32).min(1.0),
                None => 1.0,
            };
            for p in &slot.particles {
                let t = ((p.age - back).max(0.0) / p.life).min(1.0);
                let [_, r, g, b, a] = effect::sample(&e.color, t, [0.0, 1.0, 1.0, 1.0, 1.0]);
                let [_, grow] = effect::sample(&e.grow, t, [0.0, 1.0]);
                out.push(Sprite {
                    position: p.previous.lerp(p.position, alpha),
                    size: p.size * grow,
                    color: [r * e.intensity, g * e.intensity, b * e.intensity, a * fade],
                    rotation: p.rotation - p.spin * back,
                    additive: e.additive,
                    shape: e.sprite,
                    flat: e.flat,
                });
            }
        }
    }

    /// Particles alive.
    pub fn live(&self) -> usize {
        self.slots.iter().map(|s| s.particles.len()).sum()
    }

    /// Emitters in use, bursts and fading ones included.
    pub fn emitters(&self) -> usize {
        self.slots.iter().filter(|s| s.using != Use::Free).count()
    }

    /// Every live particle's place, age and size: the same on every host
    /// for the same commands.
    pub fn hash(&self) -> u64 {
        let mut h = StateHasher::default();
        for (k, slot) in self.slots.iter().enumerate() {
            if slot.particles.is_empty() {
                continue;
            }
            h.u32(k as u32);
            for p in &slot.particles {
                h.vec3(p.position);
                h.f32(p.age);
                h.f32(p.size);
            }
        }
        h.finish()
    }
}

/// Where an emitter's entity stands: its place, turn and scale; `None` once
/// the entity is gone.
fn anchor(world: &World, entity: Entity) -> Option<(Vec3, Quat, f32)> {
    if entity.is_none() {
        return Some((Vec3::ZERO, Quat::IDENTITY, 1.0));
    }
    let pose = world.pose(entity)?;
    Some((pose.position, pose.rotation, pose.scale))
}

fn spawn(
    c: &Compiled,
    rng: &mut Pcg32,
    (at, turn, scale): (Vec3, Quat, f32),
    offset: Vec3,
    path: &[Vec3],
) -> Particle {
    let e = &c.effect;
    let local = offset + along(path, rng);
    let tau = core::f32::consts::TAU;
    let unit = |rng: &mut Pcg32| {
        let z = rng.range(-1.0, 1.0);
        let a = rng.range(0.0, tau);
        let r = libm::sqrtf((1.0 - z * z).max(0.0));
        Vec3::new(r * libm::cosf(a), z, r * libm::sinf(a))
    };
    let within = unit(rng) * e.radius * libm::cbrtf(rng.next_f32());
    let position = at + turn * ((local + within) * scale);
    let cos = rng.range(c.cone, 1.0);
    let around = rng.range(0.0, tau);
    let sin = libm::sqrtf((1.0 - cos * cos).max(0.0));
    let heading = c.aim * Vec3::new(sin * libm::cosf(around), cos, sin * libm::sinf(around));
    let sway = rng.range(0.0, tau);
    Particle {
        position,
        previous: position,
        velocity: heading * rng.range(e.speed[0], e.speed[1]),
        age: 0.0,
        life: rng.range(e.life[0], e.life[1]),
        size: rng.range(e.size[0], e.size[1]),
        rotation: rng.range(0.0, tau),
        spin: rng.range(e.spin[0], e.spin[1]).to_radians(),
        drift: [libm::cosf(sway), libm::sinf(sway)],
        phase: rng.range(0.0, tau),
    }
}

/// A point along `path`, even by length; the origin without one.
fn along(path: &[Vec3], rng: &mut Pcg32) -> Vec3 {
    match path {
        [] => Vec3::ZERO,
        [only] => *only,
        _ => {
            let total: f32 = path.windows(2).map(|w| w[0].distance(w[1])).sum();
            let mut left = rng.range(0.0, total);
            for w in path.windows(2) {
                let d = w[0].distance(w[1]);
                if left <= d && d > 0.0 {
                    return w[0].lerp(w[1], left / d);
                }
                left -= d;
            }
            path[path.len() - 1]
        }
    }
}
