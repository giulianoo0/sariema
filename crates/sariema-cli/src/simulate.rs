// SPDX-License-Identifier: Apache-2.0

//! `run --headless --game`: a game pack in the whole engine, fed a recorded
//! input log, reporting what happened and the state it ended in. Sound is
//! mixed with the steps and hashed; frames are drawn only for a screenshot.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::{Args, value_parser};
use sariema_engine::{Engine, EngineSettings};
use sariema_format::script::{Event, Value as Field, decode_game, name_id, records};
use sariema_runtime::input::{InputLog, Replay};
use sariema_runtime::outbox::{HostIn, HostOut};
use serde_json::{Map, Value, json};

use crate::RunArgs;
use crate::game_frames::Frames;
use crate::output::{Done, Failure};

const INPUT_LOG_SCHEMA: &str = include_str!("../../../schemas/input-log.schema.json");

#[derive(Args)]
pub struct GameRun {
    /// Game pack (.srmg) to play in a window, or with --headless and --steps
    /// to run without drawing.
    #[arg(long, conflicts_with_all = ["pack", "max_background"])]
    pub game: Option<PathBuf>,
    /// Recorded input log to replay (schemas/input-log.schema.json).
    #[arg(long, requires_all = ["game", "headless"])]
    pub input: Option<PathBuf>,
    /// Steps to simulate, 60 per second of game time.
    #[arg(long, requires_all = ["game", "headless"], value_parser = value_parser!(u64).range(1..))]
    pub steps: Option<u64>,
    /// Seed of the simulation's random numbers.
    #[arg(long, requires = "game")]
    pub seed: Option<u64>,
    /// JSON object of saved keys to byte arrays: the save store at the start.
    #[arg(long, requires = "game")]
    pub saves: Option<PathBuf>,
}

fn read_json(path: &Path) -> Result<Value, Failure> {
    let text = fs::read_to_string(path).map_err(|e| Failure::io(path, e))?;
    serde_json::from_str(&text)
        .map_err(|e| Failure::new("E_SCHEMA", format!("{}: {e}", path.display())))
}

pub fn read_log(path: &Path) -> Result<InputLog, Failure> {
    let value = read_json(path)?;
    let schema: Value =
        serde_json::from_str(INPUT_LOG_SCHEMA).expect("input-log.schema.json is JSON");
    let fail = |why: String| Failure::new("E_SCHEMA", format!("{}: {why}", path.display()));
    jsonschema::validate(&schema, &value)
        .map_err(|e| fail(format!("{}: {e}", e.instance_path())))?;
    serde_json::from_value(value).map_err(|e| fail(e.to_string()))
}

fn read_saves(path: &Path) -> Result<BTreeMap<String, Vec<u8>>, Failure> {
    serde_json::from_value(read_json(path)?).map_err(|e| {
        Failure::new(
            "E_SCHEMA",
            format!(
                "{}: saves are an object of keys to byte arrays: {e}",
                path.display()
            ),
        )
    })
}

fn field(value: Field<'_>) -> Value {
    match value {
        Field::U64(v) => json!(v),
        Field::I64(v) => json!(v),
        Field::F32(v) => json!(v),
        Field::Bool(v) => json!(v),
        Field::Entity(e) if e.is_none() => Value::Null,
        Field::Entity(e) => json!({ "index": e.index, "generation": e.generation }),
        Field::Vec3(v) => json!(v),
        Field::Str(s) => json!(s),
        Field::Bytes(b) => json!(b),
        Field::Points(p) => json!(p.iter().collect::<Vec<_>>()),
        Field::Enum(name) => json!(name),
    }
}

fn event_json(step: u64, event: &Event<'_>) -> Value {
    let mut object = Map::new();
    object.insert("step".into(), json!(step));
    object.insert("event".into(), json!(event.name()));
    event.fields(|name, value| {
        object.insert(name.into(), field(value));
    });
    Value::Object(object)
}

fn host_json(step: u64, message: &HostOut) -> Value {
    let (kind, extra) = match message {
        HostOut::Ready => ("ready", json!({})),
        HostOut::Exit => ("exit", json!({})),
        HostOut::Orientation(o) => ("orientation", json!({ "orientation": o.name() })),
        HostOut::Save { key, value } => ("save", json!({ "key": key, "value": value })),
        HostOut::LoadSaves => ("load_saves", json!({})),
        HostOut::Message { topic, payload } => {
            ("message", json!({ "topic": topic, "payload": payload }))
        }
        HostOut::Stopped { code, message } => {
            ("stopped", json!({ "code": code, "message": message }))
        }
    };
    let mut object = json!({ "step": step, "message": kind });
    if let (Value::Object(o), Value::Object(extra)) = (&mut object, extra) {
        o.extend(extra);
    }
    object
}

fn hex(hash: u64) -> String {
    format!("{hash:016x}")
}

pub fn run(path: &Path, run: &RunArgs) -> Result<Done, Failure> {
    let args = &run.game;
    let steps = args
        .steps
        .expect("run checks --steps before a headless game");
    let bytes = fs::read(path).map_err(|e| Failure::io(path, e))?;
    let pack = decode_game(&bytes)
        .map_err(|e| Failure::new(e.code(), format!("{}: {e}", path.display())))?;
    let names: BTreeMap<String, &String> = pack
        .names
        .iter()
        .map(|n| (name_id(n).to_string(), n))
        .collect();
    let names = json!(names);
    let assets = run.screenshot.is_some().then(|| pack.assets.clone());
    let mut engine = Engine::load(pack, args.seed.unwrap_or(0), EngineSettings::default())
        .map_err(|e| Failure::new(e.code, format!("{}: {e}", path.display())))?;
    let log = match &args.input {
        Some(input) => read_log(input)?,
        None => InputLog::default(),
    };
    let mut replay = Replay::new(&log, &engine.state().settings.input).map_err(|e| {
        Failure::new(
            e.code(),
            format!("{}: {e}", args.input.as_deref().unwrap_or(path).display()),
        )
    })?;
    let mut saves = match &args.saves {
        Some(saves) => read_saves(saves)?,
        None => BTreeMap::new(),
    };
    let mut frames = Frames::new(run, assets.as_ref(), &mut engine, steps)?;

    let (mut events, mut host, mut logs) = (Vec::new(), Vec::new(), Vec::new());
    let mut took = Vec::with_capacity(steps as usize);
    let mut fed = Duration::ZERO;
    for step in 0..steps {
        let started = Instant::now();
        engine.step(replay.actions(step));
        took.push(started.elapsed());
        let started = Instant::now();
        engine.feed_audio();
        fed += started.elapsed();
        if let Some(frames) = &mut frames {
            frames.after_step(step, &mut engine)?;
        }
        let sim = engine.simulation_mut();
        for event in records::<Event>(sim.delivered().bytes()).flatten() {
            events.push(event_json(step, &event));
        }
        for line in sim.logic().log() {
            logs.push(json!({ "step": step, "level": line.level, "text": line.text }));
        }
        for message in sim.take_host() {
            host.push(host_json(step, &message));
            match message {
                HostOut::Save { key, value } => {
                    saves.insert(key, value);
                }
                HostOut::LoadSaves => sim.deliver(HostIn::Saves(
                    saves.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                )),
                _ => {}
            }
        }
        if let Some(stop) = sim.stopped() {
            let last_error = logs
                .iter()
                .rev()
                .find(|l| l["level"] == 3)
                .and_then(|l| l["text"].as_str())
                .map(|t| format!("; it logged: {t}"))
                .unwrap_or_default();
            return Err(Failure::new(
                stop.code,
                format!(
                    "{}: step {step}: {}{last_error}",
                    path.display(),
                    stop.message
                ),
            ));
        }
    }
    let frames = frames.map(Frames::finish).transpose()?;
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let total: Duration = took.iter().sum();
    took.sort_unstable();
    let (median, slowest) = (took[took.len() / 2], took[took.len() - 1]);
    let engine_hash = hex(engine.state_hash());
    let physics_hash = engine.physics().map(|p| hex(p.world.state_hash()));
    let audio_hash = engine.sound().map(|s| hex(s.hash()));
    let sim = engine.simulation();
    let state_hash = hex(sim.state_hash());
    let world_hash = hex(sim.state().world_hash());
    let fx_hash = hex(sim.state().fx.hash());
    let memory = sim.logic().vm().memory().len();
    let playing = sim.playing_cutscene();
    Ok(Done {
        json: json!({
            "steps": steps,
            "engine_hash": engine_hash,
            "state_hash": state_hash,
            "physics_hash": physics_hash,
            "audio_hash": audio_hash,
            "world_hash": world_hash,
            "fx_hash": fx_hash,
            "events": events,
            "host": host,
            "log": logs,
            "saves": saves,
            "playing_cutscene": playing,
            "names": names,
            "memory_bytes": memory,
            "interpreter": sim.logic().vm().interpreter(),
            "frames": frames,
            "timing": {
                "total_ms": ms(total),
                "mean_step_ms": ms(total) / steps as f64,
                "median_step_ms": ms(median),
                "max_step_ms": ms(slowest),
                "mean_feed_ms": ms(fed) / steps as f64,
            },
        }),
        text: format!(
            "{steps} steps: engine {engine_hash}, state {state_hash}, world {world_hash}, {} events, {} host messages, {:.3} ms per step (median {:.3}, max {:.3})",
            events.len(),
            host.len(),
            ms(total) / steps as f64,
            ms(median),
            ms(slowest)
        ),
    })
}
