// SPDX-License-Identifier: Apache-2.0

//! Building a [`World`] from a room's collision pack.

use std::borrow::Borrow;

use glam::{Mat3, Vec3};
use sariema_format::collision::{self as pack, CollisionPack};

use crate::actor::{ActorCollider, ActorDesc, Head, LocalBounds};
use crate::math::Bounds;
use crate::scene::{NewCollider, SceneBuild};
use crate::settings::Settings;
use crate::shape::Shape;
use crate::world::{BuildError, World, tolerance};

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
        let mut build = World::build_from_pack(settings, pack);
        loop {
            if let Some(world) = build.advance(usize::MAX)? {
                return Ok(world);
            }
        }
    }

    /// Starts building the world of `pack` a share at a time
    /// ([`WorldBuild::advance`]): the world [`World::from_pack`] gives.
    pub fn build_from_pack<P: Borrow<CollisionPack>>(settings: Settings, pack: P) -> WorldBuild<P> {
        let colliders = pack.borrow().colliders.len();
        WorldBuild {
            scene: SceneBuild::new(tolerance(&settings), colliders),
            settings,
            pack,
            next: 0,
        }
    }
}

/// A collision pack's world being built, a share at a time.
#[derive(Debug)]
pub struct WorldBuild<P> {
    settings: Settings,
    pack: P,
    scene: SceneBuild,
    /// The next collider to add.
    next: usize,
}

impl<P: Borrow<CollisionPack>> WorldBuild<P> {
    /// Builds on until about `units` of work are spent: a collider costs its
    /// triangles or hull points (a primitive one), the tree over the
    /// colliders' parts an item folded, sorted or written. The world once it
    /// is built, after which the build is spent.
    pub fn advance(&mut self, mut units: usize) -> Result<Option<World>, BuildError> {
        let pack = self.pack.borrow();
        while let Some(c) = pack.colliders.get(self.next) {
            if units == 0 {
                return Ok(None);
            }
            let shape = OwnedShape::of(&c.shape);
            let collider = NewCollider {
                name: &c.name,
                shape: shape.shape(),
                enabled: c.enabled,
                platform: None,
            };
            self.scene
                .add(&collider)
                .map_err(|error| BuildError::Collider {
                    name: c.name.clone(),
                    error,
                })?;
            units = units.saturating_sub(match &c.shape {
                pack::Shape::Mesh { triangles, .. } => triangles.len(),
                pack::Shape::Hull { points } => points.len(),
                _ => 1,
            });
            self.next += 1;
        }
        let Some(scene) = self.scene.advance(units) else {
            return Ok(None);
        };
        let mut world = World::with_scene(self.settings.clone(), scene);
        world.set_nav_bounds(pack.nav_bounds.as_ref().map(|b| Bounds {
            min: Vec3::from(b.min),
            max: Vec3::from(b.max),
        }));
        for t in &pack.triggers {
            world.add_trigger(&t.name, OwnedShape::of(&t.shape).shape())?;
        }
        Ok(Some(world))
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
