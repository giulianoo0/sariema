// SPDX-License-Identifier: Apache-2.0

//! The `web` command: a project's rooms and actors in both texture variants,
//! its sound bank, and the `sariema-web.json` the web package loads them by.
//! A device with ASTC takes `<id>.srma`; any other takes `<id>.rgba8.srma`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use sariema_import::TextureMode;
use serde_json::{Map, Value, json};

use crate::output::{Done, Failure};
use crate::{actor, audio, project};

const VARIANTS: [(&str, &str, TextureMode); 2] = [
    ("astc", "", TextureMode::Astc4x4),
    ("rgba8", ".rgba8", TextureMode::Rgba8),
];

#[derive(Args)]
pub struct WebArgs {
    /// Directory holding sariema.json.
    project: PathBuf,
    /// Folder to write the game into, served as it is.
    #[arg(short, long)]
    output: PathBuf,
    /// Room the page loads first; the project's first room by default.
    #[arg(long)]
    start: Option<String>,
}

/// Builds `id` in each variant with `build`, returning the manifest's entry
/// and the files' sizes.
fn variants(
    id: &str,
    extension: &str,
    out: &Path,
    build: impl Fn(&Path, TextureMode) -> Result<Done, Failure>,
) -> Result<(Value, Value), Failure> {
    let mut files = Map::new();
    let mut sizes = Map::new();
    for (variant, infix, mode) in VARIANTS {
        let file = format!("{id}{infix}.{extension}");
        let done = build(&out.join(&file), mode)?;
        sizes.insert(
            variant.into(),
            json!({ "file": file, "bytes": done.json["bytes"] }),
        );
        files.insert(variant.into(), file.into());
    }
    Ok((files.into(), sizes.into()))
}

pub fn run(args: &WebArgs) -> Result<Done, Failure> {
    let dir = &args.project;
    let out = &args.output;
    let actors = project::load(dir)?.actors;
    let rooms = project::room_ids(dir)?;
    let start = match &args.start {
        Some(room) if !rooms.contains(room) => {
            return Err(Failure::new(
                "E_USAGE",
                format!("--start {room} is not one of the project's rooms"),
            ));
        }
        Some(room) => Some(room.clone()),
        None => rooms.first().cloned(),
    };
    fs::create_dir_all(out).map_err(|e| Failure::io(out, e))?;

    let mut manifest = Map::new();
    let mut report = Map::new();
    let mut room_files = BTreeMap::new();
    let mut room_sizes = Map::new();
    for room in &rooms {
        let (files, sizes) = variants(room, "srma", out, |path, mode| {
            project::build(dir, room, path, Some(mode))
        })?;
        room_files.insert(room.clone(), files);
        room_sizes.insert(room.clone(), sizes);
    }
    let mut actor_files = BTreeMap::new();
    let mut actor_sizes = Map::new();
    for id in actors.keys() {
        let (files, sizes) = variants(id, "srmk", out, |path, mode| {
            actor::build(dir, id, path, Some(mode))
        })?;
        actor_files.insert(id.clone(), files);
        actor_sizes.insert(id.clone(), sizes);
    }
    if let Some(start) = &start {
        manifest.insert("start".into(), start.clone().into());
    }
    manifest.insert("rooms".into(), json!(room_files));
    manifest.insert("actors".into(), json!(actor_files));
    report.insert("rooms".into(), room_sizes.into());
    report.insert("actors".into(), actor_sizes.into());

    let config = dir.join("sariema.json");
    let text = fs::read_to_string(&config).map_err(|e| Failure::io(&config, e))?;
    let has_audio = serde_json::from_str::<Value>(&text).is_ok_and(|v| v.get("audio").is_some());
    if has_audio {
        let done = audio::build(dir, &out.join("sounds.srms"), false)?;
        manifest.insert("sounds".into(), "sounds.srms".into());
        report.insert("sounds".into(), done.json);
    }

    let path = out.join("sariema-web.json");
    let text = serde_json::to_string_pretty(&Value::from(manifest)).expect("JSON serializes");
    fs::write(&path, text).map_err(|e| Failure::io(&path, e))?;
    report.insert("manifest".into(), path.display().to_string().into());
    Ok(Done {
        text: format!(
            "wrote {} rooms and {} actors in two texture variants to {}",
            rooms.len(),
            actors.len(),
            out.display()
        ),
        json: report.into(),
    })
}
