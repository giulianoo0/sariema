// SPDX-License-Identifier: Apache-2.0

//! `sariema api`: every command, event, enum, setting and error code, from
//! the same table the batch decoder is generated from
//! (`schemas/api.schema.json`).

use sariema_format::script::{BATCH_VERSION, COMMANDS, ENUMS, EVENTS, EnumSpec, MessageSpec};
use sariema_runtime::settings::{DOCS, Settings};
use sariema_runtime::world::Reject;
use serde_json::{Value, json};

use crate::output::{Done, Failure};

/// How an error reaches the game or the tool using it.
#[derive(Clone, Copy)]
enum Kind {
    /// A command or a load fails with it.
    Failure,
    /// A command is refused at run time and a `rejected` event carries it;
    /// the game keeps running.
    Rejected,
    /// The game stops for good; the host gets a `stopped` message.
    Stop,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Failure => "failure",
            Kind::Rejected => "rejected",
            Kind::Stop => "stop",
        }
    }
}

/// Every stable error code, with what it means.
const ERRORS: &[(&str, Kind, &str)] = &[
    ("E_IO", Kind::Failure, "a file could not be read or written"),
    (
        "E_SCHEMA",
        Kind::Failure,
        "a JSON file or a pack field does not match its schema",
    ),
    ("E_USAGE", Kind::Failure, "the command line is not valid"),
    (
        "E_INTERNAL",
        Kind::Failure,
        "the tool failed in a way that is a bug",
    ),
    (
        "E_PACK_BAD_MAGIC",
        Kind::Failure,
        "the file is not a pack of this kind",
    ),
    (
        "E_PACK_VERSION",
        Kind::Failure,
        "the pack was written by another format version",
    ),
    ("E_PACK_TRUNCATED", Kind::Failure, "the pack ends early"),
    (
        "E_GLTF_EXTENSION",
        Kind::Failure,
        "the kit uses a glTF extension the importer refuses",
    ),
    (
        "E_ROOM_PIECE_MISSING",
        Kind::Failure,
        "the room names a piece the kit lacks",
    ),
    (
        "E_TEXTURE_ENCODE",
        Kind::Failure,
        "a texture could not be encoded",
    ),
    (
        "E_GOLDEN_MISMATCH",
        Kind::Failure,
        "a rendered frame differs from its reference image",
    ),
    (
        "E_ROOM_VOID",
        Kind::Failure,
        "too much of the frame shows no geometry",
    ),
    ("E_GPU", Kind::Failure, "the GPU device failed"),
    ("E_DEVICE", Kind::Failure, "no usable graphics device"),
    (
        "E_SURFACE",
        Kind::Failure,
        "the window surface could not be used",
    ),
    (
        "E_PROJECT_EXISTS",
        Kind::Failure,
        "new was given a directory that is not empty",
    ),
    (
        "E_GAME_BUILD",
        Kind::Failure,
        "cargo could not build the game's logic to WebAssembly",
    ),
    (
        "E_INPUT_LOG",
        Kind::Failure,
        "the input log names an unknown action or is out of step order",
    ),
    (
        "E_BATCH_VERSION",
        Kind::Failure,
        "the logic was built against another batch version",
    ),
    (
        "E_GAME_MODULE",
        Kind::Failure,
        "the logic is not valid WebAssembly, imports what the engine does not give, or lacks sariema_tick",
    ),
    (
        "E_CUTSCENE_ROLE",
        Kind::Failure,
        "a cutscene step names a role the cutscene does not declare",
    ),
    (
        "E_CUTSCENE_TELEPORT",
        Kind::Failure,
        "a cutscene teleports a role that may be on screen without declaring a cover",
    ),
    (
        "E_CUTSCENE_COVER",
        Kind::Failure,
        "a cutscene teleport's declared cover does not hold there",
    ),
    (
        "E_CUTSCENE_START",
        Kind::Failure,
        "a cutscene role has no start place and is used before a spawn",
    ),
    (
        "E_CUTSCENE_ABSENT",
        Kind::Failure,
        "a cutscene uses a role after its despawn",
    ),
    (
        "E_CUTSCENE_SPAWN",
        Kind::Failure,
        "a cutscene spawns a role that may already be in the world",
    ),
    (
        "E_CUTSCENE_RACE",
        Kind::Failure,
        "parallel branches move the same role or set the same flag",
    ),
    (
        "E_CUTSCENE_SPEED",
        Kind::Failure,
        "a cutscene walk, or the walk to a role's start, is faster than motion.max_visible_speed",
    ),
    (
        "E_SETTINGS",
        Kind::Failure,
        "the runtime settings cannot run: a time scale bound is not positive or min_time_scale is above max_time_scale",
    ),
    (
        "E_BATCH_TRUNCATED",
        Kind::Stop,
        "a command record ends early",
    ),
    (
        "E_BATCH_UNKNOWN",
        Kind::Stop,
        "a command record has an unknown opcode",
    ),
    (
        "E_BATCH_INVALID",
        Kind::Stop,
        "a command field holds a value its type does not allow",
    ),
    (
        "E_GAME_BUDGET",
        Kind::Stop,
        "a tick used its whole execution budget (vm.fuel_per_step)",
    ),
    (
        "E_GAME_MEMORY",
        Kind::Stop,
        "the logic asked for more memory than vm.memory_bytes",
    ),
    (
        "E_GAME_TRAP",
        Kind::Stop,
        "the logic trapped, panicked or overflowed its call stack",
    ),
    ("E_ENTITY_STALE", Kind::Rejected, Reject::Stale.message()),
    ("E_ENTITY_IN_USE", Kind::Rejected, Reject::InUse.message()),
    ("E_ENTITY_LIMIT", Kind::Rejected, Reject::Limit.message()),
    ("E_PARENT_CYCLE", Kind::Rejected, Reject::Cycle.message()),
    (
        "E_TELEPORT_VISIBLE",
        Kind::Rejected,
        Reject::TeleportVisible.message(),
    ),
    (
        "E_TELEPORT_BLOCKED",
        Kind::Rejected,
        "the collision module could not push a teleported actor out of what it landed in; it stayed where it was",
    ),
    (
        "E_COLLIDER_BLOCKED",
        Kind::Rejected,
        "a collider was enabled where an actor stood and the actor could not be pushed out; the collider stays off",
    ),
    (
        "E_AUDIO_UNKNOWN_SOUND",
        Kind::Rejected,
        "a sound or music id names no sound in the game's bank",
    ),
    (
        "E_AUDIO_NOT_LOADED",
        Kind::Rejected,
        "a sound could not be loaded to play",
    ),
    (
        "E_AUDIO_QUEUE_FULL",
        Kind::Rejected,
        "more sound commands in a step than the mixer's queue holds",
    ),
    (
        "E_AUDIO_NOT_FINITE",
        Kind::Rejected,
        "a sound's volume or a bus volume is not a finite number",
    ),
    (
        "E_AUDIO_DECODE",
        Kind::Rejected,
        "a sound in the bank could not be decoded",
    ),
    (
        "E_UI_UNKNOWN",
        Kind::Rejected,
        "a ui_* command names a HUD node or animation the HUD does not have",
    ),
    (
        "E_UI_FONT",
        Kind::Rejected,
        "a HUD text could not be shaped with its font",
    ),
    (
        "E_UI_ATLAS_FULL",
        Kind::Rejected,
        "the HUD's glyph atlas has no room for a text's glyphs",
    ),
    (
        "E_UI_ANIMATIONS",
        Kind::Rejected,
        "more HUD animations at once than the HUD's settings allow",
    ),
    ("E_NOT_ACTOR", Kind::Rejected, Reject::NotActor.message()),
    (
        "E_RIG_UNKNOWN",
        Kind::Rejected,
        "camera_use names a rig never defined",
    ),
    (
        "E_CUTSCENE_BUSY",
        Kind::Rejected,
        "cutscene_play while a cutscene plays",
    ),
    (
        "E_CUTSCENE_UNKNOWN",
        Kind::Rejected,
        "cutscene_play names a cutscene the pack lacks",
    ),
    (
        "E_CUTSCENE_UNBOUND",
        Kind::Rejected,
        "cutscene_play before every role without a spawn was bound to a live entity",
    ),
];

fn message(spec: &MessageSpec) -> Value {
    let fields: Vec<Value> = spec
        .fields
        .iter()
        .map(|f| json!({ "name": f.name, "type": f.ty, "doc": f.doc.trim() }))
        .collect();
    json!({ "opcode": spec.opcode, "name": spec.name, "doc": spec.doc.trim(), "fields": fields })
}

fn enumeration(spec: &EnumSpec) -> Value {
    let variants: Vec<Value> = spec
        .variants
        .iter()
        .map(|v| json!({ "value": v.value, "name": v.name, "doc": v.doc.trim() }))
        .collect();
    json!({ "name": spec.name, "doc": spec.doc.trim(), "variants": variants })
}

/// Every leaf of the default settings, by dotted key, with its doc.
fn settings() -> Vec<Value> {
    fn leaves(prefix: &str, value: &Value, out: &mut Vec<(String, Value)>) {
        match value {
            Value::Object(fields) => {
                for (k, v) in fields {
                    let key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    leaves(&key, v, out);
                }
            }
            leaf => out.push((prefix.to_string(), leaf.clone())),
        }
    }
    // Every float setting is an f32; print the shortest decimal that reads back as it.
    fn shorten(value: &mut Value) {
        match value {
            Value::Number(n) if n.is_f64() => {
                let short = (n.as_f64().unwrap_or_default() as f32).to_string();
                *value = serde_json::from_str(&short).unwrap_or(Value::Null);
            }
            Value::Array(items) => items.iter_mut().for_each(shorten),
            Value::Object(fields) => fields.values_mut().for_each(shorten),
            _ => {}
        }
    }
    let mut defaults = serde_json::to_value(Settings::default()).expect("settings serialize");
    shorten(&mut defaults);
    let mut out = Vec::new();
    leaves("", &defaults, &mut out);
    out.into_iter()
        .map(|(key, default)| {
            let doc = DOCS.iter().find(|(k, _)| *k == key).map_or("", |(_, d)| *d);
            json!({ "key": key, "default": default, "doc": doc })
        })
        .collect()
}

/// The module boundary: what a game exports and imports.
fn module() -> Value {
    json!({
        "exports": [
            { "name": "sariema_tick", "signature": "() -> ()", "doc": "Called once per step: read events, submit commands." },
            { "name": "sariema_batch_version", "signature": "() -> i32", "doc": "Optional; the batch version the logic was built against." },
            { "name": "memory", "signature": "memory", "doc": "The linear memory." },
        ],
        "imports": [
            { "module": "sariema", "name": "submit", "signature": "(ptr: i32, len: i32) -> ()", "doc": "Appends len bytes of commands." },
            { "module": "sariema", "name": "events", "signature": "(dst: i32, cap: i32) -> i32", "doc": "Copies the next whole events of this step: bytes copied, 0 when all were read, minus the size of an event larger than cap." },
            { "module": "sariema", "name": "register", "signature": "(kind: i32, ptr: i32, len: i32) -> i32", "doc": "A bulk region the engine reads after every tick; its id or -1. Kind 1 is transforms." },
            { "module": "sariema", "name": "log", "signature": "(level: i32, ptr: i32, len: i32) -> ()", "doc": "A line of UTF-8 text; level 0 debug to 3 error." },
        ],
    })
}

pub fn api() -> Result<Done, Failure> {
    let commands: Vec<Value> = COMMANDS.iter().map(message).collect();
    let events: Vec<Value> = EVENTS.iter().map(message).collect();
    let enums: Vec<Value> = ENUMS.iter().map(enumeration).collect();
    let settings = settings();
    let errors: Vec<Value> = ERRORS
        .iter()
        .map(|(code, kind, meaning)| json!({ "code": code, "kind": kind.name(), "meaning": meaning }))
        .collect();
    Ok(Done {
        text: format!(
            "batch version {BATCH_VERSION}: {} commands, {} events, {} enums, {} settings, {} error codes; --json prints them all",
            commands.len(),
            events.len(),
            enums.len(),
            settings.len(),
            errors.len()
        ),
        json: json!({
            "batch_version": BATCH_VERSION,
            "module": module(),
            "commands": commands,
            "events": events,
            "enums": enums,
            "settings": settings,
            "errors": errors,
        }),
    })
}
