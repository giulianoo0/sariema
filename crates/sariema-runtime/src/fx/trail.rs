// SPDX-License-Identifier: Apache-2.0

//! Trails: soft dots lying along a route to a goal, measured back from the
//! goal so they stay put while the route's start follows someone walking,
//! fading near both ends, with a wave of light running towards the goal.

use glam::Vec3;
use sariema_format::script::{Entity, PointList};

use super::effect::TrailStyle;
use super::ramp::Ramp;
use super::{FxReject, Sprite};
use crate::hash::StateHasher;
use crate::world::World;

/// What a `trail` command asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrailLook {
    pub spacing: f32,
    pub size: f32,
    pub color: Vec3,
    pub intensity: f32,
    pub wave_speed: f32,
    pub fade_from: f32,
    pub fade_to: f32,
}

#[derive(Clone, Debug)]
struct Trail {
    id: u32,
    from: Entity,
    /// Where `from` stood at the last two steps, while it is alive.
    start: Option<[Vec3; 2]>,
    points: usize,
    look: TrailLook,
    shown: Ramp,
}

pub(crate) struct Trails {
    trails: Vec<Trail>,
    capacity: usize,
    per_trail: usize,
    /// `per_trail` points for each trail, in the order of `trails`.
    points: Vec<Vec3>,
    dots: usize,
    style: TrailStyle,
}

impl Trails {
    pub fn new(trails: u32, points: u32, dots: u32, style: TrailStyle) -> Trails {
        let (capacity, per_trail) = (trails as usize, points as usize);
        Trails {
            trails: Vec::with_capacity(capacity),
            capacity,
            per_trail,
            points: vec![Vec3::ZERO; capacity * per_trail],
            dots: dots as usize,
            style,
        }
    }

    pub fn show(
        &mut self,
        world: &World,
        (id, from, points): (u32, Entity, PointList<'_>),
        look: TrailLook,
        fade: f32,
    ) -> Result<(), FxReject> {
        if points.len() > self.per_trail {
            return Err(FxReject::Points);
        }
        let k = match self.trails.iter().position(|t| t.id == id) {
            Some(k) => k,
            None if self.trails.len() < self.capacity => {
                self.trails.push(Trail {
                    id,
                    from,
                    start: None,
                    points: 0,
                    look,
                    shown: Ramp::default(),
                });
                self.trails.len() - 1
            }
            None => return Err(FxReject::Full),
        };
        let t = &mut self.trails[k];
        t.from = from;
        t.start = from_place(world, from).map(|at| [at; 2]);
        t.points = points.len();
        t.look = look;
        t.shown.to(1.0, fade);
        let slab = &mut self.points[k * self.per_trail..][..points.len()];
        for (to, p) in slab.iter_mut().zip(points.iter()) {
            *to = Vec3::from(p);
        }
        Ok(())
    }

    pub fn hide(&mut self, id: u32, fade: f32) {
        if let Some(t) = self.trails.iter_mut().find(|t| t.id == id) {
            t.shown.to(0.0, fade);
        }
    }

    pub fn step(&mut self, world: &World) {
        let mut k = 0;
        while k < self.trails.len() {
            let t = &mut self.trails[k];
            t.shown.step();
            t.start = from_place(world, t.from).map(|at| [t.start.map_or(at, |s| s[1]), at]);
            if t.shown.out() {
                let last = self.trails.len() - 1;
                if k != last {
                    let (a, b) = (k * self.per_trail, last * self.per_trail);
                    self.points.copy_within(b..b + self.per_trail, a);
                }
                self.trails.swap_remove(k);
                continue;
            }
            k += 1;
        }
    }

    /// The dots of every trail `alpha` of the way from the last step, at
    /// `time` seconds of world time.
    pub fn sprites(&self, alpha: f32, time: f64, out: &mut Vec<Sprite>) {
        let every = self.style.wave_every.max(1e-3);
        for (k, t) in self.trails.iter().enumerate() {
            let shown = t.shown.at(alpha);
            let points = &self.points[k * self.per_trail..][..t.points];
            let start = t.start.map(|[a, b]| a.lerp(b, alpha));
            let wave = (t.look.wave_speed as f64 * time).rem_euclid(every as f64) as f32;
            self.dots(t, (shown, wave), Route { start, points }, out);
        }
    }

    fn dots(
        &self,
        t: &Trail,
        (shown, wave_at): (f32, f32),
        route: Route<'_>,
        out: &mut Vec<Sprite>,
    ) {
        let look = &t.look;
        let style = &self.style;
        let total = route.length();
        if total <= 0.0 || look.spacing <= 0.0 || shown <= 0.0 {
            return;
        }
        let every = style.wave_every.max(1e-3);
        let (mut walked, mut next, mut placed) = (0.0, 0.0, 0);
        let mut b = route.last();
        for a in route.backwards() {
            let length = a.distance(b);
            while next <= walked + length && placed < self.dots {
                let at = if length > 0.0 {
                    b.lerp(a, (next - walked) / length)
                } else {
                    b
                };
                let (to_goal, from_start) = (next, total - next);
                let ends = rise(from_start, look.fade_from) * rise(to_goal, look.fade_to);
                let d = (to_goal + wave_at).rem_euclid(every);
                let crest = d.min(every - d) / style.wave_length.max(1e-3);
                let wave = libm::expf(-2.0 * crest * crest);
                let glow = look.intensity * (1.0 + style.wave_gain * wave);
                out.push(Sprite {
                    position: at + Vec3::Y * style.lift,
                    size: look.size * (1.0 + style.wave_grow * wave),
                    color: [
                        look.color.x * glow,
                        look.color.y * glow,
                        look.color.z * glow,
                        ends * shown * (1.0 - style.wave_alpha * (1.0 - wave)),
                    ],
                    rotation: 0.0,
                    additive: style.additive,
                    shape: style.sprite,
                    flat: true,
                });
                next += look.spacing;
                placed += 1;
            }
            walked += length;
            b = a;
        }
    }

    pub fn len(&self) -> usize {
        self.trails.len()
    }

    pub fn hash_into(&self, h: &mut StateHasher) {
        for (k, t) in self.trails.iter().enumerate() {
            h.u32(t.id);
            h.f32(t.shown.at(1.0));
            if let Some([_, at]) = t.start {
                h.vec3(at);
            }
            for p in &self.points[k * self.per_trail..][..t.points] {
                h.vec3(*p);
            }
        }
    }
}

/// 0 at the end, rising smoothly to 1 at `within` metres from it.
fn rise(distance: f32, within: f32) -> f32 {
    if within <= 0.0 {
        return 1.0;
    }
    let t = (distance / within).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn from_place(world: &World, from: Entity) -> Option<Vec3> {
    if from.is_none() {
        return None;
    }
    world.pose(from).map(|p| p.position)
}

/// A start, if the trail follows someone, then its points.
#[derive(Clone, Copy)]
struct Route<'a> {
    start: Option<Vec3>,
    points: &'a [Vec3],
}

impl Route<'_> {
    fn at(&self, i: usize) -> Vec3 {
        match self.start {
            Some(s) if i == 0 => s,
            Some(_) => self.points[i - 1],
            None => self.points[i],
        }
    }

    fn count(&self) -> usize {
        self.points.len() + self.start.is_some() as usize
    }

    fn last(&self) -> Vec3 {
        self.at(self.count() - 1)
    }

    /// Every point but the last, last first.
    fn backwards(self) -> impl Iterator<Item = Vec3> {
        (0..self.count().saturating_sub(1))
            .rev()
            .map(move |i| self.at(i))
    }

    fn length(&self) -> f32 {
        (1..self.count())
            .map(|i| self.at(i - 1).distance(self.at(i)))
            .sum()
    }
}
