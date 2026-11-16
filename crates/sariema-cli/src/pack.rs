// SPDX-License-Identifier: Apache-2.0

//! Reading a room pack, and the `inspect` command, which lists a room's
//! scene parameters with the values they load with.

use std::fs;
use std::path::Path;

use sariema_format::scenery::RoomIndex;
use sariema_format::{RoomPack, decode};
use serde_json::{Value, json};

use crate::output::{Done, Failure, list};

pub fn read(path: &Path) -> Result<RoomPack, Failure> {
    decoded(path, &fs::read(path).map_err(|e| Failure::io(path, e))?)
}

fn decoded(path: &Path, bytes: &[u8]) -> Result<RoomPack, Failure> {
    decode(bytes).map_err(|e| Failure::new(e.code(), format!("{}: {e}", path.display())))
}

pub fn inspect(path: &Path) -> Result<Done, Failure> {
    let bytes = fs::read(path).map_err(|e| Failure::io(path, e))?;
    if bytes.starts_with(&sariema_format::collision::MAGIC) {
        return crate::collision::inspect(path, &bytes);
    }
    if bytes.starts_with(&sariema_format::actor::MAGIC) {
        return crate::actor::inspect(path, &bytes);
    }
    let room = decoded(path, &bytes)?;
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
    let params: Vec<Value> = (RoomIndex::of(&room).params.iter())
        .map(|p| json!({ "name": p.name, "value": p.value }))
        .collect();
    let cutaways: Vec<Value> = (room.cutaways.iter())
        .map(|c| {
            json!({
                "name": c.name,
                "draws": c.draws.len(),
                "height": c.height,
                "areas": c.line.areas.len(),
                "offset": c.line.offset,
                "follow": c.line.follow,
                "radius": c.radius,
                "soft": c.soft,
                "cap": c.cap,
                "cut_shadows": c.cut_shadows,
            })
        })
        .collect();
    let cut: Vec<String> = (room.cutaways.iter())
        .map(|c| format!("{} ({} draws above {} m)", c.name, c.draws.len(), c.height))
        .collect();
    let stats = &room.stats;
    Ok(Done {
        json: json!({
            "room": room.name,
            "stats": stats,
            "lights": lights,
            "materials": materials,
            "draws": loose,
            "params": params,
            "cutaways": cutaways,
        }),
        text: format!(
            "{}: {} triangles in {} draws, {} texture bytes\nlights: {}\nmaterials: {}\nloose draws: {}\ncutaways: {}",
            room.name,
            stats.triangles,
            stats.draws,
            stats.texture_bytes,
            list(&lights),
            list(&materials),
            list(&loose),
            list(&cut)
        ),
    })
}
