// SPDX-License-Identifier: Apache-2.0

mod common;

use common::{sine, stereo};
use sariema_audio::offline::{CueList, RenderOptions, render};
use sariema_audio::{MixerSettings, WavFormat, integrated_lufs, wav};
use sariema_format::sound::{Bus, Container, Load, LoopPoints, Loudness, Mix, Sound, SoundBank};

fn sound(name: &str, bus: u32, rate: u32, samples: Vec<f32>, looped: bool, stream: bool) -> Sound {
    let frames = (samples.len() / 2) as u64;
    let (lufs, peak) = integrated_lufs(&samples, 2, rate);
    Sound {
        name: name.into(),
        bus,
        gain_db: 0.0,
        container: Container::Wav,
        sample_rate: rate,
        channels: 2,
        frames,
        loop_points: looped.then_some(LoopPoints {
            start: 0,
            end: frames,
        }),
        stream,
        load: Load::Bank,
        enter_s: Some(0.8),
        leave_s: Some(2.0),
        loudness: Loudness {
            integrated_lufs: lufs,
            peak_dbfs: peak,
            seam_step_db: None,
        },
        data: wav(&samples, 2, rate, WavFormat::Float32),
    }
}

fn bank() -> SoundBank {
    let bus = |name: &str| Bus {
        name: name.into(),
        gain_db: 0.0,
        target_lufs: None,
    };
    let noise: Vec<f32> = (0..96_000u32)
        .map(|n| ((n.wrapping_mul(2_654_435_761) >> 8) as f32 / 16_777_216.0 - 0.5) * 0.1)
        .collect();
    SoundBank {
        buses: ["music", "voice", "effects", "ambience"].map(bus).to_vec(),
        sounds: vec![
            sound(
                "calm",
                0,
                48_000,
                stereo(&sine(220.0, 0.3, 48_000, 96_000)),
                true,
                false,
            ),
            sound(
                "chase",
                0,
                44_100,
                stereo(&sine(330.0, 0.3, 44_100, 88_200)),
                true,
                true,
            ),
            sound(
                "line",
                1,
                48_000,
                stereo(&sine(600.0, 0.4, 48_000, 40_000)),
                false,
                false,
            ),
            sound(
                "jingle",
                0,
                48_000,
                stereo(&sine(880.0, 0.3, 48_000, 30_000)),
                false,
                false,
            ),
            sound(
                "step",
                2,
                48_000,
                stereo(&sine(1500.0, 0.5, 48_000, 3_000)),
                false,
                false,
            ),
            sound("wind", 3, 48_000, stereo(&noise), true, true),
        ],
        mixes: vec![Mix {
            name: "all".into(),
            sounds: vec![0, 5],
        }],
    }
}

fn cues() -> CueList {
    let steps: String = (0..40)
        .map(|i| {
            format!(
                r#"{{"t": {:.3}, "do": "play", "sound": "step", "pan": {:.1}}},"#,
                0.9 + i as f32 * 0.008,
                (i % 5) as f32 * 0.4 - 0.8
            )
        })
        .collect();
    let json = format!(
        r#"{{
        "duration_s": 9.0,
        "cues": [
            {steps}
            {{"t": 0.0, "do": "music", "sound": "calm"}},
            {{"t": 0.0, "do": "play", "sound": "wind", "id": "wind", "gain_db": -6, "priority": 200}},
            {{"t": 1.0, "do": "play", "sound": "line", "priority": 200}},
            {{"t": 2.5, "do": "music", "sound": "chase"}},
            {{"t": 3.7, "do": "stinger", "sound": "jingle"}},
            {{"t": 5.0, "do": "music", "sound": "calm"}},
            {{"t": 5.5, "do": "bus", "bus": "ambience", "gain_db": -12, "ramp_s": 1.0}},
            {{"t": 6.0, "do": "pause"}},
            {{"t": 6.4, "do": "resume"}},
            {{"t": 7.0, "do": "stop", "id": "wind", "fade_s": 1.0}},
            {{"t": 8.0, "do": "music_stop", "fade_s": 0.5}}
        ]
    }}"#
    );
    serde_json::from_str(&json).unwrap()
}

fn settings() -> MixerSettings {
    MixerSettings {
        voices: 8,
        ..MixerSettings::default()
    }
}

#[test]
fn the_same_cues_render_bit_identical_whatever_the_buffer_size() {
    let options = |buffer_frames| RenderOptions {
        buffer_frames,
        ..RenderOptions::default()
    };
    let reference = render(bank(), settings(), &cues(), options(480)).unwrap();
    assert!(reference.samples.iter().any(|&s| s != 0.0));
    assert!(
        reference.timeline.stats.steals > 0,
        "the steps overflow 8 voices"
    );
    for buffer in [64, 480, 4096, 1] {
        let again = render(bank(), settings(), &cues(), options(buffer)).unwrap();
        assert!(
            again.samples == reference.samples,
            "buffer {buffer} differs"
        );
        assert_eq!(again.timeline.stats, reference.timeline.stats);
        assert_eq!(again.timeline.bus_lufs, reference.timeline.bus_lufs);
    }
}

#[test]
fn the_timeline_shows_ducking_and_bus_levels() {
    let out = render(bank(), settings(), &cues(), RenderOptions::default()).unwrap();
    let t = &out.timeline;
    assert_eq!(out.samples.len(), 2 * 9 * 48_000);
    assert_eq!(t.windows.len(), 90);
    assert_eq!(t.buses, ["music", "voice", "effects", "ambience"]);
    let duck = |w: usize, bus: usize| t.windows[w].duck_db[bus];
    assert_eq!(duck(5, 0), 0.0);
    assert!((duck(15, 0) - -9.0).abs() < 1e-4, "music under the line");
    assert!((duck(15, 3) - -9.0).abs() < 1e-4, "ambience under the line");
    assert_eq!(duck(15, 2), 0.0, "effects are not ducked");
    let paused = &t.windows[62];
    assert!(paused.master_rms_db < -100.0, "{}", paused.master_rms_db);
    assert!(t.windows[88].rms_db[3] < -100.0, "the wind stopped");
    assert!(t.bus_lufs.iter().all(|l| l.is_finite()));
    assert_eq!(t.stats.underruns, 0);
    assert!(t.events.iter().any(|e| e.what == "stinger jingle"));
    assert!(t.events.iter().any(|e| e.what.contains("voice stolen")));
}

#[test]
fn an_unknown_sound_in_a_cue_is_an_error() {
    let cues: CueList = serde_json::from_str(
        r#"{"duration_s": 1, "cues": [{"t": 0, "do": "play", "sound": "nope"}]}"#,
    )
    .unwrap();
    let error = render(bank(), settings(), &cues, RenderOptions::default())
        .err()
        .unwrap();
    assert_eq!(error.code(), "E_AUDIO_UNKNOWN_SOUND");
}
