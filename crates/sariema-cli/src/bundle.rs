// SPDX-License-Identifier: Apache-2.0

//! What a game pack carries besides its logic: the room it plays in with
//! its collision, its actors, its sound bank and its HUD.

use std::fs;
use std::path::Path;

use sariema_format::script::{GameAssets, Hud};
use sariema_ui::UiDesc;

use crate::output::Failure;
use crate::{actor, audio, project};

/// The assets of the project in `dir`: `room` (the project's first room
/// without it) and the HUD description at `hud`, relative to `dir`.
pub fn assets(dir: &Path, room: Option<&str>, hud: Option<&Path>) -> Result<GameAssets, Failure> {
    let mut assets = GameAssets::default();
    let rooms = project::room_ids(dir)?;
    if let Some(room) = room.map(str::to_owned).or_else(|| rooms.first().cloned()) {
        let (pack, collision) = project::room_packs(dir, &room)?;
        assets.room = Some(pack);
        assets.collision = collision;
    }
    let project: project::Project = project::load(dir)?;
    for id in project.actors.keys() {
        let pack = actor::pack(dir, id, None)?;
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
