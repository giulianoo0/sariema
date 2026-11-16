// SPDX-License-Identifier: Apache-2.0

//! `build --actor` and `inspect` of an actor pack.

use std::fs;
use std::path::Path;

use sariema_format::actor::{ActorPack, Values, decode, encode};
use sariema_import::{ActorSource, TextureMode, build_actor};
use serde_json::{Value, json};

use crate::output::{Done, Failure, list};
use crate::project;

pub fn read(path: &Path) -> Result<ActorPack, Failure> {
    let bytes = fs::read(path).map_err(|e| Failure::io(path, e))?;
    decode(&bytes).map_err(|e| Failure::new(e.code(), format!("{}: {e}", path.display())))
}

/// `textures`, when given, replaces the actor description's choice.
pub fn build(
    dir: &Path,
    actor: &str,
    output: &Path,
    textures: Option<TextureMode>,
) -> Result<Done, Failure> {
    let project = project::load(dir)?;
    let description = project.actors.get(actor).ok_or_else(|| {
        let ids: Vec<&str> = project.actors.keys().map(String::as_str).collect();
        Failure::new(
            "E_SCHEMA",
            format!(
                "{}: no actor {actor}; its actors are {}",
                dir.join("sariema.json").display(),
                list(&ids)
            ),
        )
    })?;
    let description = dir.join(description);
    let text = fs::read_to_string(&description).map_err(|e| Failure::io(&description, e))?;
    let at = |e: sariema_import::ImportError| {
        Failure::new(e.code(), format!("{}: {e}", description.display()))
    };
    let mut source = ActorSource::from_json(&text).map_err(at)?;
    source.textures = textures.or(source.textures);
    let pack = build_actor(&source, &dir.join(&project.assets_root)).map_err(at)?;
    let bytes = encode(&pack);
    fs::write(output, &bytes).map_err(|e| Failure::io(output, e))?;
    let summary = summary(&pack);
    Ok(Done {
        text: format!(
            "built {actor} into {}: {} bytes, {} joints, {} clips, {} triangles",
            output.display(),
            bytes.len(),
            pack.joints.len(),
            pack.clips.len(),
            summary["triangles"]
        ),
        json: json!({
            "actor": actor,
            "output": output.display().to_string(),
            "bytes": bytes.len(),
            "joints": pack.joints.len(),
            "clips": pack.clips.len(),
            "triangles": summary["triangles"],
        }),
    })
}

pub fn inspect(path: &Path) -> Result<Done, Failure> {
    let pack = read(path)?;
    let summary = summary(&pack);
    let clips: Vec<String> = pack
        .clips
        .iter()
        .map(|c| {
            let speed = c
                .ground_speed
                .map_or(String::new(), |s| format!(", {s:.2} m/s"));
            let looping = if c.looping { " loop" } else { "" };
            format!("{} {:.2} s{looping}{speed}", c.name, c.duration)
        })
        .collect();
    let joints: Vec<&str> = pack.joints.iter().map(|j| j.name.as_str()).collect();
    let parts: Vec<String> = pack
        .parts
        .iter()
        .map(|p| {
            if p.visible {
                p.name.clone()
            } else {
                format!("{} (hidden)", p.name)
            }
        })
        .collect();
    let sockets: Vec<&str> = pack.sockets.iter().map(|s| s.name.as_str()).collect();
    let faces: Vec<&str> = pack.faces.iter().map(|f| f.name.as_str()).collect();
    Ok(Done {
        text: format!(
            "{}: {} joints, {} clips, {} triangles in {} parts; {} key bytes, {} texture bytes\n\
             joints: {}\nclips: {}\nparts: {}\nsockets: {}\nfaces: {}",
            pack.name,
            pack.joints.len(),
            pack.clips.len(),
            summary["triangles"],
            pack.parts.len(),
            summary["key_bytes"],
            summary["texture_bytes"],
            list(&joints),
            list(&clips),
            list(&parts),
            list(&sockets),
            list(&faces),
        ),
        json: json!({
            "actor": pack.name,
            "stats": summary,
            "joints": joints,
            "clips": pack.clips.iter().map(|c| json!({
                "name": c.name,
                "duration_s": decimal(c.duration),
                "looping": c.looping,
                "channels": c.channels.len(),
                "keys": c.channels.iter().map(|ch| ch.times.len()).sum::<usize>(),
                "markers": c.markers.iter().map(|m| json!({"name": m.name, "time_s": decimal(m.time)})).collect::<Vec<_>>(),
                "ground_speed_m_s": c.ground_speed.map(decimal),
                "bounds": c.bounds,
            })).collect::<Vec<_>>(),
            "parts": pack.parts.iter().map(|p| json!({"name": p.name, "visible": p.visible})).collect::<Vec<_>>(),
            "sockets": sockets,
            "faces": pack.faces.iter().map(|f| json!({
                "name": f.name,
                "frames": f.frames.iter().map(|fr| fr.name.as_str()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "bounds": pack.bounds,
        }),
    })
}

/// `x` as the shortest decimal that reads back as it, so 0.8 prints as 0.8.
fn decimal(x: f32) -> f64 {
    x.to_string().parse().unwrap_or(f64::from(x))
}

/// Sizes of what the pack holds.
fn summary(pack: &ActorPack) -> Value {
    let key_bytes: usize = pack
        .clips
        .iter()
        .flat_map(|c| &c.channels)
        .map(|ch| {
            let value = match &ch.values {
                Values::Vec3(_) => 12,
                Values::Quat(_) => 16,
                Values::QuatSnorm16(_) => 8,
            };
            ch.times.len() * (4 + value)
        })
        .sum();
    json!({
        "joints": pack.joints.len(),
        "clips": pack.clips.len(),
        "triangles": pack.meshes.iter().map(|m| m.indices.len() / 3).sum::<usize>(),
        "vertices": pack.meshes.iter().map(|m| m.vertices.len()).sum::<usize>(),
        "key_bytes": key_bytes,
        "texture_bytes": pack.textures.iter().flat_map(|t| &t.mips).map(Vec::len).sum::<usize>(),
    })
}
