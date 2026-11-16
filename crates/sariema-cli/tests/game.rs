// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use sariema_format::script::{COMMANDS, Command, Reader, WireError};
use sariema_runtime::cutscene::PLAY_CODES;
use sariema_runtime::cutscene::check::CODES as CHECK_CODES;
use sariema_runtime::fx::FxReject;
use sariema_runtime::scenery::SceneReject;
use sariema_runtime::world::Reject;
use sariema_vm::VmError;
use serde_json::{Value, json};
use tempfile::TempDir;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn hello() -> PathBuf {
    repo().join("examples/hello-game")
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

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

fn ok(args: &[&str]) -> Value {
    let (code, out) = sariema(args);
    assert_eq!(code, 0, "{args:?}: {out}");
    out
}

fn error_code(args: &[&str]) -> (String, String) {
    let (code, out) = sariema(args);
    assert_eq!(code, 1, "{args:?}: {out}");
    (
        out["error"]["code"].as_str().unwrap().into(),
        out["error"]["message"].as_str().unwrap().into(),
    )
}

#[test]
fn api_matches_its_schema_and_lists_every_command_the_decoder_accepts() {
    let api = ok(&["api"]);
    let schema: Value =
        serde_json::from_str(&fs::read_to_string(repo().join("schemas/api.schema.json")).unwrap())
            .unwrap();
    jsonschema::validate(&schema, &api).unwrap();

    let listed: BTreeSet<u64> = api["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["opcode"].as_u64().unwrap())
        .collect();
    let accepted: BTreeSet<u64> = (0..=u16::MAX)
        .filter(|op| {
            let mut bytes = op.to_le_bytes().to_vec();
            bytes.resize(2 + 4096, 0);
            !matches!(
                Command::read(&mut Reader::new(&bytes)),
                Err(WireError::UnknownOpcode(_))
            )
        })
        .map(u64::from)
        .collect();
    assert_eq!(listed, accepted);
    assert_eq!(listed.len(), COMMANDS.len());
}

#[test]
fn api_lists_every_error_code_the_engine_gives() {
    let api = ok(&["api"]);
    let listed: BTreeSet<&str> = api["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["code"].as_str().unwrap())
        .collect();
    let wire = [
        WireError::Truncated,
        WireError::UnknownOpcode(0),
        WireError::Invalid(""),
    ]
    .map(WireError::code);
    let given = VmError::CODES
        .into_iter()
        .chain(CHECK_CODES)
        .chain(PLAY_CODES)
        .chain(Reject::ALL.map(Reject::code))
        .chain(SceneReject::ALL.map(SceneReject::code))
        .chain(FxReject::ALL.map(FxReject::code))
        .chain(sariema_engine::REJECTED.iter().copied())
        .chain(wire)
        .chain([
            "E_INPUT_LOG",
            "E_RIG_UNKNOWN",
            "E_BATCH_VERSION",
            "E_CUTSCENE_ROLE",
            "E_GAME_BUILD",
            "E_PROJECT_EXISTS",
            "E_SETTINGS",
        ]);
    for code in given {
        assert!(listed.contains(code), "{code} is not in sariema api");
    }
}

/// The events of a run other than input changes, as `step name`.
fn story(run: &Value) -> Vec<String> {
    run["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"] != "axis")
        .map(|e| format!("{} {}", e["step"], e["event"].as_str().unwrap()))
        .collect()
}

fn build_hello(dir: &Path) -> PathBuf {
    let pack = dir.join("hello.srmg");
    let built = ok(&["build", arg(&hello()), "-o", arg(&pack)]);
    assert_eq!(built["cutscenes"], json!(["meet"]));
    assert_eq!(
        (
            &built["room"],
            &built["collision"],
            &built["actors"],
            &built["sounds"],
            &built["hud"]
        ),
        (
            &json!(true),
            &json!(false),
            &json!([]),
            &json!(false),
            &json!(false)
        ),
        "the meadow room travels in the pack; the project has no collision, actors, sounds or HUD"
    );
    pack
}

fn run_hello(pack: &Path, extra: &[&str]) -> Value {
    let input = hello().join("input/walk.json");
    let mut args = vec![
        "run",
        "--headless",
        "--game",
        arg(pack),
        "--input",
        arg(&input),
        "--steps",
        "1400",
    ];
    args.extend_from_slice(extra);
    ok(&args)
}

#[test]
fn hello_game_replays_its_input_log_to_pinned_events_and_state() {
    let tmp = TempDir::new().unwrap();
    let pack = build_hello(tmp.path());
    let run = run_hello(&pack, &[]);
    assert_eq!(
        story(&run),
        [
            "0 started",
            "1 save_load_done",
            "1 trigger_enter",
            "181 trigger_enter",
            "182 control",
            "182 cutscene_started",
            "200 arrived",
            "267 arrived",
            "418 line_done",
            "419 flag_changed",
            "419 control",
            "419 cutscene_ended",
            "955 trigger_exit",
            "1196 trigger_enter",
            "1316 line_done",
        ]
    );
    let host: Vec<&str> = run["host"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["message"].as_str().unwrap())
        .collect();
    assert_eq!(host, ["load_saves", "ready", "save"]);
    assert_eq!(run["saves"], json!({ "met": [1] }));
    assert_eq!(run["names"]["2295560534"], "meet");
    // The runtime's and glam's f32 math (sin, exp, powf) come from the libm
    // crate, not the platform's, so every platform, wasm32 included, gives
    // this hash.
    assert_eq!(run["world_hash"], "13cf49016a4d7158");
    let again = run_hello(&pack, &[]);
    assert_eq!(
        again["state_hash"], run["state_hash"],
        "the run is deterministic"
    );
}

#[test]
fn a_saved_meeting_gives_a_line_instead_of_the_cutscene() {
    let tmp = TempDir::new().unwrap();
    let pack = build_hello(tmp.path());
    let saves = tmp.path().join("saves.json");
    fs::write(&saves, r#"{ "met": [1] }"#).unwrap();
    let run = run_hello(&pack, &["--saves", arg(&saves)]);
    let story = story(&run);
    assert!(
        !story.iter().any(|e| e.ends_with("cutscene_started")),
        "{story:?}"
    );
    assert!(story.contains(&"301 line_done".to_string()), "{story:?}");
}

fn project_with(dir: &Path, cutscene: &str) {
    fs::create_dir_all(dir.join("cutscenes")).unwrap();
    fs::write(
        dir.join("sariema.json"),
        r#"{ "name": "t", "assets_root": ".", "rooms": {}, "game": {} }"#,
    )
    .unwrap();
    fs::write(dir.join("cutscenes/jump.json"), cutscene).unwrap();
}

#[test]
fn check_names_the_cutscene_and_step_of_a_visible_teleport_and_passes_it_under_a_fade() {
    let tmp = TempDir::new().unwrap();
    let jump = |steps: &str| {
        format!(
            r#"{{ "name": "jump", "roles": {{ "hero": {{ "start": {{ "position": [0, 0, 0] }} }} }}, "steps": [{steps}] }}"#
        )
    };
    project_with(
        tmp.path(),
        &jump(r#"{ "wait": 1 }, { "teleport": { "role": "hero", "position": [5, 0, 0] } }"#),
    );
    let (code, message) = error_code(&["check", arg(tmp.path())]);
    assert_eq!(code, "E_CUTSCENE_TELEPORT");
    assert!(
        message.contains("cutscene jump, steps[1].teleport"),
        "{message}"
    );

    project_with(
        tmp.path(),
        &jump(
            r#"{ "fade": { "to": 1, "duration": 0.5 } },
               { "teleport": { "role": "hero", "position": [5, 0, 0], "cover": "fade" } },
               { "fade": { "to": 0, "duration": 0.5 } }"#,
        ),
    );
    assert_eq!(
        ok(&["check", arg(tmp.path())])["cutscenes"],
        json!(["jump"])
    );

    project_with(tmp.path(), &jump(r#"{ "walk": { "role": "hero" } }"#));
    let (code, message) = error_code(&["check", arg(tmp.path())]);
    assert_eq!(code, "E_SCHEMA");
    assert!(message.contains("jump.json"), "{message}");
}

#[test]
fn check_holds_walks_to_the_speed_the_project_lets_actors_be_seen_moving() {
    let tmp = TempDir::new().unwrap();
    let project = |settings: &str| {
        fs::write(
            tmp.path().join("sariema.json"),
            format!(
                r#"{{ "name": "t", "assets_root": ".", "rooms": {{}}, "game": {{ "settings": {settings} }} }}"#
            ),
        )
        .unwrap();
    };
    project_with(
        tmp.path(),
        r#"{ "name": "dash", "roles": { "hero": { "start": { "position": [0, 0, 0] } } },
            "steps": [ { "wait": 1 }, { "walk": { "role": "hero", "to": [10, 0, 0], "speed": 30 } } ] }"#,
    );
    let (code, message) = error_code(&["check", arg(tmp.path())]);
    assert_eq!(code, "E_CUTSCENE_SPEED");
    assert!(
        message.contains("cutscene dash, steps[1].walk"),
        "{message}"
    );
    assert!(message.contains("motion.max_visible_speed"), "{message}");

    project(r#"{ "motion": { "max_visible_speed": 40 } }"#);
    assert_eq!(
        ok(&["check", arg(tmp.path())])["cutscenes"],
        json!(["dash"])
    );

    project(r#"{ "motion": { "max_visible_speed": 40 }, "cutscene": { "start_walk_speed": 50 } }"#);
    let (code, message) = error_code(&["check", arg(tmp.path())]);
    assert_eq!(code, "E_CUTSCENE_SPEED");
    assert!(
        message.contains("cutscene dash, roles.hero.start"),
        "{message}"
    );

    project(r#"{ "min_time_scale": 2, "max_time_scale": 1 }"#);
    let (code, message) = error_code(&["check", arg(tmp.path())]);
    assert_eq!(code, "E_SETTINGS");
    assert!(message.contains("time_scale"), "{message}");
}

#[test]
fn new_makes_a_project_that_checks_builds_and_runs() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("first-game");
    let made = ok(&["new", arg(&dir), "--toolchain", "1.100.0"]);
    assert_eq!(made["name"], "first-game");
    let (code, _) = error_code(&["new", arg(&dir)]);
    assert_eq!(code, "E_PROJECT_EXISTS");

    let checked = ok(&["check", arg(&dir)]);
    assert_eq!(checked["cutscenes"], json!(["intro"]));
    assert_eq!(checked["warnings"], json!([]));
    let pack = tmp.path().join("first.srmg");
    let built = ok(&["build", arg(&dir), "-o", arg(&pack)]);
    assert_eq!(
        (
            &built["room"],
            &built["collision"],
            &built["actors"],
            &built["sounds"],
            &built["hud"]
        ),
        (
            &json!(true),
            &json!(true),
            &json!(["hero"]),
            &json!(true),
            &json!(true)
        ),
        "a room with collision, a hero, sound and a HUD"
    );
    let input = dir.join("input/walk.json");
    let run = ok(&[
        "run",
        "--headless",
        "--game",
        arg(&pack),
        "--input",
        arg(&input),
        "--steps",
        "480",
    ]);
    let story: Vec<String> = story(&run)
        .into_iter()
        .map(|e| e.split_once(' ').unwrap().1.to_owned())
        .filter(|e| !["buffered", "axis", "control", "hit_wall"].contains(&e.as_str()))
        .collect();
    assert_eq!(
        story,
        [
            "started",
            "cutscene_started",
            "arrived",
            "line_done",
            "cutscene_ended",
            "button",
            "button",
            "clip_marker",
            "clip_finished"
        ],
        "the intro walks the hero in and says its line, then the player waves"
    );
    assert!(run["audio_hash"].is_string(), "{run}");

    let room = tmp.path().join("start.srma");
    ok(&["build", arg(&dir), "--room", "start", "-o", arg(&room)]);
}

#[test]
fn game_runs_take_their_own_flags() {
    let cases: [&[&str]; 4] = [
        &["run", "--game", "x.srmg", "--steps", "1"],
        &["run", "--headless", "--game", "x.srmg"],
        &[
            "run",
            "--headless",
            "--game",
            "x.srmg",
            "--steps",
            "1",
            "--frames",
            "1",
        ],
        &["build", "x", "-o", "x.srmg", "--textures", "astc"],
    ];
    for args in cases {
        assert_eq!(error_code(args).0, "E_USAGE", "{args:?}");
    }
    let tmp = TempDir::new().unwrap();
    let junk = tmp.path().join("junk.srmg");
    fs::write(&junk, b"not a pack").unwrap();
    let (code, _) = error_code(&["run", "--headless", "--game", arg(&junk), "--steps", "1"]);
    assert_eq!(code, "E_PACK_BAD_MAGIC");
}

#[test]
fn the_climb_example_replays_through_the_whole_engine_to_pinned_events_and_state() {
    let tmp = TempDir::new().unwrap();
    let climb = repo().join("examples/climb");
    let pack = tmp.path().join("climb.srmg");
    let built = ok(&["build", arg(&climb), "-o", arg(&pack)]);
    assert_eq!(
        (
            &built["collision"],
            &built["actors"],
            &built["sounds"],
            &built["hud"]
        ),
        (&json!(true), &json!(["hero"]), &json!(true), &json!(true))
    );
    let log = climb.join("input/play.json");
    let run = |steps: &str| {
        ok(&[
            "run",
            "--headless",
            "--game",
            arg(&pack),
            "--input",
            arg(&log),
            "--steps",
            steps,
        ])
    };
    let played = run("900");
    let story: Vec<String> = story(&played)
        .into_iter()
        .filter(|e| !e.ends_with(" buffered"))
        .collect();
    assert_eq!(
        story,
        [
            "0 started",
            "31 hit_wall",
            "50 button",
            "54 button",
            "66 clip_marker",
            "99 clip_finished",
            "186 trigger_enter",
            "187 control",
            "187 cutscene_started",
            "213 arrived",
            "243 arrived",
            "244 cutscene_call",
            "249 trigger_exit",
            "259 clip_marker",
            "267 trigger_enter",
            "292 clip_finished",
            "328 trigger_exit",
            "340 arrived",
            "436 line_done",
            "485 control",
            "485 cutscene_ended",
        ],
        "walls, the wave's marker, the sill zone, the walk to the mark, the climb through the window and its doorway trigger, the line"
    );
    // The same in the browsers (packages/sariema-web, `npm test`).
    assert_eq!(played["engine_hash"], "8828f445d1990c5c");
    let again = run("900");
    for hash in ["engine_hash", "physics_hash", "audio_hash"] {
        assert_eq!(again[hash], played[hash], "{hash} differs between two runs");
    }
}

#[test]
fn a_headless_game_run_draws_the_frames_asked_for_without_changing_what_happens() {
    let tmp = TempDir::new().unwrap();
    let climb = repo().join("examples/climb");
    let pack = tmp.path().join("climb.srmg");
    ok(&["build", arg(&climb), "-o", arg(&pack)]);
    let log = climb.join("input/play.json");
    let strip = tmp.path().join("strip.png");
    let mut args = vec![
        "run",
        "--headless",
        "--game",
        arg(&pack),
        "--input",
        arg(&log),
        "--steps",
        "300",
    ];
    let unseen = ok(&args);
    args.extend([
        "--screenshot",
        arg(&strip),
        "--filmstrip",
        "3",
        "--width",
        "320",
        "--height",
        "180",
    ]);
    let seen = ok(&args);
    assert_eq!(seen["engine_hash"], unseen["engine_hash"]);
    assert_eq!(seen["frames"]["steps"], json!([99, 199, 299]));
    assert_eq!(seen["frames"]["frame_stats"]["actors_drawn"], 1);
    let image = image::open(&strip).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), (960, 180));
    let first = image.get_pixel(0, 0);
    assert!(
        image.pixels().any(|p| p != first),
        "the frames show the room, the hero and the HUD"
    );
}

#[test]
fn the_rooms_example_walks_into_the_hall_then_dissolves_into_the_attic() {
    let tmp = TempDir::new().unwrap();
    let pack = tmp.path().join("rooms.srmg");
    let built = ok(&[
        "build",
        arg(&repo().join("examples/rooms")),
        "-o",
        arg(&pack),
    ]);
    assert_eq!(built["rooms"], json!(["porch", "attic", "hall"]));
    let run = || ok(&["run", "--headless", "--game", arg(&pack), "--steps", "600"]);
    let played = run();
    assert_eq!(
        story(&played),
        [
            "0 started",
            "169 trigger_enter",
            "170 room_switched",
            "207 trigger_exit",
            "319 arrived",
            "379 timer_fired",
            "380 room_switched",
        ]
    );
    let again = run();
    for hash in ["engine_hash", "physics_hash", "audio_hash"] {
        assert_eq!(again[hash], played[hash], "{hash} differs between two runs");
    }
}
