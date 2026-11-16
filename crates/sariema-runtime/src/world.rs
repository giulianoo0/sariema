// SPDX-License-Identifier: Apache-2.0

//! Entities addressed by slot index and generation, with transform, kind,
//! parent, visibility and tags. The game allocates handles (the SDK keeps
//! the free list); the world checks every handle it is given.

use glam::{EulerRot, Quat, Vec3};
pub use sariema_format::script::{Entity, Kind};

/// Why the world refused an operation; `code` is what a `rejected` event carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// The handle's slot is free or has a newer generation.
    Stale,
    /// `spawn` into a live slot, or with a generation the slot already had.
    InUse,
    /// The slot index is at or past `max_entities`.
    Limit,
    /// `set_parent` would make an entity its own ancestor.
    Cycle,
    /// A visible actor would move or turn faster than it may be seen to,
    /// without a cut, a full fade or `force`.
    TeleportVisible,
    /// The command needs an actor and the entity is another kind.
    NotActor,
}

impl Reject {
    pub const ALL: [Reject; 6] = [
        Reject::Stale,
        Reject::InUse,
        Reject::Limit,
        Reject::Cycle,
        Reject::TeleportVisible,
        Reject::NotActor,
    ];

    pub const fn code(self) -> &'static str {
        match self {
            Reject::Stale => "E_ENTITY_STALE",
            Reject::InUse => "E_ENTITY_IN_USE",
            Reject::Limit => "E_ENTITY_LIMIT",
            Reject::Cycle => "E_PARENT_CYCLE",
            Reject::TeleportVisible => "E_TELEPORT_VISIBLE",
            Reject::NotActor => "E_NOT_ACTOR",
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Reject::Stale => "the entity handle is stale: its slot was freed or reused",
            Reject::InUse => "spawn into a live slot or with a generation the slot already had",
            Reject::Limit => "the slot index is at or past the max_entities setting",
            Reject::Cycle => "set_parent would make the entity its own ancestor",
            Reject::TeleportVisible => {
                "a visible actor may not move faster than motion.max_visible_speed nor turn faster than motion.turn_rate, except under a full fade, in the step of a camera cut, or by a forced teleport"
            }
            Reject::NotActor => "the command needs an actor",
        }
    }
}

/// Local placement, relative to the parent. Angles in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    /// About +Y; 0 faces −Z.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        position: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        scale: 1.0,
    };

    pub fn rotation(&self) -> Quat {
        Quat::from_euler(
            EulerRot::YXZ,
            self.yaw.to_radians(),
            self.pitch.to_radians(),
            self.roll.to_radians(),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: Kind,
    pub asset: u32,
    pub local: Transform,
    pub parent: Entity,
    pub visible: bool,
    /// Sorted, without repeats.
    pub tags: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    generation: u32,
    used: bool,
    alive: bool,
    /// Visible when the step began.
    shown: bool,
    /// Placed discontinuously this step, itself or through a parent.
    snapped: bool,
    children: u32,
}

pub struct World {
    max_entities: u32,
    slots: Vec<Slot>,
    nodes: Vec<Node>,
    poses: Vec<Pose>,
    previous: Vec<Pose>,
    resolve_stamp: Vec<u64>,
    stamp: u64,
    chain: Vec<u32>,
    alive: u32,
}

const EMPTY_NODE: Node = Node {
    kind: Kind::Empty,
    asset: 0,
    local: Transform::IDENTITY,
    parent: Entity::NONE,
    visible: true,
    tags: Vec::new(),
};

const IDENTITY_POSE: Pose = Pose {
    position: Vec3::ZERO,
    rotation: Quat::IDENTITY,
    scale: 1.0,
};

impl World {
    pub fn new(max_entities: u32) -> World {
        World {
            max_entities,
            slots: Vec::new(),
            nodes: Vec::new(),
            poses: Vec::new(),
            previous: Vec::new(),
            resolve_stamp: Vec::new(),
            stamp: 0,
            chain: Vec::new(),
            alive: 0,
        }
    }

    pub fn len(&self) -> u32 {
        self.alive
    }

    pub fn is_empty(&self) -> bool {
        self.alive == 0
    }

    pub fn is_alive(&self, e: Entity) -> bool {
        self.slots
            .get(e.index as usize)
            .is_some_and(|s| s.alive && s.generation == e.generation)
    }

    pub fn check(&self, e: Entity) -> Result<(), Reject> {
        if self.is_alive(e) {
            Ok(())
        } else {
            Err(Reject::Stale)
        }
    }

    pub fn spawn(&mut self, e: Entity, kind: Kind, asset: u32) -> Result<(), Reject> {
        if e.index >= self.max_entities {
            return Err(Reject::Limit);
        }
        let i = e.index as usize;
        if i >= self.slots.len() {
            self.slots.resize(i + 1, Slot::default());
            self.nodes.resize(i + 1, EMPTY_NODE);
            self.poses.resize(i + 1, IDENTITY_POSE);
            self.previous.resize(i + 1, IDENTITY_POSE);
            self.resolve_stamp.resize(i + 1, 0);
        }
        let slot = &mut self.slots[i];
        if slot.alive || (slot.used && e.generation <= slot.generation) {
            return Err(Reject::InUse);
        }
        *slot = Slot {
            generation: e.generation,
            used: true,
            alive: true,
            snapped: true,
            ..Slot::default()
        };
        let mut node = std::mem::replace(&mut self.nodes[i], EMPTY_NODE);
        node.tags.clear();
        self.nodes[i] = Node {
            kind,
            asset,
            tags: node.tags,
            ..EMPTY_NODE
        };
        self.alive += 1;
        Ok(())
    }

    /// Frees the slot. Children keep their world placement and lose the parent.
    pub fn despawn(&mut self, e: Entity) -> Result<(), Reject> {
        self.check(e)?;
        let i = e.index as usize;
        self.unlink(e);
        for child in 0..self.nodes.len() {
            if self.slots[child].alive && self.nodes[child].parent == e {
                let pose = self.poses[child];
                let (yaw, pitch, roll) = pose.rotation.to_euler(EulerRot::YXZ);
                self.nodes[child].parent = Entity::NONE;
                self.nodes[child].local = Transform {
                    position: pose.position,
                    yaw: yaw.to_degrees(),
                    pitch: pitch.to_degrees(),
                    roll: roll.to_degrees(),
                    scale: pose.scale,
                };
            }
        }
        self.slots[i].alive = false;
        self.alive -= 1;
        Ok(())
    }

    pub fn node(&self, e: Entity) -> Option<&Node> {
        self.is_alive(e).then(|| &self.nodes[e.index as usize])
    }

    pub fn node_mut(&mut self, e: Entity) -> Result<&mut Node, Reject> {
        self.check(e)?;
        Ok(&mut self.nodes[e.index as usize])
    }

    pub fn set_parent(&mut self, e: Entity, parent: Entity) -> Result<(), Reject> {
        self.check(e)?;
        if !parent.is_none() {
            self.check(parent)?;
            let mut up = parent;
            while !up.is_none() {
                if up == e {
                    return Err(Reject::Cycle);
                }
                up = self.nodes[up.index as usize].parent;
            }
        }
        self.unlink(e);
        self.nodes[e.index as usize].parent = parent;
        if !parent.is_none() {
            self.slots[parent.index as usize].children += 1;
        }
        Ok(())
    }

    fn unlink(&mut self, e: Entity) {
        let parent = self.nodes[e.index as usize].parent;
        if !parent.is_none() {
            self.slots[parent.index as usize].children -= 1;
        }
    }

    pub fn has_children(&self, e: Entity) -> bool {
        self.is_alive(e) && self.slots[e.index as usize].children > 0
    }

    /// The live entities whose parent is `e`.
    pub fn children(&self, e: Entity) -> impl Iterator<Item = Entity> + '_ {
        let any = self.has_children(e);
        self.entities()
            .take(if any { usize::MAX } else { 0 })
            .filter(move |(_, n)| n.parent == e)
            .map(|(c, _)| c)
    }

    /// Whether `e` was visible when the step began.
    pub fn was_visible(&self, e: Entity) -> bool {
        self.is_alive(e) && self.slots[e.index as usize].shown
    }

    /// Placement changed discontinuously: rendering must not interpolate it.
    pub fn snap(&mut self, e: Entity) {
        if self.is_alive(e) {
            self.slots[e.index as usize].snapped = true;
        }
    }

    /// Whether `e` or a parent of it was spawned or snapped this step:
    /// placing it again in the same step adds no visible jump.
    pub fn snapped_this_step(&self, e: Entity) -> bool {
        let mut at = e;
        while self.is_alive(at) {
            if self.slots[at.index as usize].snapped {
                return true;
            }
            at = self.nodes[at.index as usize].parent;
        }
        false
    }

    /// The actors visible when the step began and still visible, not
    /// snapped, whose resolved placement went farther than `distance` or
    /// turned more than `degrees` since the previous step; parents first.
    pub fn jumps(&self, distance: f32, degrees: f32) -> Vec<Entity> {
        let mut out: Vec<Entity> = self
            .entities()
            .filter(|(e, n)| {
                let (i, slot) = (e.index as usize, &self.slots[e.index as usize]);
                let (a, b) = (self.previous[i], self.poses[i]);
                n.kind == Kind::Actor
                    && n.visible
                    && slot.shown
                    && !slot.snapped
                    && (a.position.distance(b.position) > distance
                        || a.rotation.angle_between(b.rotation).to_degrees() > degrees)
            })
            .map(|(e, _)| e)
            .collect();
        out.sort_by_key(|&e| self.depth(e));
        out
    }

    fn depth(&self, e: Entity) -> u32 {
        let (mut depth, mut at) = (0, self.nodes[e.index as usize].parent);
        while self.is_alive(at) {
            depth += 1;
            at = self.nodes[at.index as usize].parent;
        }
        depth
    }

    /// Puts `e` back where the previous step left it, whatever its parent
    /// is now.
    pub fn hold(&mut self, e: Entity) {
        if !self.is_alive(e) {
            return;
        }
        let i = e.index as usize;
        let was = self.previous[i];
        let parent = self
            .current_pose(self.nodes[i].parent)
            .filter(|p| p.scale != 0.0)
            .unwrap_or(IDENTITY_POSE);
        let inverse = parent.rotation.inverse();
        let (yaw, pitch, roll) = (inverse * was.rotation).to_euler(EulerRot::YXZ);
        self.nodes[i].local = Transform {
            position: inverse * (was.position - parent.position) / parent.scale,
            yaw: yaw.to_degrees(),
            pitch: pitch.to_degrees(),
            roll: roll.to_degrees(),
            scale: was.scale / parent.scale,
        };
    }

    /// The live entities in slot order.
    pub fn entities(&self) -> impl Iterator<Item = (Entity, &Node)> {
        self.slots
            .iter()
            .zip(&self.nodes)
            .enumerate()
            .filter(|(_, (s, _))| s.alive)
            .map(|(i, (s, n))| {
                (
                    Entity {
                        index: i as u32,
                        generation: s.generation,
                    },
                    n,
                )
            })
    }

    /// World placement from the current local placements, without waiting
    /// for [`World::resolve`].
    pub fn current_pose(&self, e: Entity) -> Option<Pose> {
        let node = self.node(e)?;
        let local = Pose {
            position: node.local.position,
            rotation: node.local.rotation(),
            scale: node.local.scale,
        };
        if node.parent.is_none() {
            return Some(local);
        }
        let p = self.current_pose(node.parent)?;
        Some(Pose {
            position: p.position + p.rotation * (local.position * p.scale),
            rotation: p.rotation * local.rotation,
            scale: p.scale * local.scale,
        })
    }

    /// Moves `e` so its world position is `at`, whatever its parent.
    pub fn set_world_position(&mut self, e: Entity, at: Vec3) -> Result<(), Reject> {
        let parent = self.node(e).ok_or(Reject::Stale)?.parent;
        let local = match self.current_pose(parent) {
            Some(p) if p.scale != 0.0 => p.rotation.inverse() * (at - p.position) / p.scale,
            _ => at,
        };
        self.nodes[e.index as usize].local.position = local;
        Ok(())
    }

    /// World placement as of the last [`World::resolve`].
    pub fn pose(&self, e: Entity) -> Option<Pose> {
        self.is_alive(e).then(|| self.poses[e.index as usize])
    }

    /// Placement between the previous and the last step for rendering,
    /// `alpha` from [`crate::clock::FixedClock::alpha`].
    pub fn interpolated(&self, e: Entity, alpha: f32) -> Option<Pose> {
        let now = self.pose(e)?;
        let before = self.previous[e.index as usize];
        Some(Pose {
            position: before.position.lerp(now.position, alpha),
            rotation: before.rotation.slerp(now.rotation, alpha),
            scale: before.scale + (now.scale - before.scale) * alpha,
        })
    }

    /// Start of a step: the resolved placement becomes the previous one.
    pub fn begin_step(&mut self) {
        self.previous.copy_from_slice(&self.poses);
        for (slot, node) in self.slots.iter_mut().zip(&self.nodes) {
            slot.shown = node.visible;
            slot.snapped = false;
        }
    }

    /// Computes every world placement from the local ones, parents first.
    /// What a snapped parent carries is snapped with it.
    pub fn resolve(&mut self) {
        self.stamp += 1;
        for i in 0..self.slots.len() {
            if !self.slots[i].alive || self.resolve_stamp[i] == self.stamp {
                continue;
            }
            self.chain.clear();
            let mut at = i as u32;
            loop {
                self.chain.push(at);
                let parent = self.nodes[at as usize].parent;
                if parent.is_none() || self.resolve_stamp[parent.index as usize] == self.stamp {
                    break;
                }
                at = parent.index;
            }
            while let Some(j) = self.chain.pop() {
                let node = &self.nodes[j as usize];
                let local = Pose {
                    position: node.local.position,
                    rotation: node.local.rotation(),
                    scale: node.local.scale,
                };
                let pose = if node.parent.is_none() {
                    local
                } else {
                    let parent = node.parent.index as usize;
                    if self.slots[parent].snapped {
                        self.slots[j as usize].snapped = true;
                    }
                    let p = self.poses[parent];
                    Pose {
                        position: p.position + p.rotation * (local.position * p.scale),
                        rotation: p.rotation * local.rotation,
                        scale: p.scale * local.scale,
                    }
                };
                self.poses[j as usize] = pose;
                self.resolve_stamp[j as usize] = self.stamp;
            }
        }
        for ((slot, pose), previous) in self.slots.iter().zip(&self.poses).zip(&mut self.previous) {
            if slot.alive && slot.snapped {
                *previous = *pose;
            }
        }
    }
}
