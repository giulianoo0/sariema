// SPDX-License-Identifier: Apache-2.0

mod common;

use std::fs;
use std::path::Path;

use common::{sine, stereo};
use sariema_audio::{AudioProject, WavFormat, build, check, integrated_lufs, wav};

/// A 1 kHz stereo tone at `dbfs` (peak), for `seconds`.
fn tone(dbfs: f32, seconds: f32, rate: u32) -> Vec<f32> {
    let amp = 10f32.powf(dbfs / 20.0);
    stereo(&sine(1000.0, amp, rate, (seconds * rate as f32) as usize))
}

fn sequence(parts: &[(f32, f32)], rate: u32) -> Vec<f32> {
    parts
        .iter()
        .flat_map(|&(db, s)| tone(db, s, rate))
        .collect()
}

fn assert_lufs(samples: &[f32], rate: u32, expected: f32) {
    let (lufs, _) = integrated_lufs(samples, 2, rate);
    assert!(
        (lufs - expected).abs() < 0.1,
        "{lufs} LUFS, expected {expected}"
    );
}

// EBU Tech 3341, cases 1 to 5.
#[test]
fn ebu_3341_reference_signals() {
    let rate = 48_000;
    assert_lufs(&tone(-23.0, 20.0, rate), rate, -23.0);
    assert_lufs(&tone(-33.0, 20.0, rate), rate, -33.0);
    assert_lufs(
        &sequence(&[(-36.0, 10.0), (-23.0, 60.0), (-36.0, 10.0)], rate),
        rate,
        -23.0,
    );
    let gated = [
        (-72.0, 10.0),
        (-36.0, 10.0),
        (-23.0, 60.0),
        (-36.0, 10.0),
        (-72.0, 10.0),
    ];
    assert_lufs(&sequence(&gated, rate), rate, -23.0);
    assert_lufs(
        &sequence(&[(-26.0, 20.0), (-20.0, 20.1), (-26.0, 20.0)], rate),
        rate,
        -23.0,
    );
}

#[test]
fn other_sample_rates_and_short_sounds() {
    assert_lufs(&tone(-23.0, 10.0, 44_100), 44_100, -23.0);
    assert_lufs(&tone(-18.0, 0.2, 48_000), 48_000, -18.0);
    let (lufs, peak) = integrated_lufs(&vec![0.0; 96_000], 2, 48_000);
    assert_eq!((lufs, peak), (f32::NEG_INFINITY, f32::NEG_INFINITY));
    let (_, peak) = integrated_lufs(&tone(-6.0, 1.0, 48_000), 2, 48_000);
    assert!((peak - -6.0).abs() < 0.01, "{peak}");
}

fn write(dir: &Path, name: &str, samples: &[f32]) {
    fs::write(dir.join(name), wav(samples, 2, 48_000, WavFormat::Float32)).unwrap();
}

/// The three findings of the reference game's audit as files: ambience 4 dB
/// over the music, a looping heartbeat 7 dB over it, and a loop whose seam
/// steps by 11 dB.
fn audit_project(dir: &Path, targets: bool) -> AudioProject {
    write(dir, "music.wav", &tone(-30.0, 4.0, 48_000));
    write(dir, "night.wav", &tone(-26.0, 4.0, 48_000));
    write(dir, "heart.wav", &tone(-23.0, 2.0, 48_000));
    let seam = sequence(&[(-20.0, 1.0), (-31.3, 1.0)], 48_000);
    write(dir, "chase.wav", &seam);
    let buses = if targets {
        ""
    } else {
        r#""buses": [
            {"name": "music"}, {"name": "voice"}, {"name": "effects"}, {"name": "ambience"}
        ],"#
    };
    let json = format!(
        r#"{{
        {buses}
        "sounds": {{
            "music": {{"file": "music.wav", "bus": "music", "loop": true}},
            "night": {{"file": "night.wav", "bus": "ambience", "loop": true}},
            "heart": {{"file": "heart.wav", "bus": "effects", "loop": true, "gain_db": 1.0}},
            "chase": {{"file": "chase.wav", "bus": "music", "loop": {{"start": 0, "end": 96000}}}}
        }},
        "mixes": {{"yard_at_night": ["music", "night", "heart"]}},
        "checks": {{"over_music_db": {{"ambience": 0, "effects": 6}}}}
    }}"#
    );
    serde_json::from_str(&json).unwrap()
}

#[test]
fn the_audit_findings_fire_as_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let project = audit_project(dir.path(), false);
    let bank = build(&project, dir.path()).unwrap();
    let warnings = check(&bank, &project.checks);
    let found: Vec<(&str, &str, i32)> = warnings
        .iter()
        .map(|w| (w.code, w.sound.as_str(), w.value_db.round() as i32))
        .collect();
    assert_eq!(
        found,
        [
            ("W_AUDIO_OVER_MUSIC", "night", 4),
            ("W_AUDIO_OVER_MUSIC", "heart", 8),
            ("W_AUDIO_LOOP_SEAM", "chase", 11),
        ]
    );
    assert!(warnings[0].message.contains("yard_at_night"));
}

#[test]
fn bus_loudness_targets_make_the_same_files_a_calm_mix() {
    let dir = tempfile::tempdir().unwrap();
    let project = audit_project(dir.path(), true);
    let bank = build(&project, dir.path()).unwrap();
    let codes: Vec<&str> = check(&bank, &project.checks)
        .iter()
        .map(|w| w.code)
        .collect();
    assert_eq!(codes, ["W_AUDIO_LOOP_SEAM"]);
    let night = &bank.sounds.iter().find(|s| s.name == "night").unwrap();
    let music = &bank.sounds.iter().find(|s| s.name == "music").unwrap();
    let over =
        sariema_audio::effective_lufs(&bank, night) - sariema_audio::effective_lufs(&bank, music);
    assert!((over - -6.0).abs() < 0.1, "{over}");
}

#[test]
fn a_loop_past_the_end_or_an_unknown_bus_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.wav", &tone(-20.0, 1.0, 48_000));
    let bad_loop: AudioProject = serde_json::from_str(
        r#"{"sounds": {"a": {"file": "a.wav", "bus": "music", "loop": {"start": 0, "end": 48001}}}}"#,
    )
    .unwrap();
    let e = build(&bad_loop, dir.path()).unwrap_err();
    assert_eq!(e.code(), "E_SCHEMA");
    assert!(e.to_string().contains("48000 frames"), "{e}");
    let bad_bus: AudioProject =
        serde_json::from_str(r#"{"sounds": {"a": {"file": "a.wav", "bus": "dialogue"}}}"#).unwrap();
    assert!(
        build(&bad_bus, dir.path())
            .unwrap_err()
            .to_string()
            .contains("dialogue")
    );
    let missing: AudioProject =
        serde_json::from_str(r#"{"sounds": {"a": {"file": "nope.wav", "bus": "music"}}}"#).unwrap();
    assert_eq!(build(&missing, dir.path()).unwrap_err().code(), "E_IO");
}
