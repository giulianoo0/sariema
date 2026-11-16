// SPDX-License-Identifier: Apache-2.0

//! Collision packs: written beside the room pack by `build`, read by
//! `inspect`, `check` and `physics-map`.

use std::fs;
use std::path::Path;

use sariema_format::collision::{CollisionPack, Shape, decode, encode};
use sariema_import::{RoomSource, build_collision};
use serde_json::{Value, json};

use crate::output::{Done, Failure, list};

pub fn read(path: &Path) -> Result<CollisionPack, Failure> {
    let bytes = fs::read(path).map_err(|e| Failure::io(path, e))?;
    decode(&bytes).map_err(|e| Failure::new(e.code(), format!("{}: {e}", path.display())))
}

/// Bakes the room's collision and writes it beside `room_pack`, with the
/// extension `srmc`. Returns the JSON report, `null` when the room declares
/// no collision, and the text to append to `build`'s.
pub fn bake_beside(
    source: &RoomSource,
    assets_root: &Path,
    room_pack: &Path,
) -> Result<(Value, String), Failure> {
    let Some(pack) = build_collision(source, assets_root)? else {
        return Ok((Value::Null, String::new()));
    };
    let output = room_pack.with_extension("srmc");
    if output == room_pack {
        return Err(Failure::new(
            "E_USAGE",
            format!(
                "{}: the room pack needs another extension than srmc, which its collision pack takes",
                room_pack.display()
            ),
        ));
    }
    let bytes = encode(&pack);
    fs::write(&output, &bytes).map_err(|e| Failure::io(&output, e))?;
    let text = format!(
        "\ncollision into {}: {} bytes, {} colliders, {} triggers, {} spots",
        output.display(),
        bytes.len(),
        pack.colliders.len(),
        pack.triggers.len(),
        pack.spots.len()
    );
    Ok((
        json!({
            "output": output.display().to_string(),
            "bytes": bytes.len(),
            "colliders": pack.colliders.len(),
            "triangles": pack.triangles(),
            "triggers": pack.triggers.len(),
            "spots": pack.spots.len(),
        }),
        text,
    ))
}

pub fn inspect(path: &Path, bytes: &[u8]) -> Result<Done, Failure> {
    let pack =
        decode(bytes).map_err(|e| Failure::new(e.code(), format!("{}: {e}", path.display())))?;
    let mut shapes = [0usize; 5];
    for c in &pack.colliders {
        shapes[match c.shape {
            Shape::Box { .. } => 0,
            Shape::Sphere { .. } => 1,
            Shape::Capsule { .. } => 2,
            Shape::Hull { .. } => 3,
            Shape::Mesh { .. } => 4,
        }] += 1;
    }
    let disabled: Vec<&str> = pack
        .colliders
        .iter()
        .filter(|c| !c.enabled)
        .map(|c| c.name.as_str())
        .collect();
    let triggers: Vec<&str> = pack.triggers.iter().map(|t| t.name.as_str()).collect();
    let spots: Vec<&str> = pack.spots.iter().map(|s| s.name.as_str()).collect();
    let traversals: Vec<&str> = pack.traversals.iter().map(|t| t.name.as_str()).collect();
    let routes: Vec<[&str; 2]> = pack
        .routes
        .iter()
        .map(|r| r.map(|i| pack.spots[i as usize].name.as_str()))
        .collect();
    let actors: Vec<Value> = pack
        .actors
        .iter()
        .map(|a| json!({ "name": a.name, "radius": a.radius, "height": a.height, "head": a.head.as_ref().map(|h| json!({ "radius": h.radius, "height": h.height, "forward": h.forward })) }))
        .collect();
    let actor_names: Vec<&str> = pack.actors.iter().map(|a| a.name.as_str()).collect();
    Ok(Done {
        json: json!({
            "kind": "collision",
            "room": pack.room,
            "bytes": bytes.len(),
            "colliders": {
                "count": pack.colliders.len(),
                "triangles": pack.triangles(),
                "shapes": { "box": shapes[0], "sphere": shapes[1], "capsule": shapes[2], "hull": shapes[3], "mesh": shapes[4] },
                "disabled": disabled,
            },
            "triggers": triggers,
            "actors": actors,
            "spots": spots,
            "routes": routes,
            "traversals": traversals,
        }),
        text: format!(
            "{} collision: {} colliders ({} boxes, {} spheres, {} capsules, {} hulls, {} meshes; {} triangles)\ndisabled: {}\ntriggers: {}\nactors: {}\nspots: {}\nroutes: {}\ntraversals: {}",
            pack.room,
            pack.colliders.len(),
            shapes[0],
            shapes[1],
            shapes[2],
            shapes[3],
            shapes[4],
            pack.triangles(),
            list(&disabled),
            list(&triggers),
            list(&actor_names),
            list(&spots),
            list(
                &routes
                    .iter()
                    .map(|[a, b]| format!("{a} → {b}"))
                    .collect::<Vec<_>>()
            ),
            list(&traversals),
        ),
    })
}
