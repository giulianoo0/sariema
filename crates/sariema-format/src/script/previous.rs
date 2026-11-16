// SPDX-License-Identifier: Apache-2.0

//! Game pack version 3, read as the current one. Its room index ended at
//! the scene parameters: its rooms have no cutaway.

use serde::Deserialize;

use super::{Cutscene, GameAssets, GamePack, GameRoom, Hud};
use crate::scenery::{LightIndex, NodeIndex, Param, PieceIndex, RoomIndex};

pub(super) const VERSION: u32 = 3;

#[derive(Deserialize)]
pub(super) struct GamePack3 {
    name: String,
    batch_version: u32,
    wasm: Vec<u8>,
    cutscenes: Vec<Cutscene>,
    names: Vec<String>,
    settings: String,
    assets: GameAssets3,
}

#[derive(Deserialize)]
struct GameAssets3 {
    rooms: Vec<GameRoom3>,
    actors: Vec<(String, Vec<u8>)>,
    sounds: Option<Vec<u8>>,
    hud: Option<Hud>,
}

#[derive(Deserialize)]
struct GameRoom3 {
    name: String,
    room: Option<Vec<u8>>,
    collision: Option<Vec<u8>>,
    index: RoomIndex3,
    neighbours: Vec<u32>,
    beside: Vec<u32>,
    ambience: Option<String>,
}

#[derive(Deserialize)]
struct RoomIndex3 {
    groups: Vec<(String, bool)>,
    nodes: Vec<NodeIndex>,
    pieces: Vec<PieceIndex>,
    lights: Vec<LightIndex>,
    params: Vec<Param>,
}

impl From<GamePack3> for GamePack {
    fn from(old: GamePack3) -> GamePack {
        let assets = old.assets;
        GamePack {
            name: old.name,
            batch_version: old.batch_version,
            wasm: old.wasm,
            cutscenes: old.cutscenes,
            names: old.names,
            settings: old.settings,
            assets: GameAssets {
                rooms: assets.rooms.into_iter().map(GameRoom::from).collect(),
                actors: assets.actors,
                sounds: assets.sounds,
                hud: assets.hud,
            },
        }
    }
}

impl From<GameRoom3> for GameRoom {
    fn from(old: GameRoom3) -> GameRoom {
        let index = old.index;
        GameRoom {
            name: old.name,
            room: old.room,
            collision: old.collision,
            index: RoomIndex {
                groups: index.groups,
                nodes: index.nodes,
                pieces: index.pieces,
                lights: index.lights,
                params: index.params,
                cutaways: Vec::new(),
            },
            neighbours: old.neighbours,
            beside: old.beside,
            ambience: old.ambience,
        }
    }
}
