// SPDX-License-Identifier: Apache-2.0

//! The rooms of a game pack as the step sees them: which one is active,
//! how the last switch is shown, and what the game changed in each (groups,
//! piece states, scene parameters), kept for the whole run so a room left
//! and entered again is as the game left it. Room nodes and lights spawned
//! as entities are bound here to what they stand for. Renderers read this
//! state; nothing here needs the rooms' meshes or textures.

use glam::{EulerRot, Mat4, Vec3};
use sariema_format::scenery::{RoomIndex, Target};
use sariema_format::script::{Entity, Kind, RoomTransition, name_id};

use crate::hash::StateHasher;
use crate::tween::steps_of;
use crate::world::{Transform, World};

/// Why a scenery command was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneReject {
    /// No room of the pack has that name.
    UnknownRoom,
    /// The active room has no group, piece, state or parameter of that name.
    Unknown,
}

impl SceneReject {
    pub const ALL: [SceneReject; 2] = [SceneReject::UnknownRoom, SceneReject::Unknown];

    pub const fn code(self) -> &'static str {
        match self {
            SceneReject::UnknownRoom => "E_ROOM_UNKNOWN",
            SceneReject::Unknown => "E_SCENE_UNKNOWN",
        }
    }
}

/// One room's index with its names sorted by id, and what the game changed.
#[derive(Clone, Debug)]
pub struct RoomScene {
    pub name: u32,
    pub index: RoomIndex,
    /// By group, whether it is on.
    pub groups: Vec<bool>,
    /// By piece, the index of its state shown.
    pub pieces: Vec<u32>,
    /// By parameter, its value.
    pub params: Vec<f32>,
    group_ids: Vec<(u32, u32)>,
    piece_ids: Vec<(u32, u32)>,
    /// By piece, its states' ids, the start look's as 0.
    state_ids: Vec<Vec<(u32, u32)>>,
    node_ids: Vec<(u32, u32)>,
    light_ids: Vec<(u32, u32)>,
    param_ids: Vec<(u32, u32)>,
}

fn sorted_ids<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(u32, u32)> {
    let mut ids: Vec<(u32, u32)> = names
        .enumerate()
        .map(|(i, n)| (name_id(n), i as u32))
        .collect();
    ids.sort_unstable();
    ids
}

fn find(ids: &[(u32, u32)], id: u32) -> Option<usize> {
    let at = ids.binary_search_by_key(&id, |&(k, _)| k).ok()?;
    Some(ids[at].1 as usize)
}

impl RoomScene {
    pub fn new(name: &str, index: RoomIndex) -> RoomScene {
        let state_ids = (index.pieces.iter())
            .map(|p| {
                let mut ids: Vec<(u32, u32)> = (p.states.iter().enumerate())
                    .map(|(i, s)| (if s.is_empty() { 0 } else { name_id(s) }, i as u32))
                    .collect();
                ids.sort_unstable();
                ids
            })
            .collect();
        RoomScene {
            name: name_id(name),
            groups: index.groups.iter().map(|g| g.1).collect(),
            pieces: index.pieces.iter().map(|p| p.state).collect(),
            params: index.params.iter().map(|p| p.value).collect(),
            group_ids: sorted_ids(index.groups.iter().map(|g| g.0.as_str())),
            piece_ids: sorted_ids(index.pieces.iter().map(|p| p.name.as_str())),
            state_ids,
            node_ids: sorted_ids(index.nodes.iter().map(|n| n.name.as_str())),
            light_ids: sorted_ids(index.lights.iter().map(|l| l.name.as_str())),
            param_ids: sorted_ids(index.params.iter().map(|p| p.name.as_str())),
            index,
        }
    }

    pub fn param(&self, id: u32) -> Option<usize> {
        find(&self.param_ids, id)
    }

    /// The value of the parameter aimed at `target`, if the room has it.
    pub fn value(&self, target: Target) -> Option<f32> {
        let i = self.index.params.iter().position(|p| p.target == target)?;
        Some(self.params[i])
    }

    fn hash_into(&self, h: &mut StateHasher) {
        let changed = |now: bool, then: bool| now != then;
        for (i, (&on, g)) in self.groups.iter().zip(&self.index.groups).enumerate() {
            if changed(on, g.1) {
                h.u32(i as u32);
                h.u32(on as u32);
            }
        }
        for (i, (&state, p)) in self.pieces.iter().zip(&self.index.pieces).enumerate() {
            if state != p.state {
                h.u32(i as u32);
                h.u32(state);
            }
        }
        for (i, (&value, p)) in self.params.iter().zip(&self.index.params).enumerate() {
            if value.to_bits() != p.value.to_bits() {
                h.u32(i as u32);
                h.f32(value);
            }
        }
    }
}

/// How the switch to the active room is shown, while it lasts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Switch {
    /// The room left, by index.
    pub from: usize,
    pub kind: RoomTransition,
    pub steps: u32,
    pub elapsed: u32,
}

impl Switch {
    /// How far it is, 0 to 1, at `alpha` between the last two steps.
    pub fn progress(&self, alpha: f32) -> f32 {
        let at = (self.elapsed as f32 - 1.0 + alpha).max(0.0);
        (at / self.steps.max(1) as f32).min(1.0)
    }
}

/// What a step asked of the rooms, for the engine and the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomRequest {
    /// Load this room, by index, in the background.
    Preload(usize),
    /// This room became the active one.
    Switch(usize),
}

/// What a room node or light entity stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Binding {
    /// Node `node` of room `room`; its draws follow the entity's placement
    /// from where `start` put them.
    Node {
        room: usize,
        node: usize,
        start: Mat4,
    },
    /// Light `light` of room `room`, at the entity's placement.
    Light { room: usize, light: usize },
}

#[derive(Clone, Debug, Default)]
pub struct Scenery {
    rooms: Vec<RoomScene>,
    active: usize,
    switch: Option<Switch>,
    /// By entity slot.
    bindings: Vec<Option<(Entity, Binding)>>,
    /// This step's, emptied when the next step starts.
    pub requests: Vec<RoomRequest>,
    /// Every room the game asked to preload, the latest last, until it
    /// becomes active.
    preloads: Vec<usize>,
}

impl Scenery {
    pub const EMPTY: Scenery = Scenery {
        rooms: Vec::new(),
        active: 0,
        switch: None,
        bindings: Vec::new(),
        requests: Vec::new(),
        preloads: Vec::new(),
    };

    /// The rooms of a pack, the first active.
    pub fn new(rooms: Vec<RoomScene>) -> Scenery {
        Scenery {
            rooms,
            ..Scenery::default()
        }
    }

    pub fn rooms(&self) -> &[RoomScene] {
        &self.rooms
    }

    pub fn room(&self, i: usize) -> Option<&RoomScene> {
        self.rooms.get(i)
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn active_room(&self) -> Option<&RoomScene> {
        self.rooms.get(self.active)
    }

    pub fn switch(&self) -> Option<Switch> {
        self.switch
    }

    pub fn room_of(&self, name: u32) -> Option<usize> {
        self.rooms.iter().position(|r| r.name == name)
    }

    pub fn begin_step(&mut self) {
        self.requests.clear();
    }

    pub fn step(&mut self) {
        if let Some(s) = &mut self.switch {
            s.elapsed += 1;
            if s.elapsed > s.steps {
                self.switch = None;
            }
        }
    }

    pub fn preload(&mut self, room: u32) -> Result<(), SceneReject> {
        let i = self.room_of(room).ok_or(SceneReject::UnknownRoom)?;
        self.requests.push(RoomRequest::Preload(i));
        self.preloads.retain(|&r| r != i);
        if i != self.active {
            self.preloads.push(i);
        }
        Ok(())
    }

    /// The rooms the game asked to preload and has not entered, the latest
    /// last.
    pub fn preloads(&self) -> &[usize] {
        &self.preloads
    }

    /// Makes `room` active; false when it already was.
    pub fn switch_to(
        &mut self,
        room: u32,
        kind: RoomTransition,
        duration: f32,
    ) -> Result<bool, SceneReject> {
        let i = self.room_of(room).ok_or(SceneReject::UnknownRoom)?;
        if i == self.active {
            return Ok(false);
        }
        self.switch = match kind {
            RoomTransition::Cut => None,
            // A dissolve lasts a step at least, so the host sees it.
            _ => Some(Switch {
                from: self.active,
                kind,
                steps: match kind {
                    RoomTransition::Dissolve => steps_of(duration).max(1),
                    _ => steps_of(duration),
                },
                elapsed: 0,
            }),
        };
        self.active = i;
        self.preloads.retain(|&r| r != i);
        self.requests.push(RoomRequest::Switch(i));
        Ok(true)
    }

    pub fn set_group(&mut self, group: u32, on: bool) -> Result<(), SceneReject> {
        let room = self
            .rooms
            .get_mut(self.active)
            .ok_or(SceneReject::Unknown)?;
        let g = find(&room.group_ids, group).ok_or(SceneReject::Unknown)?;
        room.groups[g] = on;
        Ok(())
    }

    pub fn set_piece(&mut self, piece: u32, state: u32) -> Result<(), SceneReject> {
        let room = self
            .rooms
            .get_mut(self.active)
            .ok_or(SceneReject::Unknown)?;
        let p = find(&room.piece_ids, piece).ok_or(SceneReject::Unknown)?;
        let s = find(&room.state_ids[p], state).ok_or(SceneReject::Unknown)?;
        room.pieces[p] = s as u32;
        Ok(())
    }

    /// The active room's node named `node`, as its room's index and its own.
    pub fn node(&self, node: u32) -> Result<(usize, usize), SceneReject> {
        let room = self.active_room().ok_or(SceneReject::Unknown)?;
        let n = find(&room.node_ids, node).ok_or(SceneReject::Unknown)?;
        Ok((self.active, n))
    }

    /// The active room's parameter `id`.
    pub fn param(&self, id: u32) -> Result<f32, SceneReject> {
        let room = self.active_room().ok_or(SceneReject::Unknown)?;
        let p = room.param(id).ok_or(SceneReject::Unknown)?;
        Ok(room.params[p])
    }

    pub fn set_param(&mut self, id: u32, value: f32) -> Result<(), SceneReject> {
        let room = self
            .rooms
            .get_mut(self.active)
            .ok_or(SceneReject::Unknown)?;
        let p = room.param(id).ok_or(SceneReject::Unknown)?;
        room.params[p] = value;
        Ok(())
    }

    /// Binds a room node or light entity just spawned to the active room's
    /// node or light named `asset`, placing it where the room has it; an
    /// asset the room lacks leaves it unbound.
    pub fn bind(&mut self, world: &mut World, entity: Entity, kind: Kind, asset: u32) {
        let slot = entity.index as usize;
        if self.bindings.len() <= slot {
            self.bindings.resize(slot + 1, None);
        }
        self.bindings[slot] = None;
        let Some(room) = self.rooms.get(self.active) else {
            return;
        };
        let room_index = self.active;
        let bound = match kind {
            Kind::RoomNode => find(&room.node_ids, asset).map(|node| {
                let n = &room.index.nodes[node];
                let (local, start) = placement_of(Mat4::from_cols_array(&n.pivot));
                (
                    local,
                    n.visible,
                    Binding::Node {
                        room: room_index,
                        node,
                        start,
                    },
                )
            }),
            Kind::Light => find(&room.light_ids, asset).map(|light| {
                let local = Transform {
                    position: Vec3::from(room.index.lights[light].position),
                    ..Transform::IDENTITY
                };
                (
                    local,
                    true,
                    Binding::Light {
                        room: room_index,
                        light,
                    },
                )
            }),
            _ => None,
        };
        if let Some((local, visible, binding)) = bound
            && let Ok(node) = world.node_mut(entity)
        {
            node.local = local;
            node.visible = visible;
            self.bindings[slot] = Some((entity, binding));
        }
    }

    /// What `entity` stands for, while it is alive in `world`.
    pub fn binding(&self, world: &World, entity: Entity) -> Option<Binding> {
        let (e, b) = (*self.bindings.get(entity.index as usize)?)?;
        (e == entity && world.is_alive(entity)).then_some(b)
    }

    /// Every bound entity alive in `world`.
    pub fn bound<'a>(&'a self, world: &'a World) -> impl Iterator<Item = (Entity, Binding)> + 'a {
        (self.bindings.iter().flatten())
            .filter(|(e, _)| world.is_alive(*e))
            .copied()
    }

    /// The active room, the switch and what differs in each room from how
    /// it loads; nothing when no room was ever changed or switched.
    pub fn hash_into(&self, h: &mut StateHasher) {
        if self.active != 0 {
            h.u32(self.active as u32);
        }
        if let Some(s) = self.switch {
            h.u32(s.from as u32);
            h.u32(s.kind as u32);
            h.u32(s.steps);
            h.u32(s.elapsed);
        }
        for room in &self.rooms {
            room.hash_into(h);
        }
    }
}

/// The entity placement nearest a pivot (position, yaw, pitch, roll and a
/// uniform scale), and that placement as a matrix: the node's draws follow
/// the entity's placement relative to it.
fn placement_of(pivot: Mat4) -> (Transform, Mat4) {
    let (scale, rotation, position) = pivot.to_scale_rotation_translation();
    let (yaw, pitch, roll) = rotation.to_euler(EulerRot::YXZ);
    let local = Transform {
        position,
        yaw: yaw.to_degrees(),
        pitch: pitch.to_degrees(),
        roll: roll.to_degrees(),
        scale: (scale.x * scale.y * scale.z).cbrt(),
    };
    let start = Mat4::from_scale_rotation_translation(
        Vec3::splat(local.scale),
        local.rotation(),
        local.position,
    );
    (local, start)
}
