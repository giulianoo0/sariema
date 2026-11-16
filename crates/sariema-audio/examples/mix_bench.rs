// SPDX-License-Identifier: Apache-2.0

//! CPU time to mix one second at 48 kHz stereo with 4, 32 and 128 voices,
//! and the memory a streamed track holds. Run in release:
//!
//! ```sh
//! cargo run --release -p sariema-audio --example mix_bench -- [track.mp3]
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;

use sariema_audio::{
    Audio, Feeding, MixerSettings, MusicFades, PlayParams, SoundDef, WavFormat, container_of, wav,
};
use sariema_format::sound::{Container, Load, LoopPoints};

struct Counting;

static HELD: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        HELD.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HELD.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const RATE: u32 = 48_000;
const RUNS: usize = 7;

fn tone(i: usize) -> SoundDef {
    let frames = 10 * RATE as usize;
    let freq = 110.0 + 37.0 * i as f32;
    let samples: Vec<f32> = (0..frames)
        .map(|n| 0.1 * (TAU * freq * n as f32 / RATE as f32).sin())
        .collect();
    SoundDef {
        name: format!("tone{i}"),
        bus: 2,
        gain: 1.0,
        container: Container::Wav,
        data: Arc::from(wav(&samples, 1, RATE, WavFormat::Float32)),
        loop_points: Some(LoopPoints {
            start: 0,
            end: frames as u64,
        }),
        frames: frames as u64,
        stream: false,
        load: Load::Bank,
        enter_s: None,
        leave_s: None,
    }
}

/// Median milliseconds to mix one second with `voices` voices; with
/// `moving`, every voice's pan ramps the whole time, the slow path.
fn mix_second(voices: usize, moving: bool) -> f64 {
    let settings = MixerSettings {
        voices: voices as u32,
        ..MixerSettings::default()
    };
    let (mut audio, mut mixer) =
        Audio::new(settings, (0..voices).map(tone).collect(), Feeding::Manual).unwrap();
    let ids: Vec<_> = (0..voices)
        .map(|i| {
            let pan = (i as f32 / voices as f32) * 2.0 - 1.0;
            let params = PlayParams {
                pan,
                ..PlayParams::default()
            };
            audio
                .play(0, audio.sound(&format!("tone{i}")).unwrap(), params)
                .unwrap()
        })
        .collect();
    let mut out = vec![0.0; 2 * 480];
    let mut times = Vec::new();
    for run in 0..RUNS {
        if moving {
            let at = mixer.stats().now;
            for &id in &ids {
                let pan = if run % 2 == 0 { 1.0 } else { -1.0 };
                audio.set_voice(at, id, 1.0, pan, 1.0).unwrap();
            }
        }
        let start = Instant::now();
        for _ in 0..100 {
            mixer.render(&mut out);
        }
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    times[RUNS / 2]
}

fn stream_memory(path: &str) {
    let bytes = std::fs::read(path).unwrap();
    let container = container_of(path.rsplit('.').next().unwrap()).unwrap();
    let compressed = bytes.len();
    let decoded = sariema_audio::decode_all(Arc::from(bytes.as_slice()), container).unwrap();
    let frames = decoded.frames();
    let seconds = frames as f64 / f64::from(decoded.sample_rate);
    drop(decoded);
    let def = SoundDef {
        name: "track".into(),
        bus: 0,
        gain: 1.0,
        container,
        data: Arc::from(bytes),
        loop_points: None,
        frames,
        stream: true,
        load: Load::Bank,
        enter_s: None,
        leave_s: None,
    };
    let before = HELD.load(Ordering::Relaxed);
    let (mut audio, mut mixer) =
        Audio::new(MixerSettings::default(), vec![def], Feeding::Manual).unwrap();
    let base = HELD.load(Ordering::Relaxed);
    let track = audio.sound("track").unwrap();
    audio
        .music(0, track, MusicFades::default(), Some(true))
        .unwrap();
    let mut out = vec![0.0; 2 * 480];
    let start = Instant::now();
    for _ in 0..1000 {
        audio.pump().unwrap();
        mixer.render(&mut out);
    }
    let ten_seconds = start.elapsed().as_secs_f64() * 1000.0;
    let held = HELD.load(Ordering::Relaxed) - base;
    println!(
        "{{\"stream\":{{\"file\":\"{path}\",\"seconds\":{seconds:.1},\"compressed_bytes\":{compressed},\
         \"held_bytes_besides_compressed\":{held},\"mixer_and_bank_bytes\":{},\
         \"ms_to_decode_and_mix_10_s\":{ten_seconds:.1}}}}}",
        base - before
    );
}

fn main() {
    for voices in [4, 32, 128] {
        let flat = mix_second(voices, false);
        let moving = mix_second(voices, true);
        println!(
            "{{\"voices\":{voices},\"ms_per_second_static\":{flat:.3},\"ms_per_second_moving\":{moving:.3}}}"
        );
    }
    if let Some(path) = std::env::args().nth(1) {
        stream_memory(&path);
    }
}
