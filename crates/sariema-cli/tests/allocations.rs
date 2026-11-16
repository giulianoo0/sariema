// SPDX-License-Identifier: Apache-2.0

//! The climb's allocations, step by step, over its whole input log and
//! as long again with no input: every step that allocates is pinned with
//! its count, and so is every second of the audio feed between steps.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use sariema_engine::{Engine, EngineSettings};
use sariema_format::script::decode_game;
use sariema_runtime::input::{InputLog, Replay};
use tempfile::TempDir;

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn count() {
    if ARMED.with(Cell::get) {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
    }
}

// SAFETY: every call is passed on to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn counted(f: impl FnOnce()) -> u64 {
    let before = ALLOCATIONS.with(Cell::get);
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    ALLOCATIONS.with(Cell::get) - before
}

const STEPS: u64 = 1200;

/// The pins, shared with the web package's browser tests: every step of
/// the climb that allocates, with its count (the others allocate nothing),
/// and the feed's allocations in each second.
///
/// Step 0 spawns the hero, whose walkable grid the world built at load; 186
/// starts the cutscene, which plans the hero's walk; 340 shows its line, which
/// lays the HUD out again. The rest grow a step buffer to a size it had not
/// reached. The feed's first and sixth seconds open the music stream and
/// decode the sounds loaded on demand; in every second the music decoder
/// allocates per packet. `docs/engine.md` lists them by site.
const PINS: &str = include_str!("climb-allocations.json");

#[derive(serde::Deserialize)]
struct Pins {
    steps: Vec<(u64, u64)>,
    feed_per_second: Vec<u64>,
}

#[test]
fn the_climbs_steps_allocate_only_where_pinned() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let climb = repo.join("examples/climb");
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("climb.srmg");
    let built = cargo_bin_cmd!("sariema")
        .args([
            "build",
            climb.to_str().unwrap(),
            "-o",
            pack.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    let pack = decode_game(&std::fs::read(&pack).unwrap()).unwrap();
    let log: InputLog =
        serde_json::from_slice(&std::fs::read(climb.join("input/play.json")).unwrap()).unwrap();
    let mut engine = Engine::load(pack, 0, EngineSettings::default()).unwrap();
    let mut replay = Replay::new(&log, &engine.state().settings.input).unwrap();
    let mut steps = Vec::new();
    let mut feeds = Vec::new();
    for step in 0..STEPS {
        let actions = replay.actions(step);
        let n = counted(|| engine.step(actions));
        if n > 0 {
            steps.push((step, n));
        }
        feeds.push(counted(|| engine.feed_audio()));
    }
    assert_eq!(format!("{:016x}", engine.state_hash()), ENGINE_HASH_1200);
    let seconds: Vec<u64> = feeds.chunks(60).map(|s| s.iter().sum()).collect();
    let pins: Pins = serde_json::from_str(PINS).unwrap();
    let total: u64 = steps.iter().map(|s| s.1).sum();
    assert_eq!(steps, pins.steps, "{total} allocations");
    assert_eq!(seconds, pins.feed_per_second);
}

const ENGINE_HASH_1200: &str = "b251af53b3ba0fbe";
