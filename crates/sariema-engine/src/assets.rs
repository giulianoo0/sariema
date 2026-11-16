// SPDX-License-Identifier: Apache-2.0

//! What a game pack carries besides its logic, decoded for the modules.

use sariema_format::actor::ActorPack;
use sariema_format::collision::CollisionPack;
use sariema_format::script::{GameAssets, Hud};
use sariema_format::sound::SoundBank;
use sariema_runtime::game::LoadError;

#[derive(Clone, Debug, Default)]
pub struct Assets {
    /// The room pack, for the host to draw; the engine does not read it.
    pub room: Option<Vec<u8>>,
    pub collision: Option<CollisionPack>,
    /// By the name the game spawns them with.
    pub actors: Vec<(String, ActorPack)>,
    pub sounds: Option<SoundBank>,
    pub hud: Option<Hud>,
}

impl Assets {
    pub fn decode(assets: GameAssets) -> Result<Assets, LoadError> {
        let failed = |what: &str, code: &'static str, e: &dyn std::fmt::Display| LoadError {
            code,
            message: format!("game pack {what}: {e}"),
        };
        let collision = match &assets.collision {
            Some(bytes) => Some(
                sariema_format::collision::decode(bytes)
                    .map_err(|e| failed("collision", e.code(), &e))?,
            ),
            None => None,
        };
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
            room: assets.room,
            collision,
            actors,
            sounds,
            hud: assets.hud,
        })
    }
}
