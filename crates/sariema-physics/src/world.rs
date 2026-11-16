// SPDX-License-Identifier: Apache-2.0

//! The simulation: colliders, triggers and actors, advanced one fixed step at
//! a time. Everything is processed in id order and nothing reads a clock or
//! iterates a hash map, so the same intents give the same states, bit for bit.

use std::fmt;

use glam::{Vec2, Vec3};

use crate::actor::{ActorCollider, ActorDesc};
use crate::controller::{Derived, Ground, Mover};
use crate::intent::{ActorId, BlockReason, ColliderId, Event, EventKind, Intent, TriggerId};
use crate::math::{Bounds, flat, wrap_angle, yaw_matrix, yaw_of};
use crate::narrow::Tolerance;
use crate::nav::{NavCtx, NavGrid};
use crate::scene::{Filter, NewCollider, Pose, Scene};
use crate::settings::Settings;
use crate::shape::{Shape, ShapeError};
use crate::trigger::Trigger;

/// A collider to build the world with. A platform's shape is in its own
/// frame and starts at `platform`'s pose; the others are static.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColliderDesc<'a> {
    pub name: &'a str,
    pub shape: Shape<'a>,
    pub enabled: bool,
    pub platform: Option<Pose>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BuildError {
    Collider {
        name: String,
        error: ShapeError,
    },
    Trigger {
        name: String,
        error: ShapeError,
    },
    /// A radius or height that is not positive, or a body shorter than its width.
    Actor,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::Collider { name, error } => write!(f, "collider {name}: {error}"),
            BuildError::Trigger { name, error } => write!(f, "trigger {name}: {error}"),
            BuildError::Actor => write!(f, "the actor's collider has a size that is not positive"),
        }
    }
}

impl std::error::Error for BuildError {}

/// An actor as the last step left it. `previous_*` is the step before, for
/// drawing between steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorState {
    pub position: Vec3,
    pub previous_position: Vec3,
    pub yaw: f32,
    pub previous_yaw: f32,
    pub grounded: bool,
    pub ground: Option<ColliderId>,
    /// Whether a scripted move is under way.
    pub moving_to: bool,
}

#[derive(Clone, Copy, Debug)]
struct Route {
    face: Option<f32>,
    next: usize,
    reached: bool,
    stuck: u32,
}

#[derive(Clone, Copy, Debug)]
struct Knockback {
    push: Vec3,
    steps: u32,
    done: u32,
}

#[derive(Clone, Debug)]
struct Actor {
    desc: ActorDesc,
    position: Vec3,
    previous_position: Vec3,
    yaw: f32,
    previous_yaw: f32,
    target_yaw: f32,
    velocity: Vec3,
    vertical: f32,
    ground: Option<Ground>,
    pressing_wall: bool,
    stick: Vec2,
    speed_scale: f32,
    knockback: Option<Knockback>,
    teleport: Option<(Vec3, f32)>,
    pose: Option<(Vec3, f32)>,
    posed: bool,
    move_to: Option<(Vec3, Option<f32>)>,
    /// `move_to` was given after the pending teleport, so it outlives it.
    move_after_teleport: bool,
    route: Option<Route>,
    path: Vec<Vec3>,
    nav: Option<usize>,
    awake: bool,
    triggers: Vec<u32>,
    /// Off: the actor is left out of every step, as if removed.
    active: bool,
}

/// A walkable grid being built a share at a time, by
/// [`World::start_grid`].
pub struct GridBuild {
    grid: NavGrid,
    layers: Vec<f32>,
    /// The world's collider changes when it started.
    changes: u64,
}

pub struct World {
    settings: Settings,
    derived: Derived,
    scene: Scene,
    triggers: Vec<Trigger>,
    actors: Vec<Actor>,
    navs: Vec<NavGrid>,
    nav_bounds: Option<Bounds>,
    /// Colliders switched on or off since the world was built.
    collider_changes: u64,
    steps: u64,
    geometry_changed: bool,
    /// Colliders enabled since the last step, whose actors are pushed out next step.
    enabled_now: Vec<u32>,
    order: Vec<u32>,
    pushes: Vec<Vec3>,
    inside: Vec<u32>,
}

impl Actor {
    /// Ends a pose: the collider is on again from the next step.
    fn release(&mut self) {
        if self.posed {
            self.posed = false;
            self.pose = None;
            self.awake = true;
        }
    }
}

pub(crate) fn tolerance(settings: &Settings) -> Tolerance {
    Tolerance {
        contact: settings.collision.contact_tolerance,
        sweep: settings.collision.sweep_tolerance,
        iterations: settings.collision.sweep_iterations,
        tangent: settings.collision.tangent_tolerance,
        flat: libm::cosf(settings.collision.flat_edge_deg.to_radians()),
    }
}

impl World {
    pub fn new(settings: Settings, colliders: &[ColliderDesc]) -> Result<World, BuildError> {
        let tol = tolerance(&settings);
        let new: Vec<NewCollider> = colliders
            .iter()
            .map(|c| NewCollider {
                name: c.name,
                shape: c.shape,
                enabled: c.enabled,
                platform: c.platform,
            })
            .collect();
        let scene = Scene::new(&new, tol).map_err(|(i, error)| BuildError::Collider {
            name: colliders[i].name.to_owned(),
            error,
        })?;
        Ok(World::with_scene(settings, scene))
    }

    pub(crate) fn with_scene(settings: Settings, scene: Scene) -> World {
        World {
            derived: Derived::new(&settings),
            settings,
            scene,
            triggers: Vec::new(),
            actors: Vec::new(),
            navs: Vec::new(),
            nav_bounds: None,
            collider_changes: 0,
            steps: 0,
            geometry_changed: false,
            enabled_now: Vec::new(),
            order: Vec::new(),
            pushes: Vec::new(),
            inside: Vec::new(),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Limits the walkable grids built from now on to `bounds` across, for
    /// a room whose geometry reaches far around where actors walk; `None`
    /// covers the static colliders' bounds.
    pub fn set_nav_bounds(&mut self, bounds: Option<Bounds>) {
        self.nav_bounds = bounds;
    }

    pub(crate) fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Steps run so far.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    pub fn collider_count(&self) -> usize {
        self.scene.colliders.len()
    }

    pub fn collider_name(&self, id: ColliderId) -> &str {
        &self.scene.colliders[id.0 as usize].name
    }

    pub fn collider_by_name(&self, name: &str) -> Option<ColliderId> {
        self.scene
            .colliders
            .iter()
            .position(|c| c.name == name)
            .map(|i| ColliderId(i as u32))
    }

    pub fn collider_enabled(&self, id: ColliderId) -> bool {
        self.scene.colliders[id.0 as usize].enabled
    }

    /// The box around every static collider, enabled or not.
    pub fn static_bounds(&self) -> Bounds {
        self.scene.static_bounds()
    }

    pub fn add_trigger(&mut self, name: &str, shape: Shape) -> Result<TriggerId, BuildError> {
        let trigger = Trigger::new(name, &shape).map_err(|error| BuildError::Trigger {
            name: name.to_owned(),
            error,
        })?;
        self.triggers.push(trigger);
        Ok(TriggerId(self.triggers.len() as u32 - 1))
    }

    pub fn trigger_name(&self, id: TriggerId) -> &str {
        &self.triggers[id.0 as usize].name
    }

    pub fn add_actor(&mut self, desc: ActorDesc) -> Result<ActorId, BuildError> {
        if !desc.collider.is_valid() {
            return Err(BuildError::Actor);
        }
        let mut actor = Actor {
            desc,
            position: desc.position,
            previous_position: desc.position,
            yaw: desc.yaw,
            previous_yaw: desc.yaw,
            target_yaw: desc.yaw,
            velocity: Vec3::ZERO,
            vertical: 0.0,
            ground: None,
            pressing_wall: false,
            stick: Vec2::ZERO,
            speed_scale: 1.0,
            knockback: None,
            teleport: None,
            pose: None,
            posed: false,
            move_to: None,
            move_after_teleport: false,
            route: None,
            path: Vec::new(),
            nav: None,
            awake: true,
            triggers: Vec::new(),
            active: true,
        };
        actor.ground = self.standing_on(&actor, desc.position, desc.yaw);
        self.actors.push(actor);
        self.pushes.push(Vec3::ZERO);
        self.order.push(self.actors.len() as u32 - 1);
        Ok(ActorId(self.actors.len() as u32 - 1))
    }

    /// Builds the walkable grid `actor` searches paths on, shared by every
    /// actor with the same collider. The first scripted move or path search
    /// builds it otherwise, so call this while loading to keep that cost out
    /// of play.
    pub fn prepare_paths(&mut self, actor: ActorId) -> usize {
        let i = actor.0 as usize;
        if let Some(nav) = self.actors[i].nav {
            return nav;
        }
        let nav = self.prepare_grid(self.actors[i].desc.collider);
        self.actors[i].nav = Some(nav);
        nav
    }

    /// Builds the walkable grid of actors with `collider` before any is
    /// added, so the first one added, mid-play, finds it built.
    pub fn prepare_grid(&mut self, collider: ActorCollider) -> usize {
        match self.navs.iter().position(|n| n.collider == collider) {
            Some(n) => n,
            None => self.build_nav(collider),
        }
    }

    /// Starts building the walkable grid of actors with `collider` a share
    /// at a time ([`World::grow_grid`], then [`World::finish_grid`]), or
    /// `None` when it is built.
    pub fn start_grid(&self, collider: ActorCollider) -> Option<GridBuild> {
        if self.navs.iter().any(|n| n.collider == collider) {
            return None;
        }
        Some(GridBuild {
            grid: NavGrid::start(&self.nav_ctx(), collider),
            layers: Vec::new(),
            changes: self.collider_changes,
        })
    }

    /// Builds up to `columns` more columns of `build`, then labels the
    /// islands of as many; true once all are.
    pub fn grow_grid(&self, build: &mut GridBuild, columns: usize) -> bool {
        build.grid.grow(&self.nav_ctx(), &mut build.layers, columns)
    }

    /// Takes a grid [`World::grow_grid`] built whole, as
    /// [`World::prepare_grid`] would have built it. A collider switched
    /// while it was built has it built again here, whole.
    pub fn finish_grid(&mut self, build: GridBuild) -> usize {
        let collider = build.grid.collider;
        if let Some(n) = self.navs.iter().position(|n| n.collider == collider) {
            return n;
        }
        if build.changes != self.collider_changes {
            return self.build_nav(collider);
        }
        let mut grid = build.grid;
        grid.finish();
        self.navs.push(grid);
        self.navs.len() - 1
    }

    /// Walkable grids built so far, one per collider.
    pub fn grids(&self) -> usize {
        self.navs.len()
    }

    /// The columns each walkable grid has to build.
    pub fn grid_columns(&self) -> usize {
        let (_, nx, nz) = NavGrid::extent(&self.nav_ctx());
        nx * nz
    }

    /// Whether a scripted move of `actor` to `target` would find a path
    /// from where it stands, as [`Intent::MoveTo`] would search it.
    pub fn reachable(&mut self, actor: ActorId, target: Vec3) -> bool {
        let nav = self.prepare_paths(actor);
        let ctx = NavCtx {
            scene: &self.scene,
            settings: &self.settings,
            derived: &self.derived,
            area: self.nav_bounds,
        };
        let from = self.actors[actor.0 as usize].position;
        self.navs[nav].find(&ctx, from, target, &mut Vec::new())
    }

    fn build_nav(&mut self, collider: ActorCollider) -> usize {
        let grid = NavGrid::build(&self.nav_ctx(), collider);
        self.navs.push(grid);
        self.navs.len() - 1
    }

    fn nav_ctx(&self) -> NavCtx<'_> {
        NavCtx {
            scene: &self.scene,
            settings: &self.settings,
            derived: &self.derived,
            area: self.nav_bounds,
        }
    }

    pub fn actor(&self, id: ActorId) -> ActorState {
        let a = &self.actors[id.0 as usize];
        ActorState {
            position: a.position,
            previous_position: a.previous_position,
            yaw: a.yaw,
            previous_yaw: a.previous_yaw,
            grounded: a.ground.is_some(),
            ground: a.ground.map(|g| ColliderId(g.collider)),
            moving_to: a.route.is_some() || a.move_to.is_some(),
        }
    }

    /// The actor's horizontal velocity and vertical speed, m/s: what its
    /// next step accelerates from.
    pub fn actor_velocity(&self, id: ActorId) -> (Vec3, f32) {
        let a = &self.actors[id.0 as usize];
        (a.velocity, a.vertical)
    }

    /// Sets what [`World::actor_velocity`] gives, for an actor carried over
    /// from another world without losing its speed.
    pub fn set_actor_velocity(&mut self, id: ActorId, velocity: Vec3, vertical: f32) {
        let a = &mut self.actors[id.0 as usize];
        a.velocity = velocity;
        a.vertical = vertical;
        a.awake = true;
    }

    pub fn actor_collider(&self, id: ActorId) -> ActorCollider {
        self.actors[id.0 as usize].desc.collider
    }

    /// Leaves an actor out of every step (motion, pushing, triggers, which
    /// it leaves) or brings it back; its id stays valid. A caller whose
    /// actors come and go reuses ids this way instead of adding more.
    pub fn set_actor_active(&mut self, id: ActorId, active: bool, events: &mut Vec<Event>) {
        let a = &mut self.actors[id.0 as usize];
        if a.active == active {
            return;
        }
        a.active = active;
        a.awake = true;
        if !active {
            for &t in &a.triggers {
                events.push(Event {
                    actor: id,
                    kind: EventKind::TriggerExit(TriggerId(t)),
                });
            }
            a.triggers.clear();
            a.route = None;
            a.move_to = None;
            a.stick = Vec2::ZERO;
            a.knockback = None;
        }
    }

    pub fn actor_active(&self, id: ActorId) -> bool {
        self.actors[id.0 as usize].active
    }

    pub fn actor_count(&self) -> usize {
        self.actors.len()
    }

    /// The waypoints of the actor's current scripted move.
    pub fn actor_path(&self, id: ActorId) -> &[Vec3] {
        &self.actors[id.0 as usize].path
    }

    /// Moves a platform; actors standing on it are carried in the next step.
    pub fn set_platform_pose(&mut self, collider: ColliderId, pose: Pose) {
        self.scene.set_pose(collider.0, pose);
    }

    /// Moves a platform to `pose`, through the poses between (the yaw
    /// turned the way it differs, not the shorter way), unless that would
    /// bring it within the collision skin of an actor: then it stays where
    /// it was and the first actor in the way is given. A posed or inactive
    /// actor is in no one's way. Unlike [`World::set_platform_pose`], it
    /// never pushes an actor.
    pub fn try_platform_pose(&mut self, collider: ColliderId, pose: Pose) -> Result<(), ActorId> {
        let Some((from, moved_from)) = self.scene.kinematic(collider.0) else {
            return Ok(());
        };
        let thinnest = (self.actors.iter())
            .filter(|a| a.active && !a.posed)
            .map(|a| a.desc.collider.radius)
            .fold(f32::INFINITY, f32::min);
        let turn = pose.yaw - from.yaw;
        let travel = self.scene.platform_reach(collider.0) * turn.abs()
            + (pose.translation - from.translation).length();
        let samples = if thinnest.is_finite() {
            (travel / thinnest).ceil().max(1.0) as u32
        } else {
            1
        };
        let only = Filter {
            only: Some(collider.0),
            ..Filter::ALL
        };
        let skin = self.settings.collision.skin;
        for k in 1..=samples {
            let t = k as f32 / samples as f32;
            let at = if k == samples {
                pose
            } else {
                Pose {
                    translation: from.translation.lerp(pose.translation, t),
                    yaw: from.yaw + turn * t,
                }
            };
            self.scene.set_pose(collider.0, at);
            for (i, a) in self.actors.iter().enumerate() {
                if a.posed || !a.active {
                    continue;
                }
                let probe = a.desc.collider.probe(a.position, a.yaw, 0.0);
                let mut near = false;
                self.scene.contacts(&probe, skin, only, |_, _| near = true);
                if near {
                    self.scene.put_back(collider.0, from, moved_from);
                    return Err(ActorId(i as u32));
                }
            }
        }
        Ok(())
    }

    pub fn platform_pose(&self, collider: ColliderId) -> Option<Pose> {
        self.scene.colliders[collider.0 as usize]
            .kinematic
            .as_ref()
            .map(|k| k.pose)
    }

    pub fn apply(&mut self, intent: Intent) {
        match intent {
            Intent::Move { actor, stick } => {
                let a = &mut self.actors[actor.0 as usize];
                a.stick = stick;
                a.release();
            }
            Intent::MoveTo {
                actor,
                target,
                face,
            } => {
                let a = &mut self.actors[actor.0 as usize];
                a.release();
                a.move_to = Some((target, face));
                a.move_after_teleport = a.teleport.is_some();
                a.route = None;
            }
            Intent::Stop { actor } => {
                let a = &mut self.actors[actor.0 as usize];
                a.release();
                a.stick = Vec2::ZERO;
                a.move_to = None;
                a.route = None;
            }
            Intent::Knockback { actor, push, steps } => {
                let a = &mut self.actors[actor.0 as usize];
                a.release();
                a.knockback = (steps > 0).then_some(Knockback {
                    push: flat(push),
                    steps,
                    done: 0,
                });
            }
            Intent::Teleport {
                actor,
                position,
                yaw,
            } => {
                let a = &mut self.actors[actor.0 as usize];
                a.release();
                a.teleport = Some((position, yaw));
                a.move_after_teleport = false;
            }
            Intent::Pose {
                actor,
                position,
                yaw,
            } => {
                let a = &mut self.actors[actor.0 as usize];
                a.pose = Some((position, yaw));
                a.posed = true;
            }
            Intent::SetColliderEnabled { collider, enabled } => {
                if self.set_enabled(collider.0, enabled) && enabled {
                    self.enabled_now.push(collider.0);
                }
            }
            Intent::SetSpeedScale { actor, scale } => {
                self.actors[actor.0 as usize].speed_scale = scale
            }
            Intent::Face { actor, yaw } => {
                let a = &mut self.actors[actor.0 as usize];
                a.target_yaw = wrap_angle(yaw);
                a.awake = true;
            }
        }
    }

    /// Whether the collider's state changed.
    fn set_enabled(&mut self, collider: u32, enabled: bool) -> bool {
        let c = &mut self.scene.colliders[collider as usize];
        if c.enabled == enabled {
            return false;
        }
        c.enabled = enabled;
        let bounds = c.bounds;
        self.geometry_changed = true;
        self.collider_changes += 1;
        for nav in &mut self.navs {
            nav.mark_dirty(bounds);
        }
        true
    }

    /// Pushes actors out of the colliders enabled since the last step, in the
    /// order they were enabled; a collider that cannot push one out is
    /// disabled again.
    fn settle_enabled(&mut self, events: &mut Vec<Event>) {
        for k in 0..self.enabled_now.len() {
            let collider = self.enabled_now[k];
            for i in 0..self.actors.len() {
                if !self.scene.colliders[collider as usize].enabled {
                    break;
                }
                let a = &self.actors[i];
                if a.posed || !a.active {
                    continue;
                }
                let only = Filter {
                    only: Some(collider),
                    ..Filter::ALL
                };
                let out = self.mover(a, Filter::ALL).push_out(a.position, only, false);
                let id = ActorId(i as u32);
                match out {
                    Some(at) if at == a.position => {}
                    Some(at) => {
                        let ground = self.standing_on(a, at, a.yaw);
                        let a = &mut self.actors[i];
                        a.position = at;
                        a.ground = ground;
                        a.awake = true;
                        events.push(Event {
                            actor: id,
                            kind: EventKind::PushedOut {
                                collider: ColliderId(collider),
                            },
                        });
                    }
                    None => {
                        self.set_enabled(collider, false);
                        events.push(Event {
                            actor: id,
                            kind: EventKind::EnableRefused {
                                collider: ColliderId(collider),
                            },
                        });
                    }
                }
            }
        }
        self.enabled_now.clear();
    }

    /// Runs one fixed step and appends what happened to `events`.
    pub fn step(&mut self, events: &mut Vec<Event>) {
        self.steps += 1;
        self.settle_enabled(events);
        for i in 0..self.actors.len() {
            if self.actors[i].active {
                self.step_actor(i, events);
            }
        }
        self.separate();
        for i in 0..self.actors.len() {
            if self.actors[i].active {
                self.update_triggers(i, events);
            }
        }
        self.scene.settle_platforms();
        self.geometry_changed = false;
    }

    /// The ground an actor placed at `at` already touches, within a skin, so
    /// that placing it on a floor is not a landing.
    fn standing_on(&self, a: &Actor, at: Vec3, yaw: f32) -> Option<Ground> {
        let mover = Mover {
            yaw,
            ..self.mover(a, Filter::ALL)
        };
        let (rest, ground) = mover.snap(at)?;
        (at.y - rest.y <= self.settings.collision.skin).then_some(ground)
    }

    fn mover<'a>(&'a self, a: &'a Actor, filter: Filter) -> Mover<'a> {
        Mover {
            scene: &self.scene,
            settings: &self.settings,
            derived: &self.derived,
            collider: &a.desc.collider,
            yaw: a.yaw,
            filter,
        }
    }

    fn step_actor(&mut self, i: usize, events: &mut Vec<Event>) {
        let id = ActorId(i as u32);
        let dt = self.settings.step_seconds;
        {
            let a = &mut self.actors[i];
            a.previous_position = a.position;
            a.previous_yaw = a.yaw;
        }
        if self.actors[i].posed {
            if let Some((position, yaw)) = self.actors[i].pose.take() {
                let ground = self.standing_on(&self.actors[i], position, yaw);
                let a = &mut self.actors[i];
                a.position = position;
                a.yaw = yaw;
                a.target_yaw = yaw;
                a.velocity = Vec3::ZERO;
                a.vertical = 0.0;
                a.ground = ground;
                a.route = None;
                a.move_to = None;
                a.knockback = None;
                a.pressing_wall = false;
            }
            return;
        }
        self.carry(i);
        if let Some((position, yaw)) = self.actors[i].teleport.take() {
            let a = &self.actors[i];
            let resolved = Mover {
                yaw,
                ..self.mover(a, Filter::ALL)
            }
            .depenetrate(position)
            .map(|p| (p, self.standing_on(a, p, yaw)));
            let a = &mut self.actors[i];
            match resolved {
                Some((p, ground)) => {
                    a.position = p;
                    a.previous_position = p;
                    a.yaw = yaw;
                    a.previous_yaw = yaw;
                    a.target_yaw = yaw;
                    a.velocity = Vec3::ZERO;
                    a.vertical = 0.0;
                    a.ground = ground;
                    a.route = None;
                    if !a.move_after_teleport {
                        a.move_to = None;
                    }
                    a.move_after_teleport = false;
                    a.awake = true;
                }
                None => {
                    self.actors[i].move_after_teleport = false;
                    events.push(Event {
                        actor: id,
                        kind: EventKind::TeleportRejected,
                    });
                }
            }
        }
        if let Some((target, face)) = self.actors[i].move_to.take() {
            self.start_route(i, target, face, events);
        }

        let desired = self.desired_velocity(i);
        let a = &self.actors[i];
        let c = &self.settings.controller;
        let velocity = match (a.knockback.is_some(), c.accel_steps) {
            (true, _) | (_, 0) => desired,
            (false, steps) => {
                let max = a.desc.walk_speed * a.speed_scale.max(0.0) / steps as f32;
                let change = desired - a.velocity;
                let len = change.length();
                if len > max {
                    a.velocity + change * (max / len)
                } else {
                    desired
                }
            }
        };
        let mut h = velocity * dt;
        if let Some(k) = a.knockback {
            let n = k.steps as f32;
            h += k.push * (2.0 * (n - k.done as f32) / (n * (n + 1.0)));
        }
        let grounded = a.ground.is_some();
        let vertical = if grounded {
            0.0
        } else {
            (a.vertical - c.gravity * dt).max(-c.max_fall_speed)
        };
        let idle = grounded && h.length_squared() == 0.0 && !a.awake && !self.geometry_changed;

        let (moved, snapped) = if idle {
            (None, None)
        } else {
            let mover = self.mover(a, Filter::ALL);
            let moved = mover.walk(
                a.position,
                h,
                vertical * dt,
                a.ground,
                a.knockback.is_none(),
            );
            let snapped = if grounded || moved.ground.is_some() {
                mover.snap(moved.position)
            } else {
                None
            };
            (Some(moved), snapped)
        };

        let a = &mut self.actors[i];
        a.velocity = velocity;
        if let Some(k) = &mut a.knockback {
            k.done += 1;
            if k.done >= k.steps {
                a.knockback = None;
            }
        }
        a.awake = false;
        if let Some(moved) = moved {
            let was_falling = !grounded;
            a.position = moved.position;
            a.ground = moved.ground;
            if let Some((rest, ground)) = snapped {
                a.position = rest;
                a.ground = Some(ground);
            }
            if a.ground.is_some() {
                if was_falling {
                    events.push(Event {
                        actor: id,
                        kind: EventKind::Landed { speed: -vertical },
                    });
                }
                a.vertical = 0.0;
            } else {
                a.vertical = if moved.ceiling {
                    vertical.min(0.0)
                } else {
                    vertical
                };
            }
            match moved.wall {
                Some((collider, normal)) if !a.pressing_wall => {
                    a.pressing_wall = true;
                    events.push(Event {
                        actor: id,
                        kind: EventKind::HitWall {
                            collider: ColliderId(collider),
                            normal,
                        },
                    });
                }
                Some(_) => {}
                None => a.pressing_wall = false,
            }
        }
        self.turn(i);
        self.follow_route(i, events);
    }

    /// Moves an actor standing on a platform that moved with it, and pushes
    /// one the platform moved into.
    fn carry(&mut self, i: usize) {
        let a = &self.actors[i];
        let rides = a.ground.map(|g| g.collider);
        for &p in self.scene.platforms() {
            let c = &self.scene.colliders[p as usize];
            let Some(k) = &c.kinematic else { continue };
            let Some(from) = k.moved_from else { continue };
            if rides == Some(p) {
                let local = yaw_matrix(from.yaw).transpose() * (a.position - from.translation);
                let target = k.rotation * local + k.pose.translation;
                let turn = wrap_angle(k.pose.yaw - from.yaw);
                let mover = self.mover(a, Filter::except(Some(p)));
                let at = mover.push(a.position, target - a.position);
                let a = &mut self.actors[i];
                a.position = at;
                a.yaw = wrap_angle(a.yaw + turn);
                a.target_yaw = wrap_angle(a.target_yaw + turn);
                a.awake = true;
                return;
            }
            let only = Filter {
                only: Some(p),
                ..Filter::ALL
            };
            let mover = self.mover(a, only);
            let probe = a.desc.collider.probe(a.position, a.yaw, 0.0);
            if let Some((_, deepest)) = self.scene.deepest(&probe, only) {
                let out = deepest.normal * (self.settings.collision.skin - deepest.separation);
                let mover = Mover {
                    filter: Filter::except(Some(p)),
                    ..mover
                };
                let at = mover.push(a.position, out);
                let a = &mut self.actors[i];
                a.position = at;
                a.awake = true;
                return;
            }
        }
    }

    fn desired_velocity(&self, i: usize) -> Vec3 {
        let a = &self.actors[i];
        let speed = a.desc.walk_speed * a.speed_scale.max(0.0);
        if a.knockback.is_some() {
            return Vec3::ZERO;
        }
        if let Some(route) = a.route {
            if route.reached {
                return Vec3::ZERO;
            }
            let to = flat(a.path[route.next] - a.position);
            let d = to.length();
            if d <= 1e-6 {
                return Vec3::ZERO;
            }
            let last = route.next + 1 == a.path.len();
            let travel = if last {
                (speed * self.settings.step_seconds).min(d)
            } else {
                speed * self.settings.step_seconds
            };
            return to * (travel / (d * self.settings.step_seconds));
        }
        let c = &self.settings.controller;
        let m = a.stick.length();
        if m <= c.stick_dead_zone {
            return Vec3::ZERO;
        }
        let t = ((m - c.stick_dead_zone) / (c.stick_full - c.stick_dead_zone).max(1e-6)).min(1.0);
        let curve = if c.stick_exponent == 1.0 {
            t
        } else {
            libm::powf(t, c.stick_exponent)
        };
        Vec3::new(a.stick.x, 0.0, a.stick.y) * (speed * curve / m)
    }

    fn turn(&mut self, i: usize) {
        let filter = self.settings.controller.yaw_filter;
        let blend = self.derived.yaw_blend;
        let a = &mut self.actors[i];
        let facing = match a.route {
            Some(Route {
                reached: true,
                face,
                ..
            }) => face,
            _ => None,
        };
        if let Some(face) = facing {
            a.target_yaw = face;
        } else if a.velocity.length_squared() > 0.0 {
            let want = yaw_of(a.velocity.x, a.velocity.z);
            if wrap_angle(want - a.target_yaw).abs() > filter {
                a.target_yaw = want;
            }
        }
        let max = self.derived.max_turn;
        let yaw = wrap_angle(a.yaw + (wrap_angle(a.target_yaw - a.yaw) * blend).clamp(-max, max));
        let swings = a.desc.collider.head.is_some_and(|h| h.forward != 0.0);
        if !swings || yaw == a.yaw {
            a.yaw = yaw;
            return;
        }
        let a = &self.actors[i];
        let turned = Mover {
            yaw,
            ..self.mover(a, Filter::ALL)
        }
        .push_out(a.position, Filter::ALL, true);
        let a = &mut self.actors[i];
        if let Some(at) = turned {
            a.awake |= at != a.position;
            a.position = at;
            a.yaw = yaw;
        }
    }

    fn start_route(&mut self, i: usize, target: Vec3, face: Option<f32>, events: &mut Vec<Event>) {
        let nav = self.prepare_paths(ActorId(i as u32));
        let ctx = NavCtx {
            scene: &self.scene,
            settings: &self.settings,
            derived: &self.derived,
            area: self.nav_bounds,
        };
        let a = &mut self.actors[i];
        let found = self.navs[nav].find(&ctx, a.position, target, &mut a.path);
        if found {
            a.route = Some(Route {
                face,
                next: 0,
                reached: false,
                stuck: 0,
            });
        } else {
            a.route = None;
            events.push(Event {
                actor: ActorId(i as u32),
                kind: EventKind::Blocked(BlockReason::NoPath),
            });
        }
    }

    fn follow_route(&mut self, i: usize, events: &mut Vec<Event>) {
        let m = &self.settings.move_to;
        let dt = self.settings.step_seconds;
        let a = &mut self.actors[i];
        let Some(mut route) = a.route else { return };
        let id = ActorId(i as u32);
        if !route.reached {
            let expected = a.desc.walk_speed * a.speed_scale.max(0.0) * dt;
            let before = flat(a.path[route.next] - a.previous_position).length();
            let after = flat(a.path[route.next] - a.position).length();
            let last = route.next + 1 == a.path.len();
            if last && after <= m.arrive_distance {
                route.reached = true;
            } else if !last && after <= m.waypoint_distance {
                route.next += 1;
                route.stuck = 0;
            } else if before - after < m.stuck_progress * expected.min(before) {
                route.stuck += 1;
                if route.stuck >= m.stuck_steps {
                    a.route = None;
                    events.push(Event {
                        actor: id,
                        kind: EventKind::Blocked(BlockReason::Stuck),
                    });
                    return;
                }
            } else {
                route.stuck = 0;
            }
        }
        if route.reached
            && route
                .face
                .is_none_or(|f| wrap_angle(f - a.yaw).abs() <= m.face_tolerance)
        {
            a.route = None;
            events.push(Event {
                actor: id,
                kind: EventKind::Arrived,
            });
            return;
        }
        a.route = Some(route);
    }

    /// Pushes apart actors whose body capsules overlap, by sweep and prune on x.
    fn separate(&mut self) {
        let actors = &self.actors;
        let min_x = |i: u32| {
            let a = &actors[i as usize];
            a.position.x - a.desc.collider.radius
        };
        for k in 1..self.order.len() {
            let mut j = k;
            while j > 0 {
                let (prev, cur) = (self.order[j - 1], self.order[j]);
                let before = min_x(prev).total_cmp(&min_x(cur)).then(prev.cmp(&cur));
                if before != std::cmp::Ordering::Greater {
                    break;
                }
                self.order.swap(j - 1, j);
                j -= 1;
            }
        }
        let mut any = false;
        for k in 0..self.order.len() {
            let ia = self.order[k] as usize;
            let a = &actors[ia];
            if a.posed || !a.active {
                continue;
            }
            let ra = a.desc.collider.radius;
            for &ib in &self.order[k + 1..] {
                let b = &actors[ib as usize];
                let rb = b.desc.collider.radius;
                if b.position.x - rb > a.position.x + ra {
                    break;
                }
                if b.posed || !b.active {
                    continue;
                }
                let top_a = a.position.y + a.desc.collider.height;
                let top_b = b.position.y + b.desc.collider.height;
                if a.position.y >= top_b || b.position.y >= top_a {
                    continue;
                }
                let d = flat(b.position - a.position);
                let dist = d.length();
                let overlap = ra + rb - dist;
                if overlap <= 0.0 {
                    continue;
                }
                let dir = if dist > 1e-6 {
                    d / dist
                } else if ia < ib as usize {
                    Vec3::X
                } else {
                    Vec3::NEG_X
                };
                let (ma, mb) = (a.desc.mass, b.desc.mass);
                let (wa, wb) = match (ma.is_infinite(), mb.is_infinite()) {
                    (true, true) => (0.0, 0.0),
                    (true, false) => (0.0, 1.0),
                    (false, true) => (1.0, 0.0),
                    (false, false) => (mb / (ma + mb), ma / (ma + mb)),
                };
                self.pushes[ia] -= dir * (overlap * wa);
                self.pushes[ib as usize] += dir * (overlap * wb);
                any = true;
            }
        }
        if !any {
            return;
        }
        for i in 0..self.actors.len() {
            let push = std::mem::take(&mut self.pushes[i]);
            if push == Vec3::ZERO {
                continue;
            }
            let at = self
                .mover(&self.actors[i], Filter::ALL)
                .push(self.actors[i].position, push);
            let a = &mut self.actors[i];
            a.position = at;
            a.awake = true;
        }
    }

    fn update_triggers(&mut self, i: usize, events: &mut Vec<Event>) {
        let a = &self.actors[i];
        let probe = a.desc.collider.probe(a.position, a.yaw, 0.0);
        self.inside.clear();
        for (t, trigger) in self.triggers.iter().enumerate() {
            if trigger.overlaps(&probe) {
                self.inside.push(t as u32);
            }
        }
        let a = &mut self.actors[i];
        if a.triggers == self.inside {
            return;
        }
        let id = ActorId(i as u32);
        for &t in &a.triggers {
            if self.inside.binary_search(&t).is_err() {
                events.push(Event {
                    actor: id,
                    kind: EventKind::TriggerExit(TriggerId(t)),
                });
            }
        }
        for &t in &self.inside {
            if a.triggers.binary_search(&t).is_err() {
                events.push(Event {
                    actor: id,
                    kind: EventKind::TriggerEnter(TriggerId(t)),
                });
            }
        }
        a.triggers.clear();
        a.triggers.extend_from_slice(&self.inside);
    }

    /// FNV-1a over every value the simulation carries from step to step.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.steps);
        for c in &self.scene.colliders {
            h.u32(c.enabled as u32);
            if let Some(k) = &c.kinematic {
                h.vec3(k.pose.translation);
                h.f32(k.pose.yaw);
            }
        }
        for a in &self.actors {
            h.vec3(a.position);
            h.f32(a.yaw);
            h.f32(a.target_yaw);
            h.vec3(a.velocity);
            h.f32(a.vertical);
            h.u32(a.ground.map_or(u32::MAX, |g| g.collider));
            h.u32(a.pressing_wall as u32);
            h.u32(a.route.map_or(u32::MAX, |r| r.next as u32));
            h.u32(a.route.map_or(0, |r| r.stuck));
            h.u32(a.knockback.map_or(0, |k| k.done));
            for &t in &a.triggers {
                h.u32(t);
            }
        }
        h.0
    }

    /// Finds a path for `actor`'s collider without moving it; `out` gets the waypoints.
    pub fn find_path(&mut self, actor: ActorId, from: Vec3, to: Vec3, out: &mut Vec<Vec3>) -> bool {
        let nav = self.prepare_paths(actor);
        let ctx = NavCtx {
            scene: &self.scene,
            settings: &self.settings,
            derived: &self.derived,
            area: self.nav_bounds,
        };
        self.navs[nav].find(&ctx, from, to, out)
    }

    /// Centre and feet height of every walkable cell of `actor`'s grid.
    pub fn walkable_cells(&mut self, actor: ActorId) -> impl Iterator<Item = Vec3> + '_ {
        let nav = self.prepare_paths(actor);
        self.navs[nav].cells()
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    fn u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn u64(&mut self, v: u64) {
        self.u32(v as u32);
        self.u32((v >> 32) as u32);
    }

    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
}

#[cfg(test)]
mod tests {
    use glam::{Mat3, Vec3};

    use super::*;

    /// Two 6 m floors 1 m apart: two islands.
    fn two_floors() -> World {
        let floor = |name, x: f32| ColliderDesc {
            name,
            shape: Shape::Box {
                center: Vec3::new(x, -0.1, 0.0),
                half_extents: Vec3::new(3.0, 0.1, 3.0),
                rotation: Mat3::IDENTITY,
            },
            enabled: true,
            platform: None,
        };
        World::new(
            Settings::default(),
            &[floor("west", -3.5), floor("east", 3.5)],
        )
        .unwrap()
    }

    #[test]
    fn a_grid_is_labelled_into_islands_in_its_shares_not_when_finished() {
        let whole = {
            let mut world = two_floors();
            let n = world.prepare_grid(ActorCollider::default());
            world.navs[n].islands()
        };
        let world = two_floors();
        let mut build = world.start_grid(ActorCollider::default()).unwrap();
        let columns = world.grid_columns();
        let mut shares = 0;
        while !world.grow_grid(&mut build, 97) {
            shares += 1;
        }
        assert!(
            shares > columns / 97,
            "{shares} shares of {columns} columns"
        );
        assert_eq!(build.grid.islands(), whole);
        assert_eq!(
            whole
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            2
        );
    }
}
