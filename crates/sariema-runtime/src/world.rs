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

fn local_pose(t: &Transform) -> Pose {
    Pose {
        position: t.position,
        rotation: t.rotation(),
        scale: t.scale,
    }
}

fn compose(parent: Pose, local: Pose) -> Pose {
    Pose {
        position: parent.position + parent.rotation * (local.position * parent.scale),
        rotation: parent.rotation * local.rotation,
        scale: parent.scale * local.scale,
    }
}

/// How far a visible actor may be seen to go in one step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    metres: f32,
    degrees: f32,
    /// A turn from rotation `a` to `b` is past `degrees` when `|a·b|` is
    /// below `cos(degrees / 2)`.
    cos_half: f32,
}

impl Limits {
    pub fn new(metres: f32, degrees: f32) -> Limits {
        Limits {
            metres,
            degrees,
            cos_half: (degrees.to_radians() / 2.0).cos(),
        }
    }

    /// Whether going `(metres, degrees)` in one step is past the limits.
    pub fn passed(&self, (metres, degrees): (f32, f32)) -> bool {
        metres > self.metres || degrees > self.degrees
    }

    fn passed_between(&self, a: Pose, b: Pose) -> bool {
        a.position.distance_squared(b.position) > self.metres * self.metres
            || a.rotation.dot(b.rotation).abs() < self.cos_half
    }
}

/// How far `a` is from `b`: metres and degrees.
fn between(a: Pose, b: Pose) -> (f32, f32) {
    let turn = a.rotation.conjugate() * b.rotation;
    (
        a.position.distance(b.position),
        (2.0 * turn.xyz().length().atan2(turn.w.abs())).to_degrees(),
    )
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    generation: u32,
    used: bool,
    alive: bool,
    /// Visible when the step began.
    shown: bool,
    /// Spawned or placed this step.
    placed: bool,
    /// Placed this step by a forced teleport.
    forced: bool,
    /// Its parent changed this step.
    reparented: bool,
    /// This step's frames draw it from where it ends.
    snapped: bool,
    children: u32,
}

pub struct World {
    max_entities: u32,
    slots: Vec<Slot>,
    nodes: Vec<Node>,
    poses: Vec<Pose>,
    previous: Vec<Pose>,
    /// Each local placement when the step began.
    start: Vec<Transform>,
    resolve_stamp: Vec<u64>,
    stamp: u64,
    chain: Vec<u32>,
    /// The live slots, parents first, as the last resolve placed them.
    order: Vec<u32>,
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
            start: Vec::new(),
            resolve_stamp: Vec::new(),
            stamp: 0,
            chain: Vec::new(),
            order: Vec::new(),
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
            self.start.resize(i + 1, Transform::IDENTITY);
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
            placed: true,
            ..Slot::default()
        };
        self.start[i] = Transform::IDENTITY;
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
                self.slots[child].reparented = true;
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
        self.can_parent(e, parent)?;
        if self.nodes[e.index as usize].parent == parent {
            return Ok(());
        }
        self.unlink(e);
        self.nodes[e.index as usize].parent = parent;
        if !parent.is_none() {
            self.slots[parent.index as usize].children += 1;
        }
        self.slots[e.index as usize].reparented = true;
        Ok(())
    }

    /// Whether [`World::set_parent`] would take `parent` for `e`.
    pub fn can_parent(&self, e: Entity, parent: Entity) -> Result<(), Reject> {
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

    /// `e` was placed discontinuously this step.
    pub fn place(&mut self, e: Entity) {
        if self.is_alive(e) {
            self.slots[e.index as usize].placed = true;
        }
    }

    /// A forced teleport placed `e` this step.
    pub fn force(&mut self, e: Entity) {
        if self.is_alive(e) {
            self.slots[e.index as usize].forced = true;
        }
    }

    /// Whether a forced teleport placed `e` this step: itself, or a carrier
    /// it has had since the step began.
    pub fn forced(&self, e: Entity) -> bool {
        let mut at = e;
        while self.is_alive(at) {
            let slot = self.slots[at.index as usize];
            if slot.forced {
                return true;
            }
            if slot.reparented {
                return false;
            }
            at = self.nodes[at.index as usize].parent;
        }
        false
    }

    /// The first actor from place `from` on, in the parents-first order of
    /// the last [`World::resolve`], that was visible when the step began,
    /// is still visible, was placed by no forced teleport, and is past
    /// `limits` from where the previous step left it; with its place. With
    /// `fresh`, it is judged where its local placements put it now, for
    /// after something that carries it was put back.
    pub fn next_jump(&self, from: usize, limits: Limits, fresh: bool) -> Option<(usize, Entity)> {
        (from..self.order.len()).find_map(|k| {
            let i = self.order[k] as usize;
            let (node, slot) = (&self.nodes[i], self.slots[i]);
            if node.kind != Kind::Actor || !node.visible || !slot.shown {
                return None;
            }
            let e = Entity {
                index: i as u32,
                generation: slot.generation,
            };
            let now = if fresh {
                self.current_pose(e)?
            } else {
                self.poses[i]
            };
            (limits.passed_between(self.previous[i], now) && !self.forced(e)).then_some((k, e))
        })
    }

    /// Whether `e` was past `limits` from where the previous step left it
    /// when the world was last resolved.
    pub fn went_past(&self, e: Entity, limits: Limits) -> bool {
        self.is_alive(e)
            && limits.passed_between(
                self.previous[e.index as usize],
                self.poses[e.index as usize],
            )
    }

    /// How far `level`'s own change of local placement since the step began
    /// carries `e`, in metres and degrees: `e` as it is, against `e` had
    /// `level` (`e` itself or a carrier of it) kept its placement.
    pub fn carried_by(&self, e: Entity, level: Entity) -> (f32, f32) {
        if !self.is_alive(level) {
            return (0.0, 0.0);
        }
        let kept = (level, self.start[level.index as usize]);
        match (self.current_pose(e), self.pose_with(e, Some(kept))) {
            (Some(now), Some(then)) => between(now, then),
            _ => (0.0, 0.0),
        }
    }

    /// Whether `e`'s local placement changed since the step began.
    pub fn changed(&self, e: Entity) -> bool {
        self.is_alive(e) && self.nodes[e.index as usize].local != self.start[e.index as usize]
    }

    /// Puts `e` back where the previous step left it, whatever its parent
    /// is now. A parent scaled to nothing cannot hold it anywhere, so `e`
    /// leaves it.
    pub fn hold(&mut self, e: Entity) {
        if !self.is_alive(e) {
            return;
        }
        let i = e.index as usize;
        let was = self.previous[i];
        let parent = match self.current_pose(self.nodes[i].parent) {
            Some(p) if p.scale.is_normal() => p,
            Some(_) => {
                self.unlink(e);
                self.nodes[i].parent = Entity::NONE;
                self.slots[i].reparented = true;
                IDENTITY_POSE
            }
            None => IDENTITY_POSE,
        };
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
        self.pose_with(e, None)
    }

    /// Like [`World::current_pose`], with `kept`'s entity at the given local
    /// placement instead of its own.
    fn pose_with(&self, e: Entity, kept: Option<(Entity, Transform)>) -> Option<Pose> {
        let node = self.node(e)?;
        let local = match kept {
            Some((k, t)) if k == e => local_pose(&t),
            _ => local_pose(&node.local),
        };
        if node.parent.is_none() {
            return Some(local);
        }
        Some(compose(self.pose_with(node.parent, kept)?, local))
    }

    /// Where `e` would be with `parent` as its parent, keeping its local
    /// placement.
    pub fn pose_under(&self, e: Entity, parent: Entity) -> Option<Pose> {
        let local = local_pose(&self.node(e)?.local);
        if parent.is_none() {
            return Some(local);
        }
        Some(compose(self.current_pose(parent)?, local))
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

    /// Start of a step: the resolved placement becomes the previous one,
    /// and what the step changes is counted from here.
    pub fn begin_step(&mut self) {
        self.previous.copy_from_slice(&self.poses);
        for ((slot, node), start) in self.slots.iter_mut().zip(&self.nodes).zip(&mut self.start) {
            *slot = Slot {
                shown: node.visible,
                placed: false,
                forced: false,
                reparented: false,
                snapped: false,
                ..*slot
            };
            *start = node.local;
        }
    }

    /// Computes every world placement from the local ones, parents first.
    pub fn resolve(&mut self) {
        self.stamp += 1;
        self.order.clear();
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
                let local = local_pose(&node.local);
                self.poses[j as usize] = if node.parent.is_none() {
                    local
                } else {
                    compose(self.poses[node.parent.index as usize], local)
                };
                self.resolve_stamp[j as usize] = self.stamp;
                self.order.push(j);
            }
        }
    }

    /// Decides, after the last [`World::resolve`] of a step, where its
    /// frames draw each entity from. A visible actor is drawn from where it
    /// ends when it appeared this step, or when it went past `limits` under
    /// a forced teleport ([`World::forced`]) or on an `exempt` (covered or
    /// cut) step; otherwise from where the previous step left it. Anything
    /// else is drawn from where it ends when it was spawned, placed or
    /// given another parent this step, or when what carries it is.
    pub fn settle(&mut self, limits: Limits, exempt: bool) {
        for k in 0..self.order.len() {
            let i = self.order[k] as usize;
            let (node, slot) = (&self.nodes[i], self.slots[i]);
            let snapped = if node.kind == Kind::Actor {
                let e = Entity {
                    index: i as u32,
                    generation: slot.generation,
                };
                node.visible
                    && (!slot.shown
                        || ((exempt || self.forced(e))
                            && limits.passed_between(self.previous[i], self.poses[i])))
            } else {
                slot.placed
                    || slot.reparented
                    || (!node.parent.is_none() && self.slots[node.parent.index as usize].snapped)
            };
            self.slots[i].snapped = snapped;
            if snapped {
                self.previous[i] = self.poses[i];
            }
        }
    }
}
