// SPDX-License-Identifier: Apache-2.0

//! The collision stage of a step: the runtime's actors as the collision
//! module's actors, its movement intents carried out with collision, and the
//! poses and events that come back written into the world before the
//! no-jump measurement judges them. A hinged collider turns with its room's
//! scene parameter `hinge.<node>`, to the value the parameter's tween is
//! about to write, unless an actor stands in the way: then the tween waits
//! a step and the game hears `hinge_blocked`.

use std::f32::consts::PI;
use std::sync::Arc;

use glam::{Quat, Vec2, Vec3};
use sariema_core::STEP_SECONDS;
use sariema_format::collision::{CollisionPack, Traversal};
use sariema_format::script::ray::{RAY_HIT, RAY_INVALID, RAY_MISS, Ray, RayHit};
use sariema_format::script::{
    Anchor, BlockReason, Command, Controller, Ease, Entity, Event, Kind, Property, name_id,
};
use sariema_physics::{
    ActorCollider, ActorDesc, ActorId, BlockReason as PhysicsBlock, ColliderId, EventKind, Intent,
    RayFilter, RayTarget, Settings as PhysicsSettings, World, WorldBuild, actor_collider,
    hinge_turned, wrap_angle,
};
use sariema_runtime::ease::ease;
use sariema_runtime::events::Happening;
use sariema_runtime::motion::Goal;
use sariema_runtime::outbox::PhysicsIntent;
use sariema_runtime::sim::Queries;
use sariema_runtime::state::State;
use sariema_runtime::tween::Target;

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
    /// Degrees per second it turns at most; `motion.turn_rate` when none.
    turn_rate: Option<f32>,
    /// Hidden, as the collision module was last told.
    hidden: bool,
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
    /// Held at the last point instead of landing there.
    hold_end: bool,
    /// Times the distance along `path`.
    ease: Ease,
}

impl Traversing {
    fn with_capacity(points: usize, contacts: usize) -> Traversing {
        Traversing {
            entity: Entity::NONE,
            path: Vec::with_capacity(points),
            along: Vec::with_capacity(points),
            contacts: Vec::with_capacity(contacts),
            yaw: 0.0,
            steps: 0,
            done: 0,
            hold_end: false,
            ease: Ease::Linear,
        }
    }
}

pub struct Physics {
    pub world: World,
    pack: Arc<CollisionPack>,
    bodies: Vec<(u32, ActorCollider)>,
    colliders: Vec<ActorCollider>,
    /// By entity slot.
    actors: Vec<Option<Mapped>>,
    /// The entity each collision actor stands for; NONE while it waits in
    /// `free`.
    owners: Vec<Entity>,
    free: Vec<(u32, ActorId)>,
    triggers: Vec<u32>,
    /// The entity that last switched each collider.
    switched: Vec<Entity>,
    /// The `name_id` of each collider.
    names: Vec<u32>,
    traversals: Vec<Traversing>,
    /// Ended traversals, kept for their buffers.
    spare: Vec<Traversing>,
    intents: Vec<PhysicsIntent>,
    events: Vec<sariema_physics::Event>,
    swings: Vec<Swing>,
    knockback_s: f32,
    contact_blend: f32,
    /// Degrees per step.
    max_turn: f32,
}

/// A hinged collider and the scene parameter that turns it.
struct Swing {
    collider: ColliderId,
    /// Index into the pack's hinges.
    hinge: usize,
    /// `hinge.<node>`, the node named like the collider.
    param: u32,
    /// Degrees from closed it is turned now.
    angle: f32,
    /// The entity it stopped short of, while it stays stopped.
    blocked: Option<Entity>,
}

/// A room's collision built a share at a time ([`Physics::start`]).
pub struct PhysicsBuild {
    world: WorldBuild<Arc<CollisionPack>>,
    pack: Arc<CollisionPack>,
    knockback_s: f32,
    contact_blend: f32,
    turn_rate: f32,
}

impl PhysicsBuild {
    /// Builds on until about `units` of the world's work are spent
    /// ([`WorldBuild::advance`]); the collision once it is built.
    pub fn advance(&mut self, units: usize) -> Result<Option<Physics>, String> {
        let Some(world) = self.world.advance(units).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        Ok(Some(Physics::with_world(world, self)))
    }
}

impl Physics {
    pub fn new(
        pack: Arc<CollisionPack>,
        settings: &EngineSettings,
        physics: &PhysicsSettings,
        turn_rate: f32,
    ) -> Result<Physics, String> {
        let mut build = Physics::start(pack, settings, physics, turn_rate);
        loop {
            if let Some(physics) = build.advance(usize::MAX)? {
                return Ok(physics);
            }
        }
    }

    /// Starts building the collision of `pack` a share at a time: the
    /// collision [`Physics::new`] gives. `physics` are the game's settings
    /// of the module, stepped at the runtime's step and turning at most
    /// `turn_rate` degrees a second.
    pub fn start(
        pack: Arc<CollisionPack>,
        settings: &EngineSettings,
        physics: &PhysicsSettings,
        turn_rate: f32,
    ) -> PhysicsBuild {
        let mut physics = physics.clone();
        physics.step_seconds = STEP_SECONDS;
        physics.controller.max_yaw_speed = turn_rate.to_radians();
        PhysicsBuild {
            world: World::build_from_pack(physics, pack.clone()),
            pack,
            knockback_s: settings.knockback_s,
            contact_blend: settings.contact_blend,
            turn_rate,
        }
    }

    fn with_world(world: World, build: &PhysicsBuild) -> Physics {
        let pack = build.pack.clone();
        let turn_rate = build.turn_rate;
        let bodies: Vec<(u32, ActorCollider)> = (pack.actors.iter())
            .map(|b| (name_id(&b.name), actor_collider(b)))
            .collect();
        let mut colliders: Vec<ActorCollider> = Vec::new();
        for (_, collider) in &bodies {
            if !colliders.contains(collider) {
                colliders.push(*collider);
            }
        }
        if colliders.is_empty() {
            colliders.push(ActorCollider::default());
        }
        let longest = |n: fn(&Traversal) -> usize| pack.traversals.iter().map(n).max();
        let spare = match (
            longest(|t| t.points.len() + 1),
            longest(|t| t.contacts.len()),
        ) {
            (Some(points), Some(contacts)) => vec![Traversing::with_capacity(points, contacts)],
            _ => Vec::new(),
        };
        let swings = (pack.hinges.iter().enumerate())
            .map(|(k, h)| Swing {
                collider: ColliderId(h.collider),
                hinge: k,
                param: name_id(&format!(
                    "hinge.{}",
                    pack.colliders[h.collider as usize].name
                )),
                angle: h.angle_deg,
                blocked: None,
            })
            .collect();
        Physics {
            world,
            bodies,
            colliders,
            triggers: pack.triggers.iter().map(|t| name_id(&t.name)).collect(),
            switched: vec![Entity::NONE; pack.colliders.len()],
            names: pack.colliders.iter().map(|c| name_id(&c.name)).collect(),
            pack,
            actors: Vec::new(),
            owners: Vec::new(),
            free: Vec::new(),
            traversals: Vec::with_capacity(spare.len()),
            spare,
            intents: Vec::new(),
            events: Vec::new(),
            swings,
            knockback_s: build.knockback_s,
            contact_blend: build.contact_blend,
            max_turn: turn_rate * STEP_SECONDS,
        }
    }

    /// The colliders actors take in this world, each needing its walkable
    /// grid: the bodies', or the default one when there is none.
    pub fn colliders(&self) -> &[ActorCollider] {
        &self.colliders
    }

    pub fn step(&mut self, s: &mut State, anim: &mut Animation) {
        self.track(s);
        self.hide(s);
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
        self.drive_hero(s);
        self.traverse(s, anim);
        self.swing(s);
        self.events.clear();
        self.world.step(&mut self.events);
        self.write_back(s);
        self.report(s);
        self.landed(s);
        self.intents.clear();
        self.finish_traversals(s);
    }

    /// A teleport that landed drops the walk given before it, as the
    /// collision module does, so the runtime's walk ends with it; a walk
    /// given after it goes on, and a refused teleport changes nothing.
    fn landed(&self, s: &mut State) {
        for (k, intent) in self.intents.iter().enumerate() {
            let PhysicsIntent::Teleport { entity, .. } = *intent else {
                continue;
            };
            let walks_after = self.intents[k + 1..]
                .iter()
                .any(|i| matches!(*i, PhysicsIntent::MoveTo { entity: e, .. } if e == entity));
            let refused = self.events.iter().any(|e| {
                e.kind == EventKind::TeleportRejected && self.owners[e.actor.0 as usize] == entity
            });
            if !walks_after && !refused && s.motion.walking(entity) {
                s.motion.mover_mut(entity).goal = Goal::Idle;
            }
        }
    }

    /// Turns each hinged collider to the angle its parameter is about to
    /// take, or holds the parameter where the collider stays.
    fn swing(&mut self, s: &mut State) {
        let rest = s.settings.spring_rest;
        for k in 0..self.swings.len() {
            let w = &self.swings[k];
            let target = Target {
                entity: Entity::NONE,
                property: Property::Scene,
                var: w.param,
            };
            let wanted = match s.tweens.peek(&target, rest) {
                Some(next) => next,
                None => match s.scenery.param(w.param) {
                    Ok(now) => now,
                    Err(_) => continue,
                },
            };
            if wanted == w.angle {
                continue;
            }
            let pose = hinge_turned(&self.pack.hinges[w.hinge], wanted);
            match self.world.try_platform_pose(w.collider, pose) {
                Ok(()) => {
                    self.swings[k].angle = wanted;
                    self.swings[k].blocked = None;
                }
                Err(actor) => {
                    let (param, angle) = (w.param, w.angle);
                    s.tweens.hold(&target);
                    let _ = s.scenery.set_param(param, angle);
                    let entity = self.owners[actor.0 as usize];
                    if self.swings[k].blocked != Some(entity) {
                        self.swings[k].blocked = Some(entity);
                        s.events.push(Event::HingeBlocked {
                            hinge: param,
                            entity,
                        });
                    }
                }
            }
        }
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

    /// Hands every actor to `next`, the world of the room switched to: it
    /// leaves this one, whose triggers it exits as the game hears, and
    /// enters `next` where the runtime has it, with its velocity, walking
    /// on to its goal.
    pub(crate) fn hand_over(&mut self, next: &mut Physics, s: &mut State) {
        let carried: Vec<(Entity, (Vec3, f32))> = (self.actors.iter().flatten())
            .map(|m| (m.entity, self.world.actor_velocity(m.id)))
            .collect();
        self.events.clear();
        for m in self.actors.iter().flatten() {
            self.world.set_actor_active(m.id, false, &mut self.events);
        }
        self.report(s);
        self.events.clear();
        for i in 0..self.actors.len() {
            if let Some(m) = self.actors[i].take() {
                self.retire(m);
            }
        }
        next.track(s);
        for &(entity, (velocity, vertical)) in &carried {
            if let Some(m) = next.mapped(entity) {
                let id = m.id;
                next.world.set_actor_velocity(id, velocity, vertical);
            }
        }
        for (entity, _) in carried {
            match s.motion.get(entity).map(|m| m.goal) {
                Some(Goal::To { target, speed }) => next.apply(
                    s,
                    PhysicsIntent::MoveTo {
                        entity,
                        target,
                        speed,
                    },
                ),
                Some(Goal::Velocity(v)) => next.velocity(s, entity, v),
                Some(Goal::Idle) | None => {}
            }
        }
    }

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
            let turn_rate = s.motion.get(entity).and_then(|m| m.turn_rate);
            self.world.apply(Intent::SetMaxYawSpeed {
                actor: id,
                speed: turn_rate.map(f32::to_radians),
            });
            self.world.apply(Intent::SetSolid {
                actor: id,
                solid: s.motion.get(entity).is_none_or(|m| m.body),
            });
            let hidden = !node.visible;
            self.world.apply(Intent::SetHidden { actor: id, hidden });
            if s.motion.held(entity) {
                self.world.apply(Intent::Hold {
                    actor: id,
                    position: pose.position,
                    yaw: to_physics(yaw),
                });
            }
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
                turn_rate,
                hidden,
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
        self.owners.push(Entity::NONE);
        id
    }

    fn retire(&mut self, m: Mapped) {
        self.world.set_actor_active(m.id, false, &mut self.events);
        self.owners[m.id.0 as usize] = Entity::NONE;
        self.free.push((m.body, m.id));
        self.end_traversal(m.entity);
    }

    fn apply(&mut self, s: &mut State, intent: PhysicsIntent) {
        match intent {
            PhysicsIntent::MoveTo {
                entity,
                target,
                speed,
            } => {
                self.end_traversal(entity);
                let Some(m) = self.mapped_mut(entity) else {
                    return;
                };
                m.speed = speed;
                let id = m.id;
                self.world.apply(Intent::SetSpeedScale {
                    actor: id,
                    scale: speed,
                });
                self.world.apply(Intent::MoveTo {
                    actor: id,
                    target,
                    face: None,
                });
            }
            PhysicsIntent::Velocity { entity, velocity } => {
                self.end_traversal(entity);
                self.velocity(s, entity, velocity);
            }
            PhysicsIntent::SpeedScale { entity, scale } => {
                let velocity = matches!(
                    s.motion.get(entity).map(|m| m.goal),
                    Some(Goal::Velocity(_))
                );
                if let Some(m) = self.mapped(entity).filter(|_| velocity) {
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
                let Some(node) = s.world.node(entity) else {
                    return;
                };
                if node.kind == Kind::Actor {
                    if let Some(m) = self.mapped(entity) {
                        let id = m.id;
                        self.world.apply(Intent::SetSolid {
                            actor: id,
                            solid: enabled,
                        });
                    }
                    return;
                }
                let asset = node.asset;
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
            PhysicsIntent::Hold {
                entity,
                position,
                yaw,
            } => self.pose(s, entity, position, yaw, true),
            PhysicsIntent::Pose {
                entity,
                position,
                yaw,
            } => self.pose(s, entity, position, yaw, false),
            PhysicsIntent::TurnRate { entity, rate } => {
                if let Some(m) = self.mapped_mut(entity) {
                    m.turn_rate = rate;
                    let id = m.id;
                    self.world.apply(Intent::SetMaxYawSpeed {
                        actor: id,
                        speed: rate.map(f32::to_radians),
                    });
                }
            }
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
                s.events.push(Event::Blocked {
                    entity,
                    reason: BlockReason::NoPath,
                });
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
                self.end_traversal(entity);
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
                self.end_traversal(entity);
                if let Some(m) = self.mapped_mut(entity) {
                    m.speed = 0.0;
                    let id = m.id;
                    self.world.apply(Intent::Stop { actor: id });
                }
            }
            _ => {}
        }
    }

    /// Poses `entity` exactly at `position`, collider off, until its next
    /// movement intent releases it; `held`, as a hold.
    fn pose(&mut self, s: &State, entity: Entity, position: Vec3, yaw: f32, held: bool) {
        self.end_traversal(entity);
        if s.world.node(entity).is_some_and(|n| !n.parent.is_none()) {
            return;
        }
        let Some(m) = self.mapped_mut(entity) else {
            return;
        };
        m.written = (position, yaw);
        m.speed = 0.0;
        let (actor, yaw) = (m.id, to_physics(yaw));
        self.world.apply(if held {
            Intent::Hold {
                actor,
                position,
                yaw,
            }
        } else {
            Intent::Pose {
                actor,
                position,
                yaw,
            }
        });
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

    /// Tells the collision module which actors are hidden, so it can keep
    /// them out of other actors' way.
    fn hide(&mut self, s: &State) {
        for m in self.actors.iter_mut().flatten() {
            let hidden = s.world.node(m.entity).is_some_and(|n| !n.visible);
            if hidden != m.hidden {
                m.hidden = hidden;
                self.world.apply(Intent::SetHidden {
                    actor: m.id,
                    hidden,
                });
            }
        }
    }

    /// The player's hero walks where the runtime's control points it; one
    /// handed to anything else lets go of the player's stick.
    fn drive_hero(&mut self, s: &State) {
        let hero = s.control.hero;
        if self.traversing(hero) {
            return;
        }
        if s.control.by != Controller::Player {
            let idle = matches!(s.motion.get(hero).map(|m| m.goal), Some(Goal::Idle) | None);
            if idle && self.mapped(hero).is_some_and(|m| m.speed > 0.0) {
                self.velocity(s, hero, Vec3::ZERO);
            }
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
            let moved_only = !m.carried && node.local.yaw == m.written.1;
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
            self.world.apply(if moved_only {
                Intent::Shift {
                    actor: m.id,
                    position,
                }
            } else {
                Intent::Pose {
                    actor: m.id,
                    position,
                    yaw: to_physics(yaw),
                }
            });
        }
    }

    fn start_traversal(&mut self, s: &mut State, entity: Entity, traversal: u32, duration: f32) {
        let found = self
            .pack
            .traversals
            .iter()
            .position(|t| name_id(&t.name) == traversal);
        let start = s.world.node(entity).map(|n| n.local.position);
        let (Some(t), Some(start), true) = (found, start, self.mapped(entity).is_some()) else {
            s.events.push(Event::Blocked {
                entity,
                reason: BlockReason::Unavailable,
            });
            s.happen(Happening::Blocked(entity));
            return;
        };
        self.end_traversal(entity);
        if std::mem::take(&mut s.motion.mover_mut(entity).held)
            && let Some(m) = self.mapped(entity)
        {
            let actor = m.id;
            self.world.apply(Intent::Stop { actor });
        }
        let mut run = (self.spare.pop()).unwrap_or_else(|| Traversing::with_capacity(0, 0));
        traversing(&mut run, entity, &self.pack.traversals[t], start, duration);
        self.traversals.push(run);
    }

    /// Ends `entity`'s traversal, keeping its buffers for the next one.
    fn end_traversal(&mut self, entity: Entity) {
        if let Some(k) = self.traversals.iter().position(|t| t.entity == entity) {
            self.spare.push(self.traversals.remove(k));
        }
    }

    /// Each traversal's pose for this step: along its points as far as its
    /// timing curve has come, eased onto each contact so that the actor's
    /// socket or bone of that name (its feet when it has neither) lands on
    /// it, and turning to the traversal's facing no faster than the
    /// runtime's turn rate.
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
            let f = ease(t.ease, t.done as f32 / t.steps as f32);
            let total = t.along[t.along.len() - 1];
            let mut at = point_along(&t.path, &t.along, f * total);
            let max_turn = m.turn_rate.map_or(self.max_turn, |r| r * STEP_SECONDS);
            let turn = ((t.yaw - m.written.1 + 180.0).rem_euclid(360.0) - 180.0)
                .clamp(-max_turn, max_turn);
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
            let run = self.traversals.remove(k);
            let (entity, hold_end) = (run.entity, run.hold_end);
            self.spare.push(run);
            if hold_end && s.may_hold(entity) {
                s.motion.mover_mut(entity).held = true;
                if let Some(m) = self.mapped_mut(entity) {
                    m.speed = 0.0;
                    let (actor, (position, yaw)) = (m.id, m.written);
                    self.world.apply(Intent::Hold {
                        actor,
                        position,
                        yaw: to_physics(yaw),
                    });
                }
            } else if let Some(m) = self.mapped(entity) {
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
                    if let EventKind::Blocked(why) = e.kind {
                        let reason = match why {
                            PhysicsBlock::NoPath => BlockReason::NoPath,
                            PhysicsBlock::Stuck => BlockReason::Stuck,
                        };
                        s.events.push(Event::Blocked { entity, reason });
                        s.happen(Happening::Blocked(entity));
                    } else {
                        s.events.push(Event::Arrived { entity });
                        s.happen(Happening::Arrived(entity));
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
    hold: false,
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

fn traversing(run: &mut Traversing, entity: Entity, t: &Traversal, start: Vec3, duration: f32) {
    let (path, along) = (&mut run.path, &mut run.along);
    path.clear();
    path.push(start);
    path.extend(t.points.iter().map(|&p| Vec3::from(p)));
    along.clear();
    along.push(0.0);
    for w in path.windows(2) {
        along.push(along[along.len() - 1] + w[0].distance(w[1]));
    }
    let total = along[along.len() - 1].max(f32::EPSILON);
    run.contacts.clear();
    run.contacts.extend(t.contacts.iter().map(|c| {
        let point = Vec3::from(c.point);
        (
            name_id(&c.name),
            point,
            nearest_along(&run.path, &run.along, point) / total,
        )
    }));
    run.entity = entity;
    run.yaw = to_runtime(t.yaw);
    run.steps = (duration / STEP_SECONDS).round().max(1.0) as u32;
    run.done = 0;
    run.hold_end = t.hold_end;
    run.ease = Ease::from_u8(t.ease).unwrap_or(Ease::Linear);
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

impl Queries for Physics {
    fn ray_records(&self) -> Option<(usize, usize)> {
        Some((Ray::BYTES, RayHit::BYTES))
    }

    fn ray(&mut self, query: &[u8], hit: &mut [u8]) -> i32 {
        let (Some(ray), Ok(out)) = (Ray::read(query), <&mut [u8; RayHit::BYTES]>::try_from(hit))
        else {
            return RAY_INVALID;
        };
        let (origin, dir) = (Vec3::from(ray.origin), Vec3::from(ray.dir));
        if !origin.is_finite()
            || !ray.max.is_finite()
            || ray.max < 0.0
            || dir.normalize_or_zero() == Vec3::ZERO
        {
            return RAY_INVALID;
        }
        let filter = RayFilter {
            colliders: ray.colliders,
            actors: ray.actors,
            exclude_actor: self.mapped(ray.exclude).map(|m| m.id),
        };
        let Some(contact) = self.world.cast_ray(origin, dir, ray.max, filter) else {
            return RAY_MISS;
        };
        let (entity, collider) = match contact.target {
            RayTarget::Actor(a) => (self.owners[a.0 as usize], 0),
            RayTarget::Collider(c) => (self.switched[c.0 as usize], self.names[c.0 as usize]),
        };
        RayHit {
            distance: contact.distance,
            point: contact.point.into(),
            normal: contact.normal.into(),
            entity,
            collider,
        }
        .write(out);
        RAY_HIT
    }
}
