// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{
    ActorCollider, ActorDesc, ActorId, ColliderDesc, Event, Intent, Pose, Settings, Shape, World,
    yaw_matrix,
};

pub enum Owned {
    Box {
        center: Vec3,
        half: Vec3,
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

impl Owned {
    fn shape(&self) -> Shape<'_> {
        match self {
            Owned::Box {
                center,
                half,
                rotation,
            } => Shape::Box {
                center: *center,
                half_extents: *half,
                rotation: *rotation,
            },
            Owned::Sphere { center, radius } => Shape::Sphere {
                center: *center,
                radius: *radius,
            },
            Owned::Capsule { a, b, radius } => Shape::Capsule {
                a: *a,
                b: *b,
                radius: *radius,
            },
            Owned::Hull(points) => Shape::Hull { points },
            Owned::Mesh(vertices, triangles) => Shape::Mesh {
                vertices,
                triangles,
            },
        }
    }
}

#[derive(Default)]
pub struct Room {
    pub items: Vec<(String, Owned, Option<Pose>)>,
}

impl Room {
    pub fn add(&mut self, name: &str, shape: Owned) -> &mut Room {
        self.items.push((name.to_owned(), shape, None));
        self
    }

    pub fn platform(&mut self, name: &str, shape: Owned, pose: Pose) -> &mut Room {
        self.items.push((name.to_owned(), shape, Some(pose)));
        self
    }

    /// A slab whose top is at y = 0, `size` metres square, centred on the origin.
    pub fn floor(&mut self, size: f32) -> &mut Room {
        self.add(
            "floor",
            Owned::Box {
                center: Vec3::new(0.0, -0.5, 0.0),
                half: Vec3::new(size * 0.5, 0.5, size * 0.5),
                rotation: Mat3::IDENTITY,
            },
        )
    }

    /// A box standing on the floor, centred on `at` (x, z), turned by `yaw`;
    /// `size` is length (local x), height and thickness (local z).
    pub fn wall(&mut self, name: &str, at: Vec2, size: Vec3, yaw: f32) -> &mut Room {
        self.add(
            name,
            Owned::Box {
                center: Vec3::new(at.x, size.y * 0.5, at.y),
                half: size * 0.5,
                rotation: yaw_matrix(yaw),
            },
        )
    }

    pub fn world(&self, settings: Settings) -> World {
        let descs: Vec<ColliderDesc> = self
            .items
            .iter()
            .map(|(name, shape, platform)| ColliderDesc {
                name,
                shape: shape.shape(),
                enabled: true,
                platform: *platform,
            })
            .collect();
        World::new(settings, &descs).expect("a valid fixture")
    }
}

pub fn actor(world: &mut World, at: Vec3) -> ActorId {
    actor_with(world, at, ActorCollider::default(), 2.1)
}

pub fn actor_with(world: &mut World, at: Vec3, collider: ActorCollider, speed: f32) -> ActorId {
    world
        .add_actor(ActorDesc {
            position: at,
            collider,
            walk_speed: speed,
            ..ActorDesc::default()
        })
        .unwrap()
}

/// Holds the stick at `stick` for `steps` steps; returns the position after each.
pub fn hold(
    world: &mut World,
    id: ActorId,
    stick: Vec2,
    steps: usize,
    events: &mut Vec<Event>,
) -> Vec<Vec3> {
    world.apply(Intent::Move { actor: id, stick });
    (0..steps)
        .map(|_| {
            world.step(events);
            world.actor(id).position
        })
        .collect()
}

/// Steps until the actor settles on the ground.
pub fn settle(world: &mut World, id: ActorId) {
    let mut events = Vec::new();
    for _ in 0..30 {
        world.step(&mut events);
    }
    assert!(world.actor(id).grounded, "{:?}", world.actor(id));
}

/// A stick direction at `angle` radians from +X towards +Z, full deflection.
pub fn stick_at(angle: f32) -> Vec2 {
    Vec2::new(angle.cos(), angle.sin())
}

/// A tiny deterministic generator for fixtures (xorshift64*).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}
