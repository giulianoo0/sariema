// SPDX-License-Identifier: Apache-2.0

use std::f32::consts::TAU;
use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use sariema_audio::{WavFormat, wav};
use serde_json::Value;
use tempfile::TempDir;

fn sariema(args: &[&str]) -> (i32, Value) {
    let out = cargo_bin_cmd!("sariema")
        .args(args)
        .arg("--json")
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    let value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stdout:?}"));
    (out.status.code().unwrap(), value)
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn tone(dir: &Path, name: &str, freq: f32, amp: f32, seconds: f32) {
    let frames = (seconds * 48_000.0) as usize;
    let samples: Vec<f32> = (0..frames)
        .flat_map(|n| {
            let s = amp * (TAU * freq * n as f32 / 48_000.0).sin();
            [s, s]
        })
        .collect();
    fs::write(dir.join(name), wav(&samples, 2, 48_000, WavFormat::Pcm16)).unwrap();
}

/// Ambience 4 dB over the music, with no loudness targets.
fn project(ambience_db: f32) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    tone(dir, "music.wav", 220.0, 0.03, 2.0);
    tone(
        dir,
        "night.wav",
        90.0,
        0.03 * 10f32.powf(ambience_db / 20.0),
        2.0,
    );
    tone(dir, "line.wav", 500.0, 0.2, 0.5);
    fs::write(
        dir.join("sariema.json"),
        r#"{"name": "t", "assets_root": ".", "rooms": {}, "audio": "audio.json"}"#,
    )
    .unwrap();
    fs::write(
        dir.join("audio.json"),
        r#"{
            "buses": [{"name": "music"}, {"name": "voice"}, {"name": "effects"}, {"name": "ambience"}],
            "sounds": {
                "music": {"file": "music.wav", "bus": "music", "stream": true, "loop": true, "enter_s": 0.8, "leave_s": 2},
                "night": {"file": "night.wav", "bus": "ambience", "loop": true},
                "line": {"file": "line.wav", "bus": "voice", "on_demand": true}
            },
            "mixes": {"night": ["music", "night"]}
        }"#,
    )
    .unwrap();
    tmp
}

#[test]
fn build_inspect_check_and_render() {
    let tmp = project(4.0);
    let dir = tmp.path();
    let bank = dir.join("game.srms");
    let (code, built) = sariema(&["audio", "build", arg(dir), "-o", arg(&bank)]);
    assert_eq!(code, 0, "{built}");
    assert_eq!(built["sounds"], 3);
    assert_eq!(built["warnings"][0]["code"], "W_AUDIO_OVER_MUSIC");
    assert_eq!(built["warnings"][0]["sound"], "night");
    assert_eq!(built["bytes"], fs::metadata(&bank).unwrap().len());

    let (code, inspected) = sariema(&["audio", "inspect", arg(&bank)]);
    assert_eq!(code, 0, "{inspected}");
    assert_eq!(inspected["sounds"][0]["name"], "line");
    assert_eq!(inspected["sounds"][0]["on_demand"], true);
    assert_eq!(inspected["sounds"][1]["stream"], true);
    assert_eq!(inspected["sounds"][1]["loop"]["end"], 96_000);
    assert_eq!(inspected["mixes"]["night"][1], "night");

    let (code, failed) = sariema(&["audio", "check", arg(dir), "--strict"]);
    assert_eq!(code, 1);
    assert_eq!(failed["error"]["code"], "E_AUDIO_CHECK");

    let cues = dir.join("cues.json");
    fs::write(
        &cues,
        r#"{"duration_s": 2.0, "cues": [
            {"t": 0, "do": "music", "sound": "music"},
            {"t": 0, "do": "play", "sound": "night"},
            {"t": 0, "do": "preload", "sound": "line"},
            {"t": 0.5, "do": "play", "sound": "line"}
        ]}"#,
    )
    .unwrap();
    let out = dir.join("mix.wav");
    let (code, rendered) = sariema(&[
        "audio",
        "render",
        arg(&bank),
        "--cues",
        arg(&cues),
        "-o",
        arg(&out),
        "--buffer",
        "256",
    ]);
    assert_eq!(code, 0, "{rendered}");
    assert_eq!(fs::metadata(&out).unwrap().len(), 44 + 2 * 96_000 * 3);
    let timeline: Value =
        serde_json::from_slice(&fs::read(dir.join("mix.timeline.json")).unwrap()).unwrap();
    assert_eq!(timeline["windows"].as_array().unwrap().len(), 20);
    assert!(
        rendered["bus_lufs"]["ambience"].as_f64().unwrap()
            > rendered["bus_lufs"]["music"].as_f64().unwrap()
    );
}

#[test]
fn a_calm_project_checks_clean() {
    let tmp = project(-6.0);
    let (code, checked) = sariema(&["audio", "check", arg(tmp.path()), "--strict"]);
    assert_eq!(code, 0, "{checked}");
    assert_eq!(checked["warnings"].as_array().unwrap().len(), 0);
    assert_eq!(checked["sounds"].as_array().unwrap().len(), 3);
}

#[test]
fn errors_carry_stable_codes() {
    let tmp = project(0.0);
    let dir = tmp.path();
    let bank = dir.join("game.srms");
    assert_eq!(
        sariema(&["audio", "build", arg(dir), "-o", arg(&bank)]).0,
        0
    );

    let cues = dir.join("bad.json");
    fs::write(
        &cues,
        r#"{"duration_s": 1, "cues": [{"t": 0, "do": "dance"}]}"#,
    )
    .unwrap();
    let out = dir.join("x.wav");
    let (code, bad) = sariema(&[
        "audio",
        "render",
        arg(&bank),
        "--cues",
        arg(&cues),
        "-o",
        arg(&out),
    ]);
    assert_eq!((code, bad["error"]["code"].as_str()), (1, Some("E_SCHEMA")));

    for cue in [
        r#"{"do": "play", "sound": "music", "gain_db": 800}"#,
        r#"{"do": "voice", "id": "v", "gain_db": 800}"#,
        r#"{"do": "bus", "bus": "music", "gain_db": 800}"#,
        r#"{"do": "master", "gain_db": 800}"#,
    ] {
        let cue = cue.replacen('{', r#"{"t": 0, "#, 1);
        fs::write(&cues, format!(r#"{{"duration_s": 1, "cues": [{cue}]}}"#)).unwrap();
        let (_, loud) = sariema(&[
            "audio",
            "render",
            arg(&bank),
            "--cues",
            arg(&cues),
            "-o",
            arg(&out),
        ]);
        assert_eq!(loud["error"]["code"], "E_SCHEMA", "{cue}");
    }

    fs::write(
        &cues,
        r#"{"duration_s": 1, "cues": [{"t": 0, "do": "play", "sound": "line"}]}"#,
    )
    .unwrap();
    let (_, not_loaded) = sariema(&[
        "audio",
        "render",
        arg(&bank),
        "--cues",
        arg(&cues),
        "-o",
        arg(&out),
    ]);
    assert_eq!(not_loaded["error"]["code"], "E_AUDIO_NOT_LOADED");

    fs::write(
        &cues,
        r#"{"duration_s": 1, "cues": [{"t": 0, "do": "play", "sound": "nope"}]}"#,
    )
    .unwrap();
    let (_, unknown) = sariema(&[
        "audio",
        "render",
        arg(&bank),
        "--cues",
        arg(&cues),
        "-o",
        arg(&out),
    ]);
    assert_eq!(unknown["error"]["code"], "E_AUDIO_UNKNOWN_SOUND");

    let room = dir.join("audio.json");
    let (_, magic) = sariema(&["audio", "inspect", arg(&room)]);
    assert_eq!(magic["error"]["code"], "E_PACK_BAD_MAGIC");

    let (code, usage) = sariema(&["audio", "inspect", arg(&bank), "--loud"]);
    assert_eq!(
        (code, usage["error"]["code"].as_str()),
        (1, Some("E_USAGE"))
    );

    fs::write(dir.join("broken.wav"), b"RIFF not really").unwrap();
    let text = fs::read_to_string(&room)
        .unwrap()
        .replace("line.wav", "broken.wav");
    fs::write(&room, text).unwrap();
    let (_, decode) = sariema(&["audio", "build", arg(dir), "-o", arg(&bank)]);
    assert_eq!(decode["error"]["code"], "E_AUDIO_DECODE");

    for loud in [
        r#"{"buses": [{"name": "music", "gain_db": 800}], "sounds": {}}"#,
        r#"{"buses": [{"name": "music"}], "sounds": {"m": {"file": "music.wav", "bus": "music", "gain_db": 800}}}"#,
    ] {
        fs::write(&room, loud).unwrap();
        let (_, schema) = sariema(&["audio", "build", arg(dir), "-o", arg(&bank)]);
        assert_eq!(schema["error"]["code"], "E_SCHEMA", "{loud}");
    }

    fs::write(
        dir.join("sariema.json"),
        r#"{"name": "t", "assets_root": ".", "rooms": {}}"#,
    )
    .unwrap();
    let (_, missing) = sariema(&["audio", "check", arg(dir)]);
    assert_eq!(missing["error"]["code"], "E_SCHEMA");
}
