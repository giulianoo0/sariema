// SPDX-License-Identifier: Apache-2.0

//! What callers ask of the world between steps, and what a step reports back.

use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorId(pub u32);

/// A collider, static or platform, by its position in the list the world was built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColliderId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TriggerId(pub u32);

/// Applied by [`crate::World::apply`]; one-shot intents take effect in the next step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intent {
    /// Stick input on the ground plane: `x` along +X, `y` along +Z, length up
    /// to 1. It stays until replaced; it is ignored during a scripted move or
    /// a knockback.
    Move {
        actor: ActorId,
        stick: Vec2,
    },
    /// Walk to `target` along a path, with collision, then turn to `face`
    /// (a yaw) when given. Ends with [`EventKind::Arrived`] or [`EventKind::Blocked`].
    MoveTo {
        actor: ActorId,
        target: Vec3,
        face: Option<f32>,
    },
    /// Drops the stick input and any scripted move.
    Stop {
        actor: ActorId,
    },
    /// Pushes the actor by `push` (horizontal) over `steps` steps, fastest
    /// first; stick input is ignored meanwhile.
    Knockback {
        actor: ActorId,
        push: Vec3,
        steps: u32,
    },
    /// Puts the actor at `position` facing `yaw`, then pushes it out of
    /// anything it overlaps; if that fails it stays where it was and
    /// [`EventKind::TeleportRejected`] is reported.
    Teleport {
        actor: ActorId,
        position: Vec3,
        yaw: f32,
    },
    /// Puts the actor exactly at `position` facing `yaw` with its collider
    /// off, for a traversal driven by a clip: no collision, gravity or
    /// pushing until the next `Move`, `MoveTo`, `Stop`, `Knockback` or
    /// `Teleport`. Triggers still see it. The last pose must leave it on free
    /// floor, which `sariema check` proves for declared traversals.
    Pose {
        actor: ActorId,
        position: Vec3,
        yaw: f32,
    },
    SetColliderEnabled {
        collider: ColliderId,
        enabled: bool,
    },
    /// Multiplies the actor's walk speed; 1 is normal.
    SetSpeedScale {
        actor: ActorId,
        scale: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub actor: ActorId,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventKind {
    /// The actor started pressing against a wall; `normal` points away from it.
    HitWall {
        collider: ColliderId,
        normal: Vec3,
    },
    /// The actor reached ground after falling at `speed` m/s.
    Landed {
        speed: f32,
    },
    TriggerEnter(TriggerId),
    TriggerExit(TriggerId),
    Arrived,
    Blocked(BlockReason),
    TeleportRejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    /// The walkable grid has no path to the target.
    NoPath,
    /// The actor stopped making progress along its path.
    Stuck,
}
