// SPDX-License-Identifier: Apache-2.0

mod common;

use std::f32::consts::{FRAC_PI_4, TAU};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use sariema_audio::{
    Audio, Feeding, MixerSettings, MusicFades, PlayParams, StreamSettings, decode_all,
};
use sariema_format::sound::{Container, Load, LoopPoints};

fn fixture(name: &str) -> Arc<[u8]> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    Arc::from(std::fs::read(path).unwrap())
}

fn centre() -> f32 {
    FRAC_PI_4.cos()
}

#[test]
fn streamed_loop_is_sample_exact_for_100_loops() {
    let period = 480;
    let wave = |n: usize| (TAU * (n % period) as f32 / period as f32).sin() * 0.5;
    let def = Def::mono("loop", AMBIENCE, (0..6720).map(wave).collect())
        .looped(960, 5760)
        .streamed();
    let (mut audio, mut mixer) = setup(quiet_settings(), vec![def]);
    let id = audio.sound("loop").unwrap();
    audio.play(0, id, PlayParams::default()).unwrap();
    let out = left(&render(&mut audio, &mut mixer, 960 + 100 * 4800, 480));
    for (n, &s) in out.iter().enumerate().skip(144) {
        assert_eq!(s, wave(n) * centre(), "frame {n}");
    }
    assert_eq!(audio.stats().underruns, 0);
}

#[test]
fn resampled_stream_loops_without_drift() {
    // 441 Hz is 100 frames at 44.1 kHz; the loop holds 44 periods.
    let src_rate = 44_100;
    let wave = |n: usize| (TAU * (n % 100) as f32 / 100.0).sin() * 0.5;
    let def = Def {
        rate: src_rate,
        ..Def::mono("loop", AMBIENCE, (0..8800).map(wave).collect())
    }
    .looped(2200, 6600)
    .streamed();
    let (mut audio, mut mixer) = setup(quiet_settings(), vec![def]);
    let id = audio.sound("loop").unwrap();
    audio.play(0, id, PlayParams::default()).unwrap();
    let loops = 100;
    let frames = (6600 + (loops - 1) * 4400) * 48_000 / src_rate as usize;
    let out = left(&render(&mut audio, &mut mixer, frames, 480));
    let worst = (1_000..frames)
        .map(|n| {
            let ideal = (TAU * 441.0 * n as f64 as f32 / 48_000.0).sin();
            (out[n] - 0.5 * centre() * ideal).abs()
        })
        .fold(0.0, f32::max);
    assert!(worst < 3e-3, "{worst}");
}

#[test]
fn a_starved_stream_fades_out_reports_and_comes_back() {
    let settings = MixerSettings {
        stream: StreamSettings {
            buffer_s: 0.05,
            ..StreamSettings::default()
        },
        ..quiet_settings()
    };
    let def = Def::mono("dc", AMBIENCE, vec![0.5; 48_000]).streamed();
    let (mut audio, mut mixer) = setup(settings, vec![def]);
    let id = audio.sound("dc").unwrap();
    audio.play(0, id, PlayParams::default()).unwrap();
    // Nobody refills: the ring holds 2400 frames.
    let mut out = vec![0.0; 2 * 9600];
    mixer.render(&mut out);
    let out = left(&out);
    let level = 0.5 * centre();
    assert_eq!(audio.stats().underruns, 1);
    assert!(out[200..2200].iter().all(|&s| (s - level).abs() < 1e-6));
    let gap = out.iter().rposition(|&s| s != 0.0).unwrap() + 1;
    assert!(gap <= 2400);
    assert!(out[gap..].iter().all(|&s| s == 0.0), "silence, not garbage");
    assert!(max_jump(&out) < level / 100.0, "{}", max_jump(&out));
    let after = left(&render(&mut audio, &mut mixer, 4800, 480));
    assert!(after[..10].iter().all(|&s| s < level));
    assert!(after[200..].iter().all(|&s| (s - level).abs() < 1e-6));
    assert!(max_jump(&after) < level / 100.0);
}

#[test]
fn a_stream_that_ends_frees_its_voice() {
    let def = Def::mono("short", EFFECTS, vec![0.5; 10_000]).streamed();
    let (mut audio, mut mixer) = setup(quiet_settings(), vec![def]);
    let id = audio.sound("short").unwrap();
    audio.play(0, id, PlayParams::default()).unwrap();
    let out = left(&render(&mut audio, &mut mixer, 20_000, 480));
    assert!(out[10_000..].iter().all(|&s| s == 0.0));
    assert!(out[9_999] < out[9_000] / 50.0, "the end is faded");
    assert_eq!(audio.stats().voices, 0);
}

#[test]
fn mp3_and_ogg_decode_gapless_with_the_onset_in_place() {
    // ffmpeg's own Vorbis encoder declares 56 frames of end padding as audio.
    let files = [
        ("tone.mp3", Container::Mp3, 88_200),
        ("tone.ogg", Container::Ogg, 88_256),
    ];
    for (name, container, frames) in files {
        let decoded = decode_all(fixture(name), container).unwrap();
        assert_eq!(
            (decoded.sample_rate, decoded.channels),
            (44_100, 2),
            "{name}"
        );
        assert_eq!(decoded.frames(), frames, "{name}");
        let onset = decoded
            .samples
            .iter()
            .step_by(2)
            .position(|s| s.abs() > 0.05)
            .unwrap();
        assert!(
            (22_050..22_062).contains(&onset),
            "{name}: onset at {onset}"
        );
    }
}

fn file_def(name: &str, container: Container, loop_points: LoopPoints) -> sariema_audio::SoundDef {
    sariema_audio::SoundDef {
        name: name.into(),
        bus: MUSIC,
        gain: 1.0,
        container,
        data: fixture(name),
        loop_points: Some(loop_points),
        frames: 88_200,
        stream: true,
        load: Load::Bank,
        enter_s: None,
        leave_s: None,
    }
}

#[test]
fn compressed_streams_loop_smoothly_after_seeking_back() {
    // One second of the tone, 440 whole periods, away from its onset and
    // the end of the file, where lossy codecs smear the signal.
    let region = LoopPoints {
        start: 33_075,
        end: 77_175,
    };
    for (name, container) in [("tone.mp3", Container::Mp3), ("tone.ogg", Container::Ogg)] {
        let sounds = vec![file_def(name, container, region)];
        let (mut audio, mut mixer) = Audio::new(quiet_settings(), sounds, Feeding::Manual).unwrap();
        let id = audio.sound(name).unwrap();
        let fades = MusicFades {
            fade_in_s: Some(0.0),
            ..MusicFades::default()
        };
        audio.music(0, id, fades, Some(true)).unwrap();
        let out = left(&render(&mut audio, &mut mixer, 10 * 48_000, 480));
        let natural = 0.25 * TAU * 440.0 / 48_000.0;
        let jump = max_jump(&out[48_000..]);
        assert!(jump < natural * 1.25, "{name}: {jump} vs {natural}");
        let decoded = left(&decode_all(fixture(name), container).unwrap().samples);
        let (level, file) = (rms_db(&out[48_000..]), rms_db(&decoded[33_075..77_175]));
        assert!(
            (level - file).abs() < 0.05,
            "{name}: {level} vs the file's {file}"
        );
    }
}

#[test]
fn a_decoding_thread_keeps_a_stream_fed() {
    let def = Def::mono("dc", AMBIENCE, vec![0.5; 96_000])
        .streamed()
        .build();
    let (mut audio, mut mixer) = Audio::new(quiet_settings(), vec![def], Feeding::Thread).unwrap();
    let id = audio.sound("dc").unwrap();
    audio.play(0, id, PlayParams::default()).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let mut out = vec![0.0; 2 * 24_000];
    for chunk in out.chunks_mut(960) {
        mixer.render(chunk);
        std::thread::sleep(Duration::from_millis(2));
    }
    let out = left(&out);
    assert!(
        out[1_000..]
            .iter()
            .all(|&s| (s - 0.5 * centre()).abs() < 1e-6)
    );
    assert_eq!(audio.stats().underruns, 0);
    assert!(audio.take_stream_errors().is_empty());
}
