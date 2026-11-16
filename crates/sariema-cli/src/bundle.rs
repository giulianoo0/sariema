// SPDX-License-Identifier: Apache-2.0

//! What a game pack carries besides its logic: the rooms it plays in with
//! their collision, its actors, its sound bank and its HUD.

use std::fs;
use std::path::Path;

use std::collections::BTreeMap;

use sariema_format::scenery::RoomIndex;
use sariema_format::script::{GameAssets, GameRoom, Hud};
use sariema_import::TextureMode;
use sariema_ui::UiDesc;
use serde::Deserialize;

use crate::output::Failure;
use crate::{actor, audio, project};

/// A room the game plays in besides its start room, as the `game.rooms`
/// of `sariema.json` lists it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomEntry {
    /// Rooms to keep loaded while this one is active, nearest first.
    #[serde(default)]
    pub neighbours: Vec<String>,
    /// A sound of the bank looped while the room is active.
    #[serde(default)]
    pub ambience: Option<String>,
    /// Rooms whose geometry continues this one's, drawn with it.
    #[serde(default)]
    pub beside: Vec<String>,
}

/// The assets of the project in `dir`: `room` (the project's first room
/// without it) first, then every other room of `rooms`, and the HUD
/// description at `hud`, relative to `dir`; `textures`, when given,
/// replaces the rooms' and actors' own choice.
pub fn assets(
    dir: &Path,
    room: Option<&str>,
    rooms: &BTreeMap<String, RoomEntry>,
    hud: Option<&Path>,
    textures: Option<TextureMode>,
) -> Result<GameAssets, Failure> {
    let mut assets = GameAssets::default();
    let ids = project::room_ids(dir)?;
    if let Some(start) = room.map(str::to_owned).or_else(|| ids.first().cloned()) {
        let names: Vec<&str> = std::iter::once(start.as_str())
            .chain(rooms.keys().map(String::as_str).filter(|&r| r != start))
            .collect();
        for &name in &names {
            let (pack, collision) = project::room_packs(dir, name, textures)?;
            let entry = rooms.get(name).cloned().unwrap_or_default();
            let index = |list: &[String], what: &str| {
                (list.iter())
                    .map(|n| match names.iter().position(|&r| r == n) {
                        Some(i) if names[i] != name => Ok(i as u32),
                        _ => Err(Failure::new(
                            "E_SCHEMA",
                            format!(
                                "{}: game.rooms.{name} lists {what} {n}, which is not another room of game.rooms",
                                dir.join("sariema.json").display()
                            ),
                        )),
                    })
                    .collect::<Result<Vec<u32>, Failure>>()
            };
            let neighbours = index(&entry.neighbours, "neighbour")?;
            let beside = index(&entry.beside, "beside")?;
            assets.rooms.push(GameRoom {
                name: name.to_owned(),
                index: RoomIndex::of(&pack),
                room: Some(sariema_format::encode(&pack)),
                collision,
                neighbours,
                beside,
                ambience: entry.ambience,
            });
        }
    }
    let project: project::Project = project::load(dir)?;
    for id in project.actors.keys() {
        let pack = actor::pack(dir, id, textures)?;
        assets
            .actors
            .push((id.clone(), sariema_format::actor::encode(&pack)));
    }
    assets.sounds = audio::bank_bytes(dir)?;
    if let Some(hud) = hud {
        assets.hud = Some(read_hud(&dir.join(hud))?);
    }
    Ok(assets)
}

/// The HUD description at `path` and every font and image it names, read
/// relative to it.
fn read_hud(path: &Path) -> Result<Hud, Failure> {
    let description = fs::read_to_string(path).map_err(|e| Failure::io(path, e))?;
    let desc: UiDesc = serde_json::from_str(&description)
        .map_err(|e| Failure::new("E_SCHEMA", format!("{}: {e}", path.display())))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let names = desc
        .fonts
        .values()
        .map(|f| f.file.as_str())
        .chain(desc.images.values().map(String::as_str));
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for name in names {
        if files.iter().any(|(n, _)| n == name) {
            continue;
        }
        let file = base.join(name);
        files.push((
            name.to_owned(),
            fs::read(&file).map_err(|e| Failure::io(&file, e))?,
        ));
    }
    Ok(Hud { description, files })
}
