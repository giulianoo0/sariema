// SPDX-License-Identifier: Apache-2.0

//! A game project's `sariema.json` and the `build` command.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sariema_format::encode;
use sariema_import::{RoomSource, TextureMode, build_room};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::output::{Done, Failure, list};

const FILE: &str = "sariema.json";
const SCHEMA: &str = include_str!("../../../schemas/project.schema.json");

/// `schemas/project.schema.json`; `name` is checked there and not needed here.
#[derive(Deserialize)]
struct Project {
    assets_root: PathBuf,
    /// Room id to its room description, relative to the project directory.
    rooms: BTreeMap<String, PathBuf>,
}

/// Reads the project in `dir` and checks it against the schema.
fn load(dir: &Path) -> Result<Project, Failure> {
    let path = dir.join(FILE);
    let text = fs::read_to_string(&path).map_err(|e| Failure::io(&path, e))?;
    let schema_error = |why: String| Failure::new("E_SCHEMA", format!("{}: {why}", path.display()));
    let value: Value = serde_json::from_str(&text).map_err(|e| schema_error(e.to_string()))?;
    let schema: Value = serde_json::from_str(SCHEMA).expect("project.schema.json is JSON");
    jsonschema::validate(&schema, &value).map_err(|e| {
        let at = e.instance_path().to_string();
        schema_error(if at.is_empty() {
            e.to_string()
        } else {
            format!("{at}: {e}")
        })
    })?;
    serde_json::from_value(value).map_err(|e| schema_error(e.to_string()))
}

/// `textures`, when given, replaces the room file's choice.
pub fn build(
    dir: &Path,
    room: &str,
    output: &Path,
    textures: Option<TextureMode>,
) -> Result<Done, Failure> {
    let (mut source, assets_root) = room_source(dir, room)?;
    source.textures = textures.or(source.textures);
    let pack = build_room(&source, &assets_root)?;
    let bytes = encode(&pack);
    fs::write(output, &bytes).map_err(|e| Failure::io(output, e))?;
    let (collision, collision_text) = crate::collision::bake_beside(&source, &assets_root, output)?;
    let stats = &pack.stats;
    Ok(Done {
        json: json!({
            "room": room,
            "output": output.display().to_string(),
            "bytes": bytes.len(),
            "stats": stats,
            "collision": collision,
        }),
        text: format!(
            "built {room} into {}: {} bytes, {} triangles in {} draws, {} texture bytes{collision_text}",
            output.display(),
            bytes.len(),
            stats.triangles,
            stats.draws,
            stats.texture_bytes
        ),
    })
}

/// The project's room ids, in order.
pub(crate) fn room_ids(dir: &Path) -> Result<Vec<String>, Failure> {
    Ok(load(dir)?.rooms.into_keys().collect())
}

/// Room `room`'s description, and the assets root its paths are relative to.
pub(crate) fn room_source(dir: &Path, room: &str) -> Result<(RoomSource, PathBuf), Failure> {
    let project = load(dir)?;
    let description = project.rooms.get(room).ok_or_else(|| {
        let ids: Vec<&str> = project.rooms.keys().map(String::as_str).collect();
        Failure::new(
            "E_SCHEMA",
            format!(
                "{}: no room {room}; its rooms are {}",
                dir.join(FILE).display(),
                list(&ids)
            ),
        )
    })?;
    let description = dir.join(description);
    let text = fs::read_to_string(&description).map_err(|e| Failure::io(&description, e))?;
    let source = RoomSource::from_json(&text)
        .map_err(|e| Failure::new(e.code(), format!("{}: {e}", description.display())))?;
    Ok((source, dir.join(&project.assets_root)))
}
