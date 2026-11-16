// SPDX-License-Identifier: Apache-2.0

//! The `check` command: runs every check on the project's rooms, then the
//! cutscene rules on its game, and fails with the first error's code, listing
//! every error of that kind in the message. Warnings do not fail it.

use std::path::{Path, PathBuf};

use clap::Args;
use sariema_import::build_collision;
use serde_json::{Value, json};

use crate::output::{Done, Failure};
use crate::spot_check::{self, SpotCheckSettings};
use crate::{game, project};

#[derive(Args)]
pub struct CheckArgs {
    /// Directory holding sariema.json.
    project: PathBuf,
    /// Room id from the project's `rooms`; every room when absent.
    #[arg(long)]
    room: Option<String>,
    /// Metres an actor's model may reach past its collider before a warning [default: 0.05].
    #[arg(long)]
    bounds_margin: Option<f32>,
    /// Metres below a spot's feet walkable ground must be [default: 0.05].
    #[arg(long)]
    ground_depth: Option<f32>,
    /// Metres a collider may sit inside the room at a spot [default: 0.01].
    #[arg(long)]
    penetration_tolerance: Option<f32>,
    /// Metres a traversal's declared contact point may lie off a surface, or inside one [default: 0.02].
    #[arg(long)]
    contact_tolerance: Option<f32>,
}

pub fn run(args: &CheckArgs) -> Result<Done, Failure> {
    let defaults = SpotCheckSettings::default();
    let spots = SpotCheckSettings {
        bounds_margin: args.bounds_margin.unwrap_or(defaults.bounds_margin),
        ground_depth: args.ground_depth.unwrap_or(defaults.ground_depth),
        penetration_tolerance: args
            .penetration_tolerance
            .unwrap_or(defaults.penetration_tolerance),
        contact_tolerance: args.contact_tolerance.unwrap_or(defaults.contact_tolerance),
    };
    check(&args.project, args.room.as_deref(), &spots)
}

/// One problem a check found. Codes are stable: `E_` for errors, `W_` for warnings.
pub struct Finding {
    pub code: &'static str,
    pub message: String,
    pub error: bool,
}

/// Checks `room`, or every room of the project when `None`.
pub fn check(dir: &Path, room: Option<&str>, spots: &SpotCheckSettings) -> Result<Done, Failure> {
    let ids = match room {
        Some(room) => vec![room.to_owned()],
        None => project::room_ids(dir)?,
    };
    let mut findings = Vec::new();
    let mut rooms = Vec::new();
    for id in &ids {
        let (source, assets_root) = project::room_source(dir, id)?;
        let mut report =
            json!({ "room": id, "collision": false, "spots": 0, "routes": 0, "traversals": 0 });
        if let Some(pack) = build_collision(&source, &assets_root)? {
            spot_check::check_room(id, &pack, spots, &mut findings)?;
            report["collision"] = Value::Bool(true);
            report["spots"] = pack.spots.len().into();
            report["routes"] = pack.routes.len().into();
            report["traversals"] = pack.traversals.len().into();
        }
        rooms.push(report);
    }
    let errors: Vec<&Finding> = findings.iter().filter(|f| f.error).collect();
    if let Some(first) = errors.first() {
        let messages: Vec<&str> = errors.iter().map(|f| f.message.as_str()).collect();
        return Err(Failure::new(first.code, messages.join("; ")));
    }
    let warnings: Vec<Value> = findings
        .iter()
        .map(|f| json!({ "code": f.code, "message": f.message }))
        .collect();
    let cutscenes = game::check(dir)?;
    let mut text = format!(
        "checked rooms [{}] and cutscenes [{}]: no errors",
        ids.join(", "),
        cutscenes.join(", ")
    );
    for f in &findings {
        text.push_str(&format!("\nwarning[{}]: {}", f.code, f.message));
    }
    Ok(Done {
        json: json!({ "rooms": rooms, "cutscenes": cutscenes, "warnings": warnings }),
        text,
    })
}
