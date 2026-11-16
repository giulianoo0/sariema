// SPDX-License-Identifier: Apache-2.0

//! The mix path must not allocate or free: an allocator that counts calls
//! made on this thread while armed.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::*;
use sariema_audio::{MixerSettings, MusicFades, PlayParams, StreamSettings};

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<u64> = const { Cell::new(0) };
}

fn note() {
    if ARMED.with(Cell::get) {
        CALLS.with(|c| c.set(c.get() + 1));
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn armed<R>(f: impl FnOnce() -> R) -> (R, u64) {
    CALLS.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
    let r = f();
    ARMED.with(|a| a.set(false));
    (r, CALLS.with(Cell::get))
}

#[test]
fn mixing_allocates_and_frees_nothing() {
    let settings = MixerSettings {
        voices: 4,
        stream: StreamSettings {
            buffer_s: 0.2,
            ..StreamSettings::default()
        },
        ..MixerSettings::default()
    };
    let defs = vec![
        Def::mono("a", MUSIC, sine(220.0, 0.3, RATE, 48_000)).streamed(),
        Def::mono("b", MUSIC, sine(330.0, 0.3, RATE, 48_000)).streamed(),
        Def::mono("line", VOICE, sine(600.0, 0.3, RATE, 20_000)),
        Def::mono("step", EFFECTS, sine(900.0, 0.3, RATE, 3_000)),
        Def::mono("wind", AMBIENCE, sine(80.0, 0.3, RATE, 9_000)).looped(0, 9_000),
    ];
    let (mut audio, mut mixer) = setup(settings, defs);
    let id = |name| audio.sound(name).unwrap();
    let (a, b, line, step, wind) = (id("a"), id("b"), id("line"), id("step"), id("wind"));
    let quick = MusicFades {
        fade_in_s: Some(0.1),
        fade_out_s: Some(0.1),
        curve: None,
    };
    audio.music(0, a, quick, None).unwrap();
    let w = audio.play(0, wind, PlayParams::default()).unwrap();
    audio.play(2_000, line, PlayParams::default()).unwrap();
    for i in 0..12 {
        audio
            .play(3_000 + i * 500, step, PlayParams::default())
            .unwrap();
    }
    audio.music(12_000, b, quick, None).unwrap();
    audio.stinger(20_000, line).unwrap();
    audio.set_bus(21_000, AMBIENCE, -6.0, 0.2).unwrap();
    audio.pause(30_000).unwrap();
    audio.resume(32_000).unwrap();
    audio.stop(36_000, w, 0.5).unwrap();
    let mut out = vec![0.0; 2 * 480];
    let mut calls = 0;
    // The streams are fed once and then starve: underruns are in the path too.
    for _ in 0..200 {
        calls += armed(|| mixer.render(&mut out)).1;
    }
    let stats = mixer.stats();
    assert_eq!(calls, 0);
    assert!(stats.steals > 0 && stats.underruns > 0, "{stats:?}");
    assert!(stats.now >= 96_000);
}
