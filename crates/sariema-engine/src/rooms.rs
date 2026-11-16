// SPDX-License-Identifier: Apache-2.0

//! The rooms of a game pack on the engine's side: a collision world for
//! each, built when the room is wanted and kept for the rest of the run, so
//! a room left and entered again collides as the game left it. A world is
//! built off the step by whoever takes its [`WorldJob`] (a thread, or an
//! idle moment of the host's frame); a switch to a room whose world is not
//! in yet builds it in the step. Either way the world is the same, so the
//! run is too.

use std::sync::Arc;

use sariema_format::collision::CollisionPack;
use sariema_runtime::scenery::RoomRequest;
use sariema_runtime::state::State;

use crate::assets::RoomAssets;
use crate::physics::Physics;
use crate::settings::EngineSettings;

enum World {
    /// The room has no collision.
    Bare,
    /// Not built; its job may be out.
    Waiting(Arc<CollisionPack>),
    Built(Box<Physics>),
}

/// A room's collision world to build, anywhere: it needs nothing of the
/// engine but what it carries.
pub struct WorldJob {
    pub room: usize,
    pack: Arc<CollisionPack>,
    settings: EngineSettings,
    turn_rate: f32,
}

impl WorldJob {
    pub fn build(self) -> Result<BuiltWorld, String> {
        let physics = Physics::new(self.pack, &self.settings, self.turn_rate)?;
        Ok(BuiltWorld {
            room: self.room,
            physics: Box::new(physics),
        })
    }
}

/// A world a [`WorldJob`] built, for [`crate::Engine::install_world`].
pub struct BuiltWorld {
    pub room: usize,
    physics: Box<Physics>,
}

pub(crate) struct Rooms {
    pub assets: Vec<RoomAssets>,
    worlds: Vec<World>,
    /// By room, whether its job is out.
    out: Vec<bool>,
    active: usize,
    settings: EngineSettings,
    turn_rate: f32,
}

impl Rooms {
    pub fn new(
        assets: Vec<RoomAssets>,
        settings: &EngineSettings,
        turn_rate: f32,
    ) -> Result<Rooms, String> {
        let mut worlds: Vec<World> = (assets.iter())
            .map(|r| match &r.collision {
                Some(pack) => World::Waiting(pack.clone()),
                None => World::Bare,
            })
            .collect();
        if let Some(World::Waiting(pack)) = worlds.first() {
            worlds[0] = World::Built(Box::new(Physics::new(pack.clone(), settings, turn_rate)?));
        }
        Ok(Rooms {
            out: vec![false; assets.len()],
            assets,
            worlds,
            active: 0,
            settings: settings.clone(),
            turn_rate,
        })
    }

    pub fn active(&self) -> Option<&Physics> {
        match self.worlds.get(self.active)? {
            World::Built(p) => Some(p),
            _ => None,
        }
    }

    pub fn active_mut(&mut self) -> Option<&mut Physics> {
        match self.worlds.get_mut(self.active)? {
            World::Built(p) => Some(p),
            _ => None,
        }
    }

    /// Switches the collision to the room a `room_switch` of this step
    /// asked for, its actors handed over.
    pub fn follow(&mut self, s: &mut State) {
        let Some(to) = (s.scenery.requests.iter()).rev().find_map(|r| match *r {
            RoomRequest::Switch(to) => Some(to),
            RoomRequest::Preload(_) => None,
        }) else {
            return;
        };
        if to == self.active {
            return;
        }
        self.build_now(to);
        let (from, active) = (self.active, to);
        self.active = active;
        let (a, b) = if from < to {
            let (left, right) = self.worlds.split_at_mut(to);
            (&mut left[from], &mut right[0])
        } else {
            let (left, right) = self.worlds.split_at_mut(from);
            (&mut right[0], &mut left[to])
        };
        if let (World::Built(old), World::Built(new)) = (a, b) {
            old.hand_over(new, s);
        }
    }

    fn build_now(&mut self, room: usize) {
        if let World::Waiting(pack) = &self.worlds[room] {
            let physics = Physics::new(pack.clone(), &self.settings, self.turn_rate)
                .expect("a collision pack the engine loaded builds");
            self.worlds[room] = World::Built(Box::new(physics));
        }
    }

    /// Jobs for the worlds of `rooms` not built and not handed out yet.
    pub fn jobs(&mut self, rooms: &[usize]) -> Vec<WorldJob> {
        let mut jobs = Vec::new();
        for &room in rooms {
            if let (World::Waiting(pack), false) = (&self.worlds[room], self.out[room]) {
                self.out[room] = true;
                jobs.push(WorldJob {
                    room,
                    pack: pack.clone(),
                    settings: self.settings.clone(),
                    turn_rate: self.turn_rate,
                });
            }
        }
        jobs
    }

    /// Takes a built world unless the room already has one.
    pub fn install(&mut self, built: BuiltWorld) {
        if let World::Waiting(_) = self.worlds[built.room] {
            self.worlds[built.room] = World::Built(built.physics);
        }
    }

    pub fn built(&self, room: usize) -> bool {
        matches!(self.worlds.get(room), Some(World::Built(_)))
    }
}
