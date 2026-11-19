// SPDX-License-Identifier: Apache-2.0

//! A ray a game casts against the active room during its tick, and what it
//! hits, as the fixed little-endian records the `ray` import exchanges.
//! Shared verbatim with `sariema-sdk`.

use super::wire::Entity;

const F32: usize = core::mem::size_of::<f32>();
const U32: usize = core::mem::size_of::<u32>();
const ENTITY: usize = 2 * U32;
const VEC3: usize = 3 * F32;

/// Bit of [`Ray::write`]'s flags byte: the room's colliders are hit.
const COLLIDERS: u8 = 1;
/// Bit of the flags byte: actors' bodies are hit.
const ACTORS: u8 = 2;

/// What the `ray` import returns: the hit was written.
pub const RAY_HIT: i32 = 1;
/// Nothing within `max`.
pub const RAY_MISS: i32 = 0;
/// The ray is not one: a record of the wrong length, a direction of zero
/// length, a negative or non-finite number.
pub const RAY_INVALID: i32 = -1;
/// The step already cast `vm.rays_per_step` rays.
pub const RAY_OVER_BUDGET: i32 = -2;

/// A ray from `origin` along `dir` (any length but zero) up to `max` metres.
/// It sees the room as it stood when the step began, before the commands
/// of the tick casting it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: [f32; 3],
    pub dir: [f32; 3],
    pub max: f32,
    /// The room's colliders, static pieces and switched or hinged ones.
    pub colliders: bool,
    /// Actors' body capsules, of the actors in other actors' way.
    pub actors: bool,
    /// An actor the ray passes through, such as the one casting it;
    /// [`Entity::NONE`] for none.
    pub exclude: Entity,
}

/// The first thing a [`Ray`] hits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// Metres from the ray's origin.
    pub distance: f32,
    pub point: [f32; 3],
    /// The surface's normal, facing the ray's origin.
    pub normal: [f32; 3],
    /// The actor hit, or the entity that last switched the collider hit;
    /// [`Entity::NONE`] for a collider no entity switched.
    pub entity: Entity,
    /// The [`name_id`](super::wire::name_id) of the collider hit; 0 for an actor.
    pub collider: u32,
}

/// Fills a record field by field.
struct Out<'a> {
    bytes: &'a mut [u8],
    at: usize,
}

impl Out<'_> {
    fn put(&mut self, b: &[u8]) {
        self.bytes[self.at..self.at + b.len()].copy_from_slice(b);
        self.at += b.len();
    }

    fn f32(&mut self, v: f32) {
        self.put(&v.to_le_bytes());
    }

    fn vec3(&mut self, v: [f32; 3]) {
        v.iter().for_each(|&c| self.f32(c));
    }

    fn u32(&mut self, v: u32) {
        self.put(&v.to_le_bytes());
    }

    fn entity(&mut self, e: Entity) {
        self.u32(e.index);
        self.u32(e.generation);
    }
}

/// Reads a record field by field.
struct In<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl In<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0; N];
        out.copy_from_slice(&self.bytes[self.at..self.at + N]);
        self.at += N;
        out
    }

    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take())
    }

    fn vec3(&mut self) -> [f32; 3] {
        [self.f32(), self.f32(), self.f32()]
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }

    fn entity(&mut self) -> Entity {
        Entity {
            index: self.u32(),
            generation: self.u32(),
        }
    }
}

impl Ray {
    /// Bytes of the record: origin, direction, max, a flags byte, exclude.
    pub const BYTES: usize = VEC3 + VEC3 + F32 + 1 + ENTITY;

    pub fn write(&self, out: &mut [u8; Ray::BYTES]) {
        let mut o = Out { bytes: out, at: 0 };
        o.vec3(self.origin);
        o.vec3(self.dir);
        o.f32(self.max);
        let flags =
            if self.colliders { COLLIDERS } else { 0 } | if self.actors { ACTORS } else { 0 };
        o.put(&[flags]);
        o.entity(self.exclude);
    }

    /// The ray `bytes` hold, if they are one record long.
    pub fn read(bytes: &[u8]) -> Option<Ray> {
        if bytes.len() != Ray::BYTES {
            return None;
        }
        let mut r = In { bytes, at: 0 };
        let origin = r.vec3();
        let dir = r.vec3();
        let max = r.f32();
        let [flags] = r.take();
        Some(Ray {
            origin,
            dir,
            max,
            colliders: flags & COLLIDERS != 0,
            actors: flags & ACTORS != 0,
            exclude: r.entity(),
        })
    }
}

impl RayHit {
    /// Bytes of the record: distance, point, normal, entity, collider.
    pub const BYTES: usize = F32 + VEC3 + VEC3 + ENTITY + U32;

    pub fn write(&self, out: &mut [u8; RayHit::BYTES]) {
        let mut o = Out { bytes: out, at: 0 };
        o.f32(self.distance);
        o.vec3(self.point);
        o.vec3(self.normal);
        o.entity(self.entity);
        o.u32(self.collider);
    }

    /// The hit `bytes` hold, if they are one record long.
    pub fn read(bytes: &[u8]) -> Option<RayHit> {
        if bytes.len() != RayHit::BYTES {
            return None;
        }
        let mut r = In { bytes, at: 0 };
        Some(RayHit {
            distance: r.f32(),
            point: r.vec3(),
            normal: r.vec3(),
            entity: r.entity(),
            collider: r.u32(),
        })
    }
}
