// SPDX-License-Identifier: Apache-2.0

//! The collision stage of a step: the runtime's actors as the collision
//! module's actors, its movement intents carried out with collision, and the
//! poses and events that come back written into the world before the
//! no-jump measurement judges them.

use std::f32::consts::PI;

use glam::{Quat, Vec2, Vec3};
use sariema_core::STEP_SECONDS;
use sariema_format::collision::{CollisionPack, Traversal};
use sariema_format::script::{Anchor, Command, Controller, Entity, Event, Kind, name_id};
use sariema_physics::{
    ActorCollider, ActorDesc, ActorId, ColliderId, EventKind, Intent, World, actor_collider,
    wrap_angle,
};
use sariema_runtime::events::Happening;
use sariema_runtime::motion::Goal;
use sariema_runtime::outbox::PhysicsIntent;
use sariema_runtime::state::State;

use crate::anim::Animation;
use crate::settings::EngineSettings;

/// The runtime's yaw (degrees, 0 facing −Z) as the collision module's
/// (radians, 0 facing +Z).
pub fn to_physics(yaw: f32) -> f32 {
    wrap_angle(yaw.to_radians() + PI)
}

pub fn to_runtime(yaw: f32) -> f32 {
    (yaw - PI).to_degrees()
}

struct Mapped {
    entity: Entity,
    id: ActorId,
    body: u32,
    /// Where the collision module has it, as the runtime last saw it; the
    /// runtime moving it elsewhere poses it there.
    written: (Vec3, f32),
    /// m/s of the walk it was last given, before its speed scale.
    speed: f32,
    /// Placed by the entity carrying it, with its collider off.
    carried: bool,
}

struct Traversing {
    entity: Entity,
    path: Vec<Vec3>,
    /// Arc length at each point of `path`.
    along: Vec<f32>,
    /// Name, point, and the fraction of the way at which the path passes
    /// nearest to it.
    contacts: Vec<(u32, Vec3, f32)>,
    yaw: f32,
    steps: u32,
    done: u32,
}

pub struct Physics {
    pub world: World,
    pack: CollisionPack,
    bodies: Vec<(u32, ActorCollider)>,
    /// By entity slot.
    actors: Vec<Option<Mapped>>,
    /// The entity each collision actor stands for; NONE while it waits in
    /// `free`.
    owners: Vec<Entity>,
    free: Vec<(u32, ActorId)>,
    triggers: Vec<u32>,
    /// The entity that last switched each collider.
    switched: Vec<Entity>,
    traversals: Vec<Traversing>,
    intents: Vec<PhysicsIntent>,
    events: Vec<sariema_physics::Event>,
    knockback_s: f32,
    contact_blend: f32,
    /// Degrees per step.
    max_turn: f32,
}

impl Physics {
    pub fn new(
        pack: CollisionPack,
        settings: &EngineSettings,
        turn_rate: f32,
    ) -> Result<Physics, String> {
        let mut physics = settings.physics.clone();
        physics.step_seconds = STEP_SECONDS;
        physics.controller.max_yaw_speed = turn_rate.to_radians();
        let world = World::from_pack(physics, &pack).map_err(|e| e.to_string())?;
        Ok(Physics {
            world,
            bodies: pack
                .actors
                .iter()
                .map(|b| (name_id(&b.name), actor_collider(b)))
                .collect(),
            triggers: pack.triggers.iter().map(|t| name_id(&t.name)).collect(),
            switched: vec![Entity::NONE; pack.colliders.len()],
            pack,
            actors: Vec::new(),
            owners: Vec::new(),
            free: Vec::new(),
            traversals: Vec::new(),
            intents: Vec::new(),
            events: Vec::new(),
            knockback_s: settings.knockback_s,
            contact_blend: settings.contact_blend,
            max_turn: turn_rate * STEP_SECONDS,
        })
    }

    pub fn step(&mut self, s: &mut State, anim: &mut Animation) {
        self.track(s);
        std::mem::swap(&mut self.intents, &mut s.outbox.physics);
        for intent in &self.intents {
            if let PhysicsIntent::Teleport {
                entity,
                position,
                yaw,
            } = *intent
                && let Some(m) = self
                    .actors
                    .get_mut(entity.index as usize)
                    .and_then(Option::as_mut)
                    .filter(|m| m.entity == entity)
            {
                m.written = (position, yaw);
            }
        }
        self.pose_moved(s);
        for k in 0..self.intents.len() {
            let intent = self.intents[k];
            self.apply(s, intent);
        }
        self.intents.clear();
        self.drive_hero(s);
        self.traverse(s, anim);
        self.events.clear();
        self.world.step(&mut self.events);
        self.write_back(s);
        self.report(s);
        self.finish_traversals(s);
    }

    /// The teleports and stops the runtime gave after this step's collision
    /// stage, putting back a move that jumped, for the module to know before
    /// the next step.
    pub fn after_step(&mut self, s: &State) {
        for &intent in &s.outbox.physics {
            if matches!(
                intent,
                PhysicsIntent::Teleport { .. } | PhysicsIntent::Stop { .. }
            ) {
                self.halt(s, intent);
            }
        }
    }

    /// Whether `e` is in the middle of a traversal.
    fn traversing(&self, e: Entity) -> bool {
        self.traversals.iter().any(|t| t.entity == e)
    }

    fn mapped(&self, e: Entity) -> Option<&Mapped> {
        self.actors
            .get(e.index as usize)?
            .as_ref()
            .filter(|m| m.entity == e)
    }

    fn mapped_mut(&mut self, e: Entity) -> Option<&mut Mapped> {
        self.actors
            .get_mut(e.index as usize)?
            .as_mut()
            .filter(|m| m.entity == e)
    }

    /// One collision actor per live actor entity. A despawned entity's
    /// collision actor goes inactive and waits for the next one with the
    /// same body.
    fn track(&mut self, s: &State) {
        for i in 0..self.actors.len() {
            let gone = self.actors[i]
                .as_ref()
                .is_some_and(|m| !s.world.is_alive(m.entity));
            if gone && let Some(m) = self.actors[i].take() {
                self.retire(m);
            }
        }
        for (entity, node) in s.world.entities() {
            if node.kind != Kind::Actor || self.mapped(entity).is_some() {
                continue;
            }
            let i = entity.index as usize;
            if let Some(old) = self.actors.get_mut(i).and_then(Option::take) {
                self.retire(old);
            }
            let Some(pose) = s.world.current_pose(entity) else {
                continue;
            };
            let body = if self.bodies.iter().any(|(b, _)| *b == node.asset) {
                node.asset
            } else {
                0
            };
            let yaw = node.local.yaw;
            let id = self.add(body, pose.position, yaw);
            self.owners[id.0 as usize] = entity;
            if self.actors.len() <= i {
                self.actors.resize_with(i + 1, || None);
            }
            self.actors[i] = Some(Mapped {
                entity,
                id,
                body,
                written: (pose.position, yaw),
                speed: 0.0,
                carried: !node.parent.is_none(),
            });
        }
    }

    /// An actor without a body of its own in the pack gets the pack's first.
    fn add(&mut self, body: u32, position: Vec3, yaw: f32) -> ActorId {
        if let Some(k) = self.free.iter().position(|&(b, _)| b == body) {
            let (_, id) = self.free.swap_remove(k);
            self.world.set_actor_active(id, true, &mut self.events);
            self.world.apply(Intent::Teleport {
                actor: id,
                position,
                yaw: to_physics(yaw),
            });
            self.world.apply(Intent::Stop { actor: id });
            return id;
        }
        let collider = self
            .bodies
            .iter()
            .find(|(b, _)| *b == body)
            .or(self.bodies.first())
            .map_or_else(ActorCollider::default, |b| b.1);
        let id = self
            .world
            .add_actor(ActorDesc {
                position,
                yaw: to_physics(yaw),
                collider,
                walk_speed: 1.0,
                ..ActorDesc::default()
            })
            .expect("bodies from a valid collision pack are valid");
        self.world.prepare_paths(id);
        self.owners.push(Entity::NONE);
        id
    }

    fn retire(&mut self, m: Mapped) {
        self.world.set_actor_active(m.id, false, &mut self.events);
        self.owners[m.id.0 as usize] = Entity::NONE;
        self.free.push((m.body, m.id));
        self.traversals.retain(|t| t.entity != m.entity);
    }

    fn apply(&mut self, s: &mut State, intent: PhysicsIntent) {
        match intent {
            PhysicsIntent::MoveTo {
                entity,
                target,
                speed,
            } => {
                let scale = speed_scale(s, entity);
                self.traversals.retain(|t| t.entity != entity);
                let Some(m) = self.mapped_mut(entity) else {
                    return;
                };
                m.speed = speed;
                let id = m.id;
                self.world.apply(Intent::SetSpeedScale {
                    actor: id,
                    scale: speed * scale,
                });
                self.world.apply(Intent::MoveTo {
                    actor: id,
                    target,
                    face: None,
                });
            }
            PhysicsIntent::Velocity { entity, velocity } => {
                self.traversals.retain(|t| t.entity != entity);
                self.velocity(s, entity, velocity);
            }
            PhysicsIntent::SpeedScale { entity, scale } => {
                if let Some(m) = self.mapped(entity) {
                    let (id, speed) = (m.id, m.speed);
                    self.world.apply(Intent::SetSpeedScale {
                        actor: id,
                        scale: speed * scale,
                    });
                }
            }
            PhysicsIntent::Face { entity, yaw } => {
                if let Some(m) = self.mapped(entity) {
                    let id = m.id;
                    self.world.apply(Intent::Face {
                        actor: id,
                        yaw: to_physics(yaw),
                    });
                }
                s.motion.mover_mut(entity).face = None;
            }
            PhysicsIntent::Knockback { entity, impulse } => {
                if let Some(m) = self.mapped(entity) {
                    let id = m.id;
                    self.world.apply(Intent::Knockback {
                        actor: id,
                        push: impulse * self.knockback_s,
                        steps: (self.knockback_s / STEP_SECONDS).round().max(1.0) as u32,
                    });
                }
            }
            PhysicsIntent::Collider { entity, enabled } => {
                let asset = s.world.node(entity).map_or(0, |n| n.asset);
                if let Some(c) = self
                    .pack
                    .colliders
                    .iter()
                    .position(|c| name_id(&c.name) == asset)
                {
                    self.switched[c] = entity;
                    self.world.apply(Intent::SetColliderEnabled {
                        collider: ColliderId(c as u32),
                        enabled,
                    });
                }
            }
            PhysicsIntent::Traverse {
                entity,
                traversal,
                duration,
            } => self.start_traversal(s, entity, traversal, duration),
            PhysicsIntent::Teleport { .. } | PhysicsIntent::Stop { .. } => self.halt(s, intent),
            PhysicsIntent::SkippedWalk { entity, target } => {
                let Some(m) = self.mapped(entity) else {
                    return;
                };
                let (id, yaw) = (m.id, m.written.1);
                if self.world.reachable(id, target) {
                    return;
                }
                let position = self.world.actor(id).position;
                self.halt(
                    s,
                    PhysicsIntent::Teleport {
                        entity,
                        position,
                        yaw,
                    },
                );
                s.events.push(Event::Blocked { entity });
                s.happen(Happening::Blocked(entity));
            }
        }
    }

    fn halt(&mut self, s: &State, intent: PhysicsIntent) {
        match intent {
            PhysicsIntent::Teleport {
                entity,
                position,
                yaw,
            } => {
                self.traversals.retain(|t| t.entity != entity);
                if s.world.node(entity).is_some_and(|n| !n.parent.is_none()) {
                    return;
                }
                let Some(m) = self.mapped_mut(entity) else {
                    return;
                };
                m.written = (position, yaw);
                let id = m.id;
                self.world.apply(Intent::Teleport {
                    actor: id,
                    position,
                    yaw: to_physics(yaw),
                });
            }
            PhysicsIntent::Stop { entity } => {
                self.traversals.retain(|t| t.entity != entity);
                if let Some(m) = self.mapped_mut(entity) {
                    m.speed = 0.0;
                    let id = m.id;
                    self.world.apply(Intent::Stop { actor: id });
                }
            }
            _ => {}
        }
    }

    fn velocity(&mut self, s: &State, entity: Entity, velocity: Vec3) {
        let scale = speed_scale(s, entity);
        let Some(m) = self.mapped_mut(entity) else {
            return;
        };
        let flat = Vec2::new(velocity.x, velocity.z);
        let speed = flat.length();
        m.speed = speed;
        let id = m.id;
        if speed > 0.0 {
            self.world.apply(Intent::SetSpeedScale {
                actor: id,
                scale: speed * scale,
            });
        }
        self.world.apply(Intent::Move {
            actor: id,
            stick: if speed > 0.0 {
                flat / speed
            } else {
                Vec2::ZERO
            },
        });
    }

    /// The player's hero walks where the runtime's control points it.
    fn drive_hero(&mut self, s: &State) {
        let hero = s.control.hero;
        if s.control.by != Controller::Player || self.traversing(hero) {
            return;
        }
        let velocity = match s.motion.get(hero).map(|m| m.goal) {
            Some(Goal::Velocity(v)) => v,
            Some(Goal::Idle) => Vec3::ZERO,
            _ => return,
        };
        let moving = self.mapped(hero).is_some_and(|m| m.speed > 0.0);
        if velocity != Vec3::ZERO || moving {
            self.velocity(s, hero, velocity);
        }
    }

    /// An actor the runtime moved since the last write-back (a tween, a
    /// spring, a follow) is posed where it put it, collider off; a carried
    /// actor is posed where its carrier holds it.
    fn pose_moved(&mut self, s: &State) {
        for m in self.actors.iter_mut().flatten() {
            let Some(node) = s.world.node(m.entity) else {
                continue;
            };
            m.carried = !node.parent.is_none();
            let (position, yaw) = if m.carried {
                let Some(pose) = s.world.current_pose(m.entity) else {
                    continue;
                };
                let (y, _, _) = pose.rotation.to_euler(glam::EulerRot::YXZ);
                (pose.position, y.to_degrees())
            } else if (node.local.position, node.local.yaw) != m.written {
                (node.local.position, node.local.yaw)
            } else {
                continue;
            };
            m.written = (position, yaw);
            self.world.apply(Intent::Pose {
                actor: m.id,
                position,
                yaw: to_physics(yaw),
            });
        }
    }

    fn start_traversal(&mut self, s: &mut State, entity: Entity, traversal: u32, duration: f32) {
        let found = self
            .pack
            .traversals
            .iter()
            .find(|t| name_id(&t.name) == traversal);
        let start = s.world.node(entity).map(|n| n.local.position);
        let (Some(t), Some(start), true) = (found, start, self.mapped(entity).is_some()) else {
            s.events.push(Event::Blocked { entity });
            s.happen(Happening::Blocked(entity));
            return;
        };
        let run = traversing(entity, t, start, duration);
        self.traversals.retain(|r| r.entity != entity);
        self.traversals.push(run);
    }

    /// Each traversal's pose for this step: along its points, eased onto
    /// each contact so that the actor's socket or bone of that name (its
    /// feet when it has neither) lands on it, and turning to the
    /// traversal's facing no faster than the runtime's turn rate.
    fn traverse(&mut self, s: &State, anim: &mut Animation) {
        for t in &mut self.traversals {
            let Some(m) = self
                .actors
                .get_mut(t.entity.index as usize)
                .and_then(Option::as_mut)
                .filter(|m| m.entity == t.entity)
            else {
                continue;
            };
            t.done = (t.done + 1).min(t.steps);
            let f = t.done as f32 / t.steps as f32;
            let total = t.along[t.along.len() - 1];
            let mut at = point_along(&t.path, &t.along, f * total);
            let turn = ((t.yaw - m.written.1 + 180.0).rem_euclid(360.0) - 180.0)
                .clamp(-self.max_turn, self.max_turn);
            let yaw = m.written.1 + turn;
            let scale = s.world.node(t.entity).map_or(1.0, |n| n.local.scale);
            let rotation = Quat::from_rotation_y(yaw.to_radians());
            for &(name, point, when) in &t.contacts {
                let weight = 1.0 - (f - when).abs() / self.contact_blend.max(f32::EPSILON);
                if weight <= 0.0 {
                    continue;
                }
                let offset = anim
                    .point(t.entity, Anchor::Origin, name)
                    .unwrap_or(Vec3::ZERO);
                at = at.lerp(point - rotation * (offset * scale), smooth(weight));
            }
            m.written = (at, yaw);
            self.world.apply(Intent::Pose {
                actor: m.id,
                position: at,
                yaw: to_physics(yaw),
            });
        }
    }

    fn finish_traversals(&mut self, s: &mut State) {
        let mut k = 0;
        while k < self.traversals.len() {
            if self.traversals[k].done < self.traversals[k].steps {
                k += 1;
                continue;
            }
            let entity = self.traversals.remove(k).entity;
            if let Some(m) = self.mapped(entity) {
                let id = m.id;
                self.world.apply(Intent::Stop { actor: id });
            }
            s.events.push(Event::Arrived { entity });
            s.happen(Happening::Arrived(entity));
        }
    }

    fn write_back(&mut self, s: &mut State) {
        for m in self.actors.iter_mut().flatten() {
            if m.carried {
                continue;
            }
            let a = self.world.actor(m.id);
            let Ok(node) = s.world.node_mut(m.entity) else {
                continue;
            };
            let yaw = if a.yaw == to_physics(m.written.1) {
                m.written.1
            } else {
                to_runtime(a.yaw)
            };
            node.local.position = a.position;
            node.local.yaw = yaw;
            m.written = (a.position, yaw);
        }
    }

    fn report(&mut self, s: &mut State) {
        for k in 0..self.events.len() {
            let e = self.events[k];
            let entity = self.owners[e.actor.0 as usize];
            if entity.is_none() {
                continue;
            }
            match e.kind {
                EventKind::Arrived | EventKind::Blocked(_) => {
                    if s.motion.walking(entity) {
                        s.motion.mover_mut(entity).goal = Goal::Idle;
                    }
                    if let Some(m) = self.mapped_mut(entity) {
                        m.speed = 0.0;
                    }
                    if e.kind == EventKind::Arrived {
                        s.events.push(Event::Arrived { entity });
                        s.happen(Happening::Arrived(entity));
                    } else {
                        s.events.push(Event::Blocked { entity });
                        s.happen(Happening::Blocked(entity));
                    }
                }
                EventKind::HitWall { normal, .. } => s.events.push(Event::HitWall {
                    entity,
                    normal: normal.to_array(),
                }),
                EventKind::Landed { .. } => s.events.push(Event::Landed { entity }),
                EventKind::TriggerEnter(t) => s.events.push(Event::TriggerEnter {
                    trigger: self.triggers[t.0 as usize],
                    entity,
                }),
                EventKind::TriggerExit(t) => s.events.push(Event::TriggerExit {
                    trigger: self.triggers[t.0 as usize],
                    entity,
                }),
                EventKind::TeleportRejected => s.events.push(Event::Rejected {
                    opcode: TELEPORT.opcode(),
                    code: "E_TELEPORT_BLOCKED",
                    entity,
                }),
                EventKind::EnableRefused { collider } => s.events.push(Event::Rejected {
                    opcode: SET_COLLIDER.opcode(),
                    code: "E_COLLIDER_BLOCKED",
                    entity: self.switched[collider.0 as usize],
                }),
                EventKind::PushedOut { .. } => {}
            }
        }
    }
}

const TELEPORT: Command<'static> = Command::Teleport {
    entity: Entity::NONE,
    position: [0.0; 3],
    yaw: 0.0,
    force: false,
};

const SET_COLLIDER: Command<'static> = Command::SetCollider {
    entity: Entity::NONE,
    enabled: false,
};

fn speed_scale(s: &State, e: Entity) -> f32 {
    s.motion.get(e).map_or(1.0, |m| m.speed_scale)
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn traversing(entity: Entity, t: &Traversal, start: Vec3, duration: f32) -> Traversing {
    let mut path = vec![start];
    path.extend(t.points.iter().map(|&p| Vec3::from(p)));
    let mut along = vec![0.0];
    for w in path.windows(2) {
        along.push(along[along.len() - 1] + w[0].distance(w[1]));
    }
    let total = along[along.len() - 1].max(f32::EPSILON);
    let contacts = t
        .contacts
        .iter()
        .map(|c| {
            let point = Vec3::from(c.point);
            (
                name_id(&c.name),
                point,
                nearest_along(&path, &along, point) / total,
            )
        })
        .collect();
    Traversing {
        entity,
        path,
        along,
        contacts,
        yaw: to_runtime(t.yaw),
        steps: (duration / STEP_SECONDS).round().max(1.0) as u32,
        done: 0,
    }
}

/// The arc length at which `path` passes nearest to `point`.
fn nearest_along(path: &[Vec3], along: &[f32], point: Vec3) -> f32 {
    let mut best = (f32::INFINITY, 0.0);
    for k in 1..path.len() {
        let (a, b) = (path[k - 1], path[k]);
        let span = b - a;
        let f = ((point - a).dot(span) / span.length_squared().max(f32::EPSILON)).clamp(0.0, 1.0);
        let d = (a + span * f).distance_squared(point);
        if d < best.0 {
            best = (d, along[k - 1] + f * (along[k] - along[k - 1]));
        }
    }
    best.1
}

/// The point `distance` metres along `path`, whose arc lengths are `along`.
fn point_along(path: &[Vec3], along: &[f32], distance: f32) -> Vec3 {
    for k in 1..path.len() {
        if distance <= along[k] || k + 1 == path.len() {
            let span = (along[k] - along[k - 1]).max(f32::EPSILON);
            let f = ((distance - along[k - 1]) / span).clamp(0.0, 1.0);
            return path[k - 1].lerp(path[k], f);
        }
    }
    path[0]
}
