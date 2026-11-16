// SPDX-License-Identifier: Apache-2.0

//! Reading a room pack, and the `inspect` command.

use std::fs;
use std::path::Path;

use sariema_format::{RoomPack, decode};
use serde_json::json;

use crate::output::{Done, Failure, list};

pub fn read(path: &Path) -> Result<RoomPack, Failure> {
    let bytes = fs::read(path).map_err(|e| Failure::io(path, e))?;
    decode(&bytes).map_err(|e| Failure::new(e.code(), format!("{}: {e}", path.display())))
}

pub fn inspect(path: &Path) -> Result<Done, Failure> {
    let bytes = fs::read(path).map_err(|e| Failure::io(path, e))?;
    if bytes.starts_with(&sariema_format::actor::MAGIC) {
        return crate::actor::inspect(path);
    }
    let room = read(path)?;
    let lights: Vec<&str> = room.lights.iter().map(|l| l.name.as_str()).collect();
    let materials: Vec<&str> = room.materials.iter().map(|m| m.name.as_str()).collect();
    // shortcut: the pack has no loose flag yet, so this relies on the importer naming
    // merged draws `<room>/<material>`; a loose piece has one draw per primitive.
    let mut loose: Vec<&str> = room
        .draws
        .iter()
        .filter_map(|d| d.name.as_deref())
        .filter(|name| !name.contains('/'))
        .collect();
    loose.dedup();
    let stats = &room.stats;
    Ok(Done {
        json: json!({
            "room": room.name,
            "stats": stats,
            "lights": lights,
            "materials": materials,
            "draws": loose,
        }),
        text: format!(
            "{}: {} triangles in {} draws, {} texture bytes\nlights: {}\nmaterials: {}\nloose draws: {}",
            room.name,
            stats.triangles,
            stats.draws,
            stats.texture_bytes,
            list(&lights),
            list(&materials),
            list(&loose)
        ),
    })
}
