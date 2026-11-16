// SPDX-License-Identifier: Apache-2.0

//! `replay(pack, log, steps)` runs a game pack in the engine with an input
//! log, as `sariema run --headless --game` does, and leaves its hashes in
//! [`HASHES`]: engine, runtime state, collision world, mixed sound.

use sariema_engine::{Engine, EngineSettings};
use sariema_format::script::decode_game;
use sariema_runtime::input::{InputLog, Replay};

static mut HASHES: [u64; 4] = [0; 4];

/// A buffer of `len` bytes for the host to write a file into; never freed.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    Box::leak(vec![0u8; len].into_boxed_slice()).as_mut_ptr()
}

/// Where the four hashes are, after [`replay`] returned 0.
#[unsafe(no_mangle)]
pub extern "C" fn hashes() -> *const u64 {
    &raw const HASHES as *const u64
}

/// 0 when it ran; 1 when the pack, 2 when the log could not be read.
///
/// # Safety
/// `pack` and `log` point at `pack_len` and `log_len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn replay(
    pack: *const u8,
    pack_len: usize,
    log: *const u8,
    log_len: usize,
    steps: u64,
) -> u32 {
    // SAFETY: the caller's contract.
    let (pack, log) = unsafe {
        (
            std::slice::from_raw_parts(pack, pack_len),
            std::slice::from_raw_parts(log, log_len),
        )
    };
    let Ok(pack) = decode_game(pack) else {
        return 1;
    };
    let Ok(mut engine) = Engine::load(pack, 0, EngineSettings::default()) else {
        return 1;
    };
    let Ok(log) = serde_json::from_slice::<InputLog>(log) else {
        return 2;
    };
    let Ok(mut replay) = Replay::new(&log, &engine.state().settings.input) else {
        return 2;
    };
    for step in 0..steps {
        engine.step(replay.actions(step));
        engine.feed_audio();
    }
    let hashes = [
        engine.state_hash(),
        engine.simulation().state_hash(),
        engine.physics().map_or(0, |p| p.world.state_hash()),
        engine.sound().map_or(0, |s| s.hash()),
    ];
    // SAFETY: wasm32-unknown-unknown runs one thread.
    unsafe { HASHES = hashes };
    0
}
