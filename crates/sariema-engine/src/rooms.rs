// SPDX-License-Identifier: Apache-2.0

//! The rooms of a game pack on the engine's side: a collision world for
//! each, built when the room is wanted and kept for the rest of the run, so
//! a room left and entered again collides as the game left it. A world is
//! built off the step by whoever takes its [`WorldJob`]: a thread builds it
//! whole, a host without threads a share of it each frame (the colliders,
//! the tree over them, then the walkable grids).
//! A switch to a room whose world is not in yet builds the world in the step
//! and leaves its grids to [`crate::Engine::grow_grids`], or to the first
//! path search that needs one. Either way the world is the same, so the run
//! is too.

use std::sync::Arc;

use sariema_format::collision::CollisionPack;
use sariema_physics::{ActorCollider, GridBuild};
use sariema_runtime::scenery::RoomRequest;
use sariema_runtime::state::State;

use crate::assets::RoomAssets;
use crate::physics::{Physics, PhysicsBuild};
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
    building: Option<Box<PhysicsBuild>>,
    physics: Option<Box<Physics>>,
    /// Colliders whose walkable grid is still to start, the next last.
    grids: Vec<ActorCollider>,
    grid: Option<GridBuild>,
}

impl WorldJob {
    fn new(room: usize, pack: Arc<CollisionPack>, rooms: &Rooms) -> WorldJob {
        WorldJob {
            room,
            pack,
            settings: rooms.settings.clone(),
            turn_rate: rooms.turn_rate,
            building: None,
            physics: None,
            grids: Vec::new(),
            grid: None,
        }
    }

    pub fn build(mut self) -> Result<BuiltWorld, String> {
        loop {
            if let Some(built) = self.advance(usize::MAX, usize::MAX)? {
                return Ok(built);
            }
        }
    }

    /// Builds a share of the world: about `units` of the world itself
    /// ([`sariema_physics::WorldBuild::advance`]) a call, then its walkable
    /// grids, `columns` columns at most a call. Gives the world once all of
    /// it is built, after which the job is spent.
    pub fn advance(&mut self, columns: usize, units: usize) -> Result<Option<BuiltWorld>, String> {
        let Some(physics) = &mut self.physics else {
            let building = self.building.get_or_insert_with(|| {
                Box::new(Physics::start(
                    self.pack.clone(),
                    &self.settings,
                    self.turn_rate,
                ))
            });
            if let Some(physics) = building.advance(units)? {
                self.building = None;
                self.grids = physics.colliders().iter().rev().copied().collect();
                self.physics = Some(Box::new(physics));
            }
            return Ok(None);
        };
        if self.grid.is_none() {
            let grids = &mut self.grids;
            self.grid =
                std::iter::from_fn(|| grids.pop()).find_map(|c| physics.world.start_grid(c));
        }
        match &mut self.grid {
            Some(grid) => {
                if physics.world.grow_grid(grid, columns)
                    && let Some(grid) = self.grid.take()
                {
                    physics.world.finish_grid(grid);
                }
                Ok(None)
            }
            None => Ok(self.physics.take().map(|physics| BuiltWorld {
                room: self.room,
                physics,
            })),
        }
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
    /// A grid of a built world being built, and the world's room.
    growing: Option<(usize, GridBuild)>,
}

impl Rooms {
    pub fn new(
        assets: Vec<RoomAssets>,
        settings: &EngineSettings,
        turn_rate: f32,
    ) -> Result<Rooms, String> {
        let worlds: Vec<World> = (assets.iter())
            .map(|r| match &r.collision {
                Some(pack) => World::Waiting(pack.clone()),
                None => World::Bare,
            })
            .collect();
        let mut rooms = Rooms {
            out: vec![false; assets.len()],
            assets,
            worlds,
            active: 0,
            settings: settings.clone(),
            turn_rate,
            growing: None,
        };
        if let Some(World::Waiting(pack)) = rooms.worlds.first() {
            let built = WorldJob::new(0, pack.clone(), &rooms).build()?;
            rooms.install(built);
        }
        Ok(rooms)
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

    /// Builds up to `columns` columns of a grid a built world lacks, the
    /// active one's first; whether any is left.
    pub fn grow_grids(&mut self, columns: usize) -> bool {
        if self.growing.is_none() {
            let worlds = &self.worlds;
            let mut order = std::iter::once(self.active).chain(0..worlds.len());
            self.growing = order.find_map(|r| match &worlds[r] {
                World::Built(p) => (p.colliders().iter())
                    .find_map(|&c| p.world.start_grid(c))
                    .map(|grid| (r, grid)),
                _ => None,
            });
        }
        let Some((room, grid)) = &mut self.growing else {
            return false;
        };
        let World::Built(physics) = &mut self.worlds[*room] else {
            unreachable!("only a built world grows a grid");
        };
        if physics.world.grow_grid(grid, columns)
            && let Some((_, grid)) = self.growing.take()
        {
            physics.world.finish_grid(grid);
        }
        true
    }

    /// Jobs for the worlds of `rooms` not built and not handed out yet.
    pub fn jobs(&mut self, rooms: &[usize]) -> Vec<WorldJob> {
        let mut jobs = Vec::new();
        for &room in rooms {
            if let (World::Waiting(pack), false) = (&self.worlds[room], self.out[room]) {
                self.out[room] = true;
                jobs.push(WorldJob::new(room, pack.clone(), self));
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use sariema_import::{RoomSource, build_collision};

    use super::*;

    #[test]
    fn a_world_is_built_with_the_walkable_grid_of_each_body() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/collision");
        let text = std::fs::read_to_string(root.join("rooms/sill.json")).unwrap();
        let source = RoomSource::from_json(&text).unwrap();
        let pack = build_collision(&source, &root).unwrap().unwrap();
        let rooms = Rooms::new(Vec::new(), &EngineSettings::default(), 360.0).unwrap();
        let built = WorldJob::new(0, Arc::new(pack), &rooms).build().unwrap();
        assert_eq!(
            built.physics.world.grids(),
            built.physics.colliders().len(),
            "an actor entering the world, at a switch, finds its grid built"
        );
    }
}
