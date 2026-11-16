// SPDX-License-Identifier: Apache-2.0

//! Building a [`World`] from a room's collision pack.

use glam::{Mat3, Vec3};
use sariema_format::collision::{self as pack, CollisionPack};

use crate::actor::{ActorCollider, ActorDesc, Head, LocalBounds};
use crate::math::Bounds;
use crate::settings::Settings;
use crate::shape::Shape;
use crate::world::{BuildError, ColliderDesc, World};

/// A pack shape in glam types, for a [`Shape`] to borrow.
#[derive(Clone, Debug, PartialEq)]
pub enum OwnedShape {
    Box {
        center: Vec3,
        half_extents: Vec3,
        rotation: Mat3,
    },
    Sphere {
        center: Vec3,
        radius: f32,
    },
    Capsule {
        a: Vec3,
        b: Vec3,
        radius: f32,
    },
    Hull(Vec<Vec3>),
    Mesh(Vec<Vec3>, Vec<[u32; 3]>),
}

impl OwnedShape {
    pub fn of(shape: &pack::Shape) -> OwnedShape {
        let v = Vec3::from_array;
        match shape {
            pack::Shape::Box {
                center,
                half_extents,
                rotation,
            } => OwnedShape::Box {
                center: v(*center),
                half_extents: v(*half_extents),
                rotation: Mat3::from_cols_array(rotation),
            },
            pack::Shape::Sphere { center, radius } => OwnedShape::Sphere {
                center: v(*center),
                radius: *radius,
            },
            pack::Shape::Capsule { a, b, radius } => OwnedShape::Capsule {
                a: v(*a),
                b: v(*b),
                radius: *radius,
            },
            pack::Shape::Hull { points } => {
                OwnedShape::Hull(points.iter().map(|p| v(*p)).collect())
            }
            pack::Shape::Mesh {
                vertices,
                triangles,
            } => OwnedShape::Mesh(vertices.iter().map(|p| v(*p)).collect(), triangles.clone()),
        }
    }

    pub fn shape(&self) -> Shape<'_> {
        match self {
            OwnedShape::Box {
                center,
                half_extents,
                rotation,
            } => Shape::Box {
                center: *center,
                half_extents: *half_extents,
                rotation: *rotation,
            },
            OwnedShape::Sphere { center, radius } => Shape::Sphere {
                center: *center,
                radius: *radius,
            },
            OwnedShape::Capsule { a, b, radius } => Shape::Capsule {
                a: *a,
                b: *b,
                radius: *radius,
            },
            OwnedShape::Hull(points) => Shape::Hull { points },
            OwnedShape::Mesh(vertices, triangles) => Shape::Mesh {
                vertices,
                triangles,
            },
        }
    }
}

impl World {
    /// The pack's colliders and triggers: collider `i` and trigger `i` get id
    /// `i`. Actors are the caller's to add, for instance with [`spot_actor`].
    pub fn from_pack(settings: Settings, pack: &CollisionPack) -> Result<World, BuildError> {
        let shapes: Vec<OwnedShape> = pack
            .colliders
            .iter()
            .map(|c| OwnedShape::of(&c.shape))
            .collect();
        let descs: Vec<ColliderDesc> = pack
            .colliders
            .iter()
            .zip(&shapes)
            .map(|(c, s)| ColliderDesc {
                name: &c.name,
                shape: s.shape(),
                enabled: c.enabled,
                platform: None,
            })
            .collect();
        let mut world = World::new(settings, &descs)?;
        world.set_nav_bounds(pack.nav_bounds.as_ref().map(|b| Bounds {
            min: Vec3::from(b.min),
            max: Vec3::from(b.max),
        }));
        for t in &pack.triggers {
            world.add_trigger(&t.name, OwnedShape::of(&t.shape).shape())?;
        }
        Ok(world)
    }
}

pub fn actor_collider(body: &pack::ActorBody) -> ActorCollider {
    ActorCollider {
        radius: body.radius,
        height: body.height,
        head: body.head.as_ref().map(|h| Head {
            radius: h.radius,
            height: h.height,
            forward: h.forward,
        }),
    }
}

pub fn visual_bounds(body: &pack::ActorBody) -> Option<LocalBounds> {
    body.visual_bounds.as_ref().map(|b| LocalBounds {
        min: Vec3::from_array(b.min),
        max: Vec3::from_array(b.max),
    })
}

/// The actor a spot names, standing on it.
pub fn spot_actor(pack: &CollisionPack, spot: &pack::Spot) -> ActorDesc {
    ActorDesc {
        position: Vec3::from_array(spot.position),
        yaw: spot.yaw,
        collider: actor_collider(&pack.actors[spot.actor as usize]),
        ..ActorDesc::default()
    }
}
