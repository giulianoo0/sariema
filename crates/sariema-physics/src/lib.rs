// SPDX-License-Identifier: Apache-2.0

//! Movement and collision for Sariema: a deterministic kinematic character
//! controller over 3D collision queries, triggers, platforms and path search.
//!
//! Build a [`World`] from colliders, add triggers and actors, then each fixed
//! step [`World::apply`] the intents and [`World::step`], reading the events.
//! The same intents in the same order give bit-identical states on every
//! platform: the simulation uses scalar `f32` arithmetic and `libm` only.
//!
//! Conventions: right-handed, Y up, north is −Z, metres, yaw about +Y with
//! yaw 0 facing +Z.

mod actor;
mod bvh;
mod controller;
mod geom;
mod hull;
mod intent;
mod math;
mod narrow;
mod nav;
mod pack;
mod query;
mod scene;
mod settings;
mod shape;
mod trigger;
mod world;

pub use actor::{ActorCollider, ActorDesc, Head, LocalBounds};
pub use hull::{Hull, convex_hull};
pub use intent::{ActorId, BlockReason, ColliderId, Event, EventKind, Intent, TriggerId};
pub use math::{Bounds, forward, wrap_angle, yaw_matrix, yaw_of};
pub use pack::{OwnedShape, actor_collider, spot_actor, visual_bounds};
pub use query::{ClosestPoint, Occluder, RayHit, ShapeHit};
pub use scene::Pose;
pub use settings::{CollisionSettings, ControllerSettings, MoveToSettings, NavSettings, Settings};
pub use shape::{Shape, ShapeError};
pub use world::{ActorState, BuildError, ColliderDesc, World};
