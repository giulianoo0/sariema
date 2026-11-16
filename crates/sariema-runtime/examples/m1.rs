// SPDX-License-Identifier: Apache-2.0

//! The M1 load case, measured: a game of 2,000 objects deciding every step
//! (`benches/swarm`), on each interpreter, placing its objects by one
//! command each or through a bulk region; then the native systems driving
//! 2,000 tweens. Prints one JSON object.
//!
//! ```text
//! sariema build crates/sariema-runtime/benches/swarm -o swarm.srmg
//! cargo run --release -p sariema-runtime --features tinywasm --example m1 -- swarm.srmg
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use sariema_format::script::{
    Command, Ease, Entity, GamePack, Kind, Property, decode_game, records,
};
use sariema_runtime::game::pack_settings;
use sariema_runtime::input::Actions;
use sariema_runtime::outbox::HostIn;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};
use sariema_vm::{Interpreter, Tinywasm, Vm, Wasmi};
use serde_json::{Value, json};

struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call forwards to the system allocator unchanged; the
// counters are only statistics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: same layout contract as the caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `ptr` came from `alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LIVE.fetch_add(new_size, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `ptr` came from `alloc` with this layout.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const WARM_UP: u64 = 120;
const MEASURED: u64 = 600;

/// The game's logic, timing each tick and keeping one batch.
struct Timed<L> {
    inner: L,
    spent: Vec<Duration>,
    keep: bool,
    kept: Vec<u8>,
}

impl<L: Logic> Logic for Timed<L> {
    fn tick(&mut self, events: &[u8], ends: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        let started = Instant::now();
        let result = self.inner.tick(events, ends, commands);
        self.spent.push(started.elapsed());
        if std::mem::take(&mut self.keep) {
            self.kept = commands.clone();
        }
        result
    }

    fn regions(&self, each: &mut dyn FnMut(u32, &[u8])) {
        self.inner.regions(each)
    }

    fn memory(&self) -> Option<&[u8]> {
        self.inner.memory()
    }
}

struct Wasm(Vm);

impl Logic for Wasm {
    fn tick(&mut self, events: &[u8], ends: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        self.0.tick(events, ends, commands).map_err(|e| Stop {
            code: e.code(),
            message: e.to_string(),
        })
    }

    fn regions(&self, each: &mut dyn FnMut(u32, &[u8])) {
        for (region, bytes) in self.0.regions() {
            each(region.kind, bytes);
        }
    }

    fn memory(&self) -> Option<&[u8]> {
        Some(self.0.memory())
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn summary(mut times: Vec<Duration>) -> Value {
    times.sort();
    let at = |q: f64| ms(times[((times.len() - 1) as f64 * q) as usize]);
    let mean = ms(times.iter().sum::<Duration>()) / times.len() as f64;
    json!({ "mean_ms": mean, "median_ms": at(0.5), "p95_ms": at(0.95), "max_ms": at(1.0) })
}

fn simulation<I: Interpreter>(
    pack: &GamePack,
    settings: &Settings,
    region: bool,
) -> Simulation<Timed<Wasm>> {
    let vm = Vm::load_with::<I>(&pack.wasm, settings.vm).expect("the swarm loads");
    let logic = Timed {
        inner: Wasm(vm),
        spent: Vec::new(),
        keep: false,
        kept: Vec::new(),
    };
    let mut sim = Simulation::new(settings.clone(), pack.cutscenes.clone(), logic, 7);
    if region {
        sim.deliver(HostIn::Message {
            topic: "mode".into(),
            payload: vec![1],
        });
    }
    sim
}

fn measure<I: Interpreter>(pack: &GamePack, settings: &Settings, region: bool) -> Value {
    let actions = Actions::for_settings(&settings.input);
    let mut sim = simulation::<I>(pack, settings, region);
    for _ in 0..WARM_UP {
        sim.step(&actions);
    }
    assert!(sim.stopped().is_none(), "{:?}", sim.stopped());
    let live_before = LIVE.load(Ordering::Relaxed);
    let allocations_before = ALLOCATIONS.load(Ordering::Relaxed);
    let mut steps = Vec::new();
    for _ in 0..MEASURED {
        let started = Instant::now();
        sim.step(&actions);
        steps.push(started.elapsed());
    }
    let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations_before;
    let live_after = LIVE.load(Ordering::Relaxed);
    let logic: Vec<Duration> = sim.logic().spent[WARM_UP as usize..].to_vec();
    json!({
        "interpreter": I::NAME,
        "placement": if region { "bulk region" } else { "one set_transform each" },
        "logic_tick": summary(logic),
        "whole_step": summary(steps),
        "allocations_per_step": allocations as f64 / MEASURED as f64,
        "host_heap_growth_bytes": live_after as i64 - live_before as i64,
        "wasm_memory_bytes": sim.logic().inner.0.memory().len(),
    })
}

fn decode_cost(pack: &GamePack, settings: &Settings) -> Value {
    let actions = Actions::for_settings(&settings.input);
    let mut sim = simulation::<Wasmi>(pack, settings, false);
    for step in 0..WARM_UP {
        sim.step(&actions);
        if step == WARM_UP - 2 {
            sim.logic_mut().keep = true;
        }
    }
    let batch = sim.logic().kept.clone();
    let rounds = 1000;
    let started = Instant::now();
    let mut records_seen = 0;
    for _ in 0..rounds {
        records_seen += records::<Command>(std::hint::black_box(&batch))
            .filter(|r| r.is_ok())
            .count();
    }
    let per_batch = started.elapsed() / rounds;
    json!({
        "batch_bytes": batch.len(),
        "records": records_seen / rounds as usize,
        "decode_ms": ms(per_batch),
    })
}

/// The smallest steady-state budget the swarm runs within, by bisection.
fn fuel_needed<I: Interpreter>(pack: &GamePack, settings: &Settings) -> u64 {
    let actions = Actions::for_settings(&settings.input);
    let runs = |fuel: u64| {
        let mut s = settings.clone();
        s.vm.fuel_per_step = fuel;
        let mut sim = simulation::<I>(pack, &s, false);
        for _ in 0..60 {
            sim.step(&actions);
        }
        sim.stopped().is_none()
    };
    let (mut low, mut high) = (1_000, settings.vm.fuel_per_step);
    assert!(runs(high), "the default budget is too small for the swarm");
    while high - low > high / 100 {
        let mid = (low + high) / 2;
        if runs(mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
    high
}

/// A native game that sets 2,000 long tweens going on its first step.
struct Tweener;

impl Logic for Tweener {
    fn tick(&mut self, events: &[u8], _: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        commands.clear();
        let first = records::<sariema_format::script::Event>(events)
            .flatten()
            .any(|e| matches!(e, sariema_format::script::Event::Started { .. }));
        if first {
            for i in 0..2_000 {
                let entity = Entity {
                    index: i,
                    generation: 0,
                };
                Command::Spawn {
                    entity,
                    kind: Kind::Empty,
                    asset: 0,
                }
                .write(commands);
                Command::Tween {
                    tween: i,
                    entity,
                    property: Property::PositionX,
                    var: 0,
                    to: 100.0,
                    duration: 1_000.0,
                    ease: Ease::InOutCubic,
                }
                .write(commands);
            }
        }
        Ok(())
    }
}

fn tweens() -> Value {
    let settings = Settings::default();
    let actions = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, Vec::new(), Tweener, 1);
    for _ in 0..WARM_UP {
        sim.step(&actions);
    }
    let allocations_before = ALLOCATIONS.load(Ordering::Relaxed);
    let mut steps = Vec::new();
    for _ in 0..MEASURED {
        let started = Instant::now();
        sim.step(&actions);
        steps.push(started.elapsed());
    }
    let allocations = ALLOCATIONS.load(Ordering::Relaxed) - allocations_before;
    json!({
        "tweens": 2_000,
        "whole_step": summary(steps),
        "allocations_per_step": allocations as f64 / MEASURED as f64,
    })
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: m1 <swarm.srmg>");
    let pack = decode_game(&std::fs::read(&path).expect("read the pack")).expect("a game pack");
    let settings = pack_settings(&pack).expect("settings");
    let runs = [
        measure::<Wasmi>(&pack, &settings, false),
        measure::<Wasmi>(&pack, &settings, true),
        measure::<Tinywasm>(&pack, &settings, false),
        measure::<Tinywasm>(&pack, &settings, true),
    ];
    let fuel = json!({
        "wasmi": fuel_needed::<Wasmi>(&pack, &settings),
        "tinywasm": fuel_needed::<Tinywasm>(&pack, &settings),
    });
    let report = json!({
        "objects": 2_000,
        "warm_up_steps": WARM_UP,
        "measured_steps": MEASURED,
        "runs": runs,
        "batch_decode": decode_cost(&pack, &settings),
        "fuel_needed_per_step": fuel,
        "fuel_per_step_default": settings.vm.fuel_per_step,
        "native": tweens(),
    });
    println!("{}", serde_json::to_string_pretty(&report).expect("JSON"));
}
