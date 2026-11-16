// SPDX-License-Identifier: Apache-2.0

//! Geometric queries for gameplay and camera code. They see the enabled
//! colliders, static and platforms, unless a method says otherwise.

use glam::Vec3;

use crate::actor::{ActorCollider, Probe};
use crate::intent::ColliderId;
use crate::narrow::Round;
use crate::scene::Filter;
use crate::world::World;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub distance: f32,
    pub point: Vec3,
    /// Faces the ray's origin.
    pub normal: Vec3,
    pub collider: ColliderId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeHit {
    /// Fraction of the motion travelled before touching.
    pub fraction: f32,
    /// Contact point on the collider.
    pub point: Vec3,
    /// Points from the collider towards the moving shape.
    pub normal: Vec3,
    pub collider: ColliderId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClosestPoint {
    pub point: Vec3,
    /// Negative inside a solid collider.
    pub distance: f32,
    pub collider: ColliderId,
}

/// A collider between a camera and what it looks at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occluder {
    pub collider: ColliderId,
    /// From the target to where the camera sphere first touches it.
    pub distance: f32,
}

/// A capsule from `a` to `b` of `radius`; `a == b` is a sphere and radius 0 a segment.
fn capsule(a: Vec3, b: Vec3, radius: f32) -> Probe {
    Probe::single(Round { a, b, r: radius })
}

impl World {
    /// First surface along `dir` from `origin` within `max` metres.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<RayHit> {
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        self.scene()
            .raycast(origin, dir, max, Filter::ALL)
            .map(|h| RayHit {
                distance: h.distance,
                point: origin + dir * h.distance,
                normal: h.normal,
                collider: ColliderId(h.collider),
            })
    }

    /// First contact of the capsule `a`–`b` (radius `radius`) moved by `motion`.
    /// A capsule that starts touching something only stops if it moves into it.
    pub fn shapecast(&self, a: Vec3, b: Vec3, radius: f32, motion: Vec3) -> Option<ShapeHit> {
        self.scene()
            .sweep(&capsule(a, b, radius), motion, Filter::ALL)
            .map(|h| ShapeHit {
                fraction: h.t,
                point: h.point,
                normal: h.normal,
                collider: ColliderId(h.collider),
            })
    }

    /// Writes into `out`, sorted and without repeats, the colliders the capsule overlaps.
    pub fn overlap(&self, a: Vec3, b: Vec3, radius: f32, out: &mut Vec<ColliderId>) {
        out.clear();
        self.scene()
            .contacts(&capsule(a, b, radius), 0.0, Filter::ALL, |c, _| {
                out.push(ColliderId(c))
            });
        out.sort_unstable();
        out.dedup();
    }

    /// The nearest collider surface within `max` of `point`.
    pub fn closest_point(&self, point: Vec3, max: f32) -> Option<ClosestPoint> {
        let mut best: Option<ClosestPoint> = None;
        self.scene()
            .contacts(&capsule(point, point, 0.0), max, Filter::ALL, |c, hit| {
                if best.is_none_or(|b| hit.separation < b.distance) {
                    best = Some(ClosestPoint {
                        point: hit.point,
                        distance: hit.separation,
                        collider: ColliderId(c),
                    });
                }
            });
        best
    }

    /// The deepest overlap of an actor collider placed at `feet` facing `yaw`:
    /// the collider and how far it is inside (negative separation).
    pub fn actor_overlap(
        &self,
        collider: &ActorCollider,
        feet: Vec3,
        yaw: f32,
    ) -> Option<(ColliderId, f32)> {
        self.scene()
            .deepest(&collider.probe(feet, yaw, 0.0), Filter::ALL)
            .map(|(c, hit)| (ColliderId(c), hit.separation))
    }

    /// What lies between `target` and `camera` for a camera sphere of
    /// `radius`: writes the static colliders it passes through into `out`
    /// (nearest first) and returns how far from the target the camera can
    /// sit before touching anything.
    pub fn camera_clearance(
        &self,
        target: Vec3,
        camera: Vec3,
        radius: f32,
        out: &mut Vec<Occluder>,
    ) -> f32 {
        out.clear();
        let motion = camera - target;
        let len = motion.length();
        let probe = capsule(target, target, radius);
        self.scene().sweep_all(&probe, motion, Filter::ALL, |c, t| {
            let distance = t * len;
            match out.iter_mut().find(|o| o.collider.0 == c) {
                Some(o) => o.distance = o.distance.min(distance),
                None => out.push(Occluder {
                    collider: ColliderId(c),
                    distance,
                }),
            }
        });
        out.sort_unstable_by(|a, b| {
            a.distance
                .total_cmp(&b.distance)
                .then(a.collider.cmp(&b.collider))
        });
        let skin = self.settings().collision.skin;
        self.scene()
            .sweep(&probe, motion, Filter::ALL)
            .map_or(len, |h| (h.t * len - skin).max(0.0))
    }
}
