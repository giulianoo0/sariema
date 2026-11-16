// SPDX-License-Identifier: Apache-2.0

//! Things held at a socket of an actor: a bag in the hand, a lantern on the
//! belt. Each step the animation module says where the socket is in its
//! actor's frame, and the held entity is placed rigidly on it, in metres of
//! the world: it never grows with the actor's scale. Let go, it stays where
//! it was.

use glam::{EulerRot, Mat4, Quat, Vec3};

use crate::world::{Entity, World};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Held {
    entity: Entity,
    actor: Entity,
    socket: u32,
    /// Where it sits from the socket, in the socket's frame and metres of
    /// the world; `None` until the socket is first known, for a `keep`.
    on: Option<(Vec3, Quat)>,
    /// The socket in its actor's frame, as of this step.
    socket_pose: Option<(Vec3, Quat)>,
}

#[derive(Clone, Debug, Default)]
pub struct Holds {
    list: Vec<Held>,
}

impl Holds {
    /// `actor` holds `entity` at its socket or joint `socket`: where
    /// `entity` stands when `keep`, else at `offset` from it, turned with it.
    pub fn hold(&mut self, entity: Entity, actor: Entity, socket: u32, offset: Vec3, keep: bool) {
        self.release(entity);
        self.list.push(Held {
            entity,
            actor,
            socket,
            on: (!keep).then_some((offset, Quat::IDENTITY)),
            socket_pose: None,
        });
    }

    /// Whether `entity` was held.
    pub fn release(&mut self, entity: Entity) -> bool {
        let before = self.list.len();
        self.list.retain(|h| h.entity != entity);
        self.list.len() != before
    }

    /// Lets go of `e` and of what `e` held.
    pub fn forget(&mut self, e: Entity) {
        self.list.retain(|h| h.entity != e && h.actor != e);
    }

    /// Each held entity with its actor and the socket's name, for the
    /// animation module.
    pub fn sockets(&self) -> impl Iterator<Item = (Entity, Entity, u32)> + '_ {
        self.list.iter().map(|h| (h.entity, h.actor, h.socket))
    }

    /// Where the socket holding `entity` is this step, in its actor's frame.
    pub fn set_socket(&mut self, entity: Entity, socket: Mat4) {
        if let Some(h) = self.list.iter_mut().find(|h| h.entity == entity) {
            let (_, rotation, position) = socket.to_scale_rotation_translation();
            h.socket_pose = Some((position, rotation.normalize()));
        }
    }

    /// Places each held entity on its socket, where the animation module
    /// put the socket this step; one whose socket is not known stays.
    pub fn step(&mut self, world: &mut World) {
        for h in &mut self.list {
            let Some((at, turn)) = h.socket_pose.take() else {
                continue;
            };
            let (Some(actor), Some(me)) =
                (world.current_pose(h.actor), world.current_pose(h.entity))
            else {
                continue;
            };
            let socket = actor.position + actor.rotation * (at * actor.scale);
            let turned = (actor.rotation * turn).normalize();
            let (offset, rotation) = *h.on.get_or_insert_with(|| {
                let back = turned.inverse();
                (back * (me.position - socket), back * me.rotation)
            });
            let Ok(node) = world.node_mut(h.entity) else {
                continue;
            };
            let (yaw, pitch, roll) = (turned * rotation).to_euler(EulerRot::YXZ);
            node.local.position = socket + turned * offset;
            node.local.yaw = yaw.to_degrees();
            node.local.pitch = pitch.to_degrees();
            node.local.roll = roll.to_degrees();
        }
    }
}
