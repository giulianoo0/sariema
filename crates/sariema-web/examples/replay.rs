// SPDX-License-Identifier: Apache-2.0

//! Replays a game pack with an input log natively, as the page's
//! `replayGame` does (each step, then the audio feed), and times it:
//!
//!   cargo run -p sariema-web --example replay --release -- <pack.srmg> <log.json> <steps> [reps]
//!
//! Prints the hashes `sariema run --headless --game` prints and, per rep,
//! the mean milliseconds a step took. A game that stops ends the replay at
//! that step, which is printed with the stop's code.

use std::time::Instant;

use sariema_engine::{Engine, EngineSettings};
use sariema_format::script::decode_game;
use sariema_runtime::input::{InputLog, Replay};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [pack, log, steps, rest @ ..] = args.as_slice() else {
        panic!("usage: replay <pack.srmg> <log.json> <steps> [reps]");
    };
    let bytes = std::fs::read(pack).unwrap();
    let log: InputLog = serde_json::from_slice(&std::fs::read(log).unwrap()).unwrap();
    let steps: u64 = steps.parse().unwrap();
    let reps: usize = rest.first().map_or(1, |r| r.parse().unwrap());
    let mut per_step_ms = Vec::new();
    let (mut hashes, mut stopped) = (json!(null), json!(null));
    for _ in 0..reps {
        let mut engine =
            Engine::load(decode_game(&bytes).unwrap(), 0, EngineSettings::default()).unwrap();
        let mut replay = Replay::new(&log, &engine.state().settings.input).unwrap();
        let start = Instant::now();
        for step in 0..steps {
            engine.step(replay.actions(step));
            engine.feed_audio();
            if let Some(stop) = engine.simulation().stopped() {
                stopped = json!({ "code": stop.code, "step": step });
                break;
            }
        }
        per_step_ms.push(start.elapsed().as_secs_f64() * 1000.0 / steps as f64);
        hashes = engine.hashes();
    }
    println!(
        "{}",
        json!({ "hashes": hashes, "stopped": stopped, "per_step_ms": per_step_ms })
    );
}
