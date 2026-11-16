// SPDX-License-Identifier: Apache-2.0

//! Writes what the browser tests load, and the native frames they compare
//! against:
//!
//!   cargo run -p sariema-web --example fixtures --release -- <out> [--packs <dir>]
//!
//! `<out>` gets the demo room with and without occlusion, the demo arm, a
//! sound bank with a beep, the UI example and its font, the goldens those
//! scenes match, a game whose logic never returns (`forever.srmg`), and
//! `fixtures.json` listing the scenes. Each `.srma` in
//! `--packs` is drawn natively at the same size and quality, the reference
//! kept beside it as `<pack>.png`: game rooms and their images stay in
//! that folder, outside the repository.

use std::path::{Path, PathBuf};

use sariema_core::room_camera;
use sariema_format::actor::demo::demo_arm;
use sariema_format::demo::demo_room;
use sariema_format::script::{BATCH_VERSION, GameAssets, GamePack, encode_game};
use sariema_format::sound::{Bus, Container, Load, Loudness, Sound, SoundBank};
use sariema_render::{Headless, Quality};
use serde_json::{Value, json};

const W: u32 = 960;
const H: u32 = 540;

fn write(out: &Path, name: &str, bytes: &[u8]) {
    std::fs::write(out.join(name), bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
}

fn beep_bank(rate: u32) -> Vec<u8> {
    let frames = rate / 4;
    let samples: Vec<f32> = (0..frames)
        .map(|i| 0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin())
        .collect();
    let bank = SoundBank {
        buses: (sariema_audio::default_buses().into_iter())
            .map(|b| Bus {
                name: b.name,
                gain_db: 0.0,
                target_lufs: None,
            })
            .collect(),
        sounds: vec![Sound {
            name: "beep".into(),
            bus: 2,
            gain_db: 0.0,
            container: Container::Wav,
            sample_rate: rate,
            channels: 1,
            frames: frames as u64,
            loop_points: None,
            stream: false,
            load: Load::Bank,
            enter_s: None,
            leave_s: None,
            loudness: Loudness {
                integrated_lufs: -9.0,
                peak_dbfs: -6.0,
                seam_step_db: None,
            },
            data: sariema_audio::wav(&samples, 1, rate, sariema_audio::WavFormat::Pcm16),
        }],
        mixes: Vec::new(),
    };
    sariema_format::sound::encode(&bank)
}

fn native_reference(pack: &Path) -> Value {
    let bytes = std::fs::read(pack).unwrap();
    let room = sariema_format::decode(&bytes).unwrap();
    let mut headless = Headless::new(W, H, Quality::high()).unwrap();
    headless.renderer().load_room(&room).unwrap();
    let image = headless.render(&room_camera(&room.camera, W as f32 / H as f32), 0.0);
    let name = pack.file_name().unwrap().to_string_lossy().into_owned();
    image.save(pack.with_extension("srma.png")).unwrap();
    json!({ "name": name, "room": format!("packs/{name}"), "reference": format!("packs/{name}.png") })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().expect("usage: fixtures <out> [--packs <dir>]"));
    let packs = match (args.next().as_deref(), args.next()) {
        (Some("--packs"), Some(dir)) => Some(PathBuf::from(dir)),
        (None, _) => None,
        _ => panic!("usage: fixtures <out> [--packs <dir>]"),
    };
    std::fs::create_dir_all(&out).unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    write(&out, "demo.srma", &sariema_format::encode(&demo_room()));
    let mut occluded = demo_room();
    occluded.look.ao_strength = 1.0;
    write(&out, "demo_ao.srma", &sariema_format::encode(&occluded));
    write(
        &out,
        "arm.srmk",
        &sariema_format::actor::encode(&demo_arm()),
    );
    write(&out, "beep.srms", &beep_bank(48_000));
    let forever = GamePack {
        name: "forever".into(),
        batch_version: BATCH_VERSION,
        wasm: wat::parse_str(
            r#"(module (memory (export "memory") 1) (func (export "sariema_tick") (loop $l (br $l))))"#,
        )
        .unwrap(),
        cutscenes: Vec::new(),
        names: Vec::new(),
        settings: "{}".into(),
        assets: GameAssets {
            room: Some(sariema_format::encode(&demo_room())),
            ..GameAssets::default()
        },
    };
    write(&out, "forever.srmg", &encode_game(&forever));
    for (from, to) in [
        ("examples/ui/controls.json", "controls.json"),
        ("crates/sariema-ui/fonts/Nunito.ttf", "Nunito.ttf"),
        ("tests/golden/demo_lit.png", "demo_lit.png"),
        ("tests/golden/demo_ao.png", "demo_ao.png"),
        ("tests/golden/demo_actor_bent.png", "demo_actor_bent.png"),
    ] {
        std::fs::copy(repo.join(from), out.join(to)).unwrap_or_else(|e| panic!("{from}: {e}"));
    }

    let arm = json!({
        "pack": "arm.srmk",
        "clip": "bend",
        "time": 0.5,
        "speed": 0.0,
        "scale": 1.5,
        "rotate_y": 0.6,
        "translate": [0.6, 0.0, 1.2],
    });
    let mut scenes = vec![
        json!({ "name": "demo_lit", "room": "demo.srma", "reference": "demo_lit.png" }),
        json!({ "name": "demo_ao", "room": "demo_ao.srma", "reference": "demo_ao.png" }),
        json!({ "name": "demo_actor_bent", "room": "demo.srma", "reference": "demo_actor_bent.png", "actor": arm }),
    ];
    let mut rooms = Vec::new();
    if let Some(dir) = packs {
        let mut found: Vec<PathBuf> = (std::fs::read_dir(&dir).unwrap())
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "srma"))
            .collect();
        found.sort();
        for pack in found {
            eprintln!("drawing {}", pack.display());
            rooms.push(native_reference(&pack));
        }
    }
    scenes.extend(rooms);
    let manifest = json!({
        "width": W,
        "height": H,
        "quality": "high",
        "scenes": scenes,
        "sounds": "beep.srms",
        "forever": "forever.srmg",
        "ui": { "description": "controls.json", "font": "../../crates/sariema-ui/fonts/Nunito.ttf", "font_file": "Nunito.ttf" },
    });
    write(
        &out,
        "fixtures.json",
        serde_json::to_string_pretty(&manifest).unwrap().as_bytes(),
    );
    println!(
        "{}",
        json!({ "ok": true, "out": out, "scenes": manifest["scenes"].as_array().unwrap().len() })
    );
}
