// SPDX-License-Identifier: Apache-2.0

//! What a game pack carries besides its logic, decoded for the modules.

use std::sync::Arc;

use sariema_format::actor::ActorPack;
use sariema_format::collision::CollisionPack;
use sariema_format::scenery::RoomIndex;
use sariema_format::script::{GameAssets, GameRoom, Hud};
use sariema_format::sound::SoundBank;
use sariema_runtime::game::LoadError;

#[derive(Clone, Debug, Default)]
pub struct Assets {
    /// The first is where the game starts.
    pub rooms: Vec<RoomAssets>,
    /// By the name the game spawns them with.
    pub actors: Vec<(String, ActorPack)>,
    pub sounds: Option<SoundBank>,
    pub hud: Option<Hud>,
}

#[derive(Clone, Debug, Default)]
pub struct RoomAssets {
    pub name: String,
    /// The room pack, for the host to draw; the engine does not read it.
    pub room: Option<Arc<[u8]>>,
    pub collision: Option<Arc<CollisionPack>>,
    pub index: RoomIndex,
    /// Rooms by index, nearest first.
    pub neighbours: Vec<u32>,
    /// Rooms by index drawn with this one, their geometry continuing it.
    pub beside: Vec<u32>,
    pub ambience: Option<String>,
}

impl RoomAssets {
    /// A room only collided with, as tests and benches make them.
    pub fn collision(name: &str, pack: CollisionPack) -> RoomAssets {
        RoomAssets {
            name: name.into(),
            collision: Some(Arc::new(pack)),
            ..RoomAssets::default()
        }
    }
}

impl Assets {
    pub fn decode(assets: GameAssets) -> Result<Assets, LoadError> {
        let failed = |what: &str, code: &'static str, e: &dyn std::fmt::Display| LoadError {
            code,
            message: format!("game pack {what}: {e}"),
        };
        let rooms = (assets.rooms.into_iter())
            .map(|r: GameRoom| {
                let collision = match &r.collision {
                    Some(bytes) => {
                        Some(Arc::new(sariema_format::collision::decode(bytes).map_err(
                            |e| failed(&format!("room {} collision", r.name), e.code(), &e),
                        )?))
                    }
                    None => None,
                };
                Ok(RoomAssets {
                    name: r.name,
                    room: r.room.map(Arc::from),
                    collision,
                    index: r.index,
                    neighbours: r.neighbours,
                    beside: r.beside,
                    ambience: r.ambience,
                })
            })
            .collect::<Result<Vec<_>, LoadError>>()?;
        let actors = assets
            .actors
            .iter()
            .map(|(name, bytes)| {
                sariema_format::actor::decode(bytes)
                    .map(|pack| (name.clone(), pack))
                    .map_err(|e| failed(&format!("actor {name}"), e.code(), &e))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let sounds = match &assets.sounds {
            Some(bytes) => Some(
                sariema_format::sound::decode(bytes)
                    .map_err(|e| failed("sound bank", e.code(), &e))?,
            ),
            None => None,
        };
        Ok(Assets {
            rooms,
            actors,
            sounds,
            hud: assets.hud,
        })
    }

    pub fn collides(&self) -> bool {
        self.rooms.first().is_some_and(|r| r.collision.is_some())
    }
}
