// SPDX-License-Identifier: Apache-2.0

//! Records a game's ticks (each step's event batch) over an input log,
//! natively, and times them under native wasmi; writes beside them the
//! game's logic module and buttons, a synthetic tick of float and memory
//! work, and a tick that never returns:
//!
//!   record <pack.srmg> <log.json> <steps> <out dir>
//!
//! Prints the ticks' bytes and native wasmi's milliseconds per tick.

use std::path::Path;
use std::time::Instant;

use sariema_engine::{Assets, Engine, EngineSettings};
use sariema_format::script::{decode_game, name_id};
use sariema_runtime::game::{WasmLogic, pack_settings};
use sariema_runtime::input::{InputLog, Replay};
use sariema_runtime::sim::{Logic, Simulation, Stop};
use sariema_vm::Vm;
use vm_bench::Runner;

struct Recorder {
    inner: WasmLogic,
    out: Vec<u8>,
}

impl Logic for Recorder {
    fn tick(&mut self, events: &[u8], ends: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        self.out.extend((events.len() as u32).to_le_bytes());
        self.out.extend(events);
        self.out.extend((ends.len() as u32).to_le_bytes());
        for e in ends {
            self.out.extend(e.to_le_bytes());
        }
        self.inner.tick(events, ends, commands)
    }

    fn regions(&self, each: &mut dyn FnMut(u32, &[u8])) {
        self.inner.regions(each)
    }

    fn memory(&self) -> Option<&[u8]> {
        self.inner.memory()
    }
}

/// 20,000 rounds over 1024 floats.
const SYNTHETIC: &str = r#"(module
  (memory (export "memory") 1)
  (func (export "sariema_tick") (local $i i32) (local $a i32) (local $x f32)
    (local.set $i (i32.const 0))
    (loop $l
      (local.set $a (i32.shl (i32.and (local.get $i) (i32.const 1023)) (i32.const 2)))
      (local.set $x (f32.load (local.get $a)))
      (local.set $x (f32.add (f32.mul (local.get $x) (f32.const 0.999)) (f32.const 0.5)))
      (f32.store (local.get $a) (f32.sqrt (f32.mul (local.get $x) (local.get $x))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $l (i32.lt_u (local.get $i) (i32.const 20000))))))"#;

const FOREVER: &str =
    r#"(module (memory (export "memory") 1) (func (export "sariema_tick") (loop $l (br $l))))"#;

fn write(out: &Path, name: &str, bytes: &[u8]) {
    std::fs::write(out.join(name), bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [pack, log, steps, out] = args.as_slice() else {
        panic!("usage: record <pack.srmg> <log.json> <steps> <out dir>");
    };
    let steps: u64 = steps.parse().expect("steps is a number");
    let out = Path::new(out);
    std::fs::create_dir_all(out).unwrap();
    let mut pack = decode_game(&std::fs::read(pack).unwrap()).unwrap();
    let assets = Assets::decode(std::mem::take(&mut pack.assets)).unwrap();
    let mut settings = pack_settings(&pack).unwrap();
    if assets.collision.is_some() {
        settings.motion.builtin = false;
    }
    let buttons: Vec<u32> = (settings.input.buttons.iter())
        .map(|b| name_id(&b.name))
        .collect();
    let mut vm = Vm::load(&pack.wasm, settings.vm).unwrap();
    vm.set_buttons(buttons.iter().copied());
    let wasm = pack.wasm.clone();
    let recorder = Recorder {
        inner: WasmLogic::new(vm),
        out: Vec::new(),
    };
    let sim = Simulation::new(settings, pack.cutscenes, recorder, 0);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let log: InputLog = serde_json::from_slice(&std::fs::read(log).unwrap()).unwrap();
    let mut replay = Replay::new(&log, &engine.state().settings.input).unwrap();
    for step in 0..steps {
        engine.step(replay.actions(step));
        engine.feed_audio();
    }
    let ticks = engine.simulation().logic().out.clone();
    write(out, "climb.wasm", &wasm);
    write(out, "climb.ticks", &ticks);
    write(
        out,
        "climb.buttons.json",
        serde_json::to_string(&buttons).unwrap().as_bytes(),
    );
    let synthetic = wat::parse_str(SYNTHETIC).unwrap();
    let idle: Vec<u8> = (0..60).flat_map(|_| [0u8; 8]).collect();
    write(out, "synthetic.wasm", &synthetic);
    write(out, "synthetic.ticks", &idle);
    write(out, "synthetic.buttons.json", b"[]");
    write(out, "forever.wasm", &wat::parse_str(FOREVER).unwrap());

    let mut native = serde_json::Map::new();
    for (name, wasm, ticks, buttons, copies) in [
        ("climb", &wasm, &ticks, &buttons, 10usize),
        ("synthetic", &synthetic, &idle, &Vec::new(), 1),
    ] {
        let mut runs = Vec::new();
        for _ in 0..7 {
            let mut r = Runner::new(wasm, buttons, ticks, copies).unwrap();
            let n = r.ticks() * copies;
            let t = Instant::now();
            r.run().unwrap();
            runs.push(t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }
        native.insert(name.into(), serde_json::json!(runs));
    }
    println!(
        "{}",
        serde_json::json!({ "ticks_bytes": ticks.len(), "native_wasmi_ms_per_tick": native })
    );
}
