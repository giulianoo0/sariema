// SPDX-License-Identifier: Apache-2.0

//! A game pack played by the web host, natively: replays hash alike and
//! keep the log they started with, a frame draws the game's room, a page
//! frame runs the steps its time owes, a game that never returns is stopped
//! by its budget, a replay ends at the step that stops its game, a new
//! renderer takes the game on where it was, and the page takes the game's
//! messages and answers its loads.

mod common;

use std::time::Instant;

use common::{H, Target, W};
use sariema_format::demo::demo_room;
use sariema_format::script::{BATCH_VERSION, Event, GameAssets, GamePack, encode_game, records};
use sariema_render::{GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, compare_images};
use sariema_runtime::outbox::HostOut;
use sariema_web::feed::FeedSettings;
use sariema_web::game::GameHost;
use sariema_web::stage::Stage;

const IDLE: &str = r#"(module (memory (export "memory") 1) (func (export "sariema_tick")))"#;
const FOREVER: &str =
    r#"(module (memory (export "memory") 1) (func (export "sariema_tick") (loop $l (br $l))))"#;
const LATE: &str = r#"(module (memory (export "memory") 1) (global $ticks (mut i32) (i32.const 0))
    (func (export "sariema_tick")
      (global.set $ticks (i32.add (global.get $ticks) (i32.const 1)))
      (if (i32.eq (global.get $ticks) (i32.const 30)) (then (loop $l (br $l))))))"#;
const NO_INPUT: &str = r#"{"changes":[]}"#;
/// Sends back every tick's events as a `host_send` on topic `events`, asks
/// for its saves at its second tick and saves `best` = [12] at its third.
const ECHO: &str = r#"(module
  (import "sariema" "submit" (func $submit (param i32 i32)))
  (import "sariema" "events" (func $events (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "\a4\00")
  (data (i32.const 16) "\a3\00\04\00\00\00best\01\00\00\00\0c")
  (data (i32.const 1008) "\a5\00\06\00\00\00events")
  (global $ticks (mut i32) (i32.const 0))
  (func (export "sariema_tick")
    (local $n i32)
    (local.set $n (call $events (i32.const 1024) (i32.const 60000)))
    (if (i32.gt_s (local.get $n) (i32.const 0))
      (then
        (i32.store (i32.const 1020) (local.get $n))
        (call $submit (i32.const 1008) (i32.add (i32.const 16) (local.get $n)))))
    (if (i32.eq (global.get $ticks) (i32.const 1))
      (then (call $submit (i32.const 0) (i32.const 2))))
    (if (i32.eq (global.get $ticks) (i32.const 2))
      (then (call $submit (i32.const 16) (i32.const 15))))
    (global.set $ticks (i32.add (global.get $ticks) (i32.const 1)))))"#;

fn pack(wat: &str) -> Vec<u8> {
    encode_game(&GamePack {
        name: "test".into(),
        batch_version: BATCH_VERSION,
        wasm: wat::parse_str(wat).unwrap(),
        cutscenes: Vec::new(),
        names: Vec::new(),
        settings: "{}".into(),
        assets: GameAssets {
            rooms: vec![sariema_format::script::GameRoom::of("demo", &demo_room())],
            ..GameAssets::default()
        },
    })
}

fn host(stage: &mut Stage, bytes: &[u8]) -> GameHost {
    let renderer = stage.renderer_mut();
    GameHost::load(
        bytes,
        0,
        None,
        FeedSettings::default(),
        renderer,
        (W, H),
        true,
    )
    .unwrap()
}

#[test]
fn a_replay_reaches_the_same_hashes_every_time() {
    let target = Target::new();
    let bytes = pack(IDLE);
    let mut runs = Vec::new();
    for _ in 0..2 {
        let mut stage = target.stage("");
        let mut game = host(&mut stage, &bytes);
        game.replay(NO_INPUT, 120).unwrap();
        runs.push(game.hashes());
    }
    assert_eq!(runs[0]["steps"], 120);
    assert_eq!(runs[0], runs[1]);
}

#[test]
fn a_frame_draws_the_games_room_as_the_room_would_be_drawn() {
    let target = Target::new();
    let mut stage = target.stage(r#"{"quality":"high"}"#);
    let mut game = host(&mut stage, &pack(IDLE));
    game.draw(stage.renderer_mut(), &target.view).unwrap();
    let golden = image::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/golden/demo_lit.png"
    ))
    .unwrap()
    .to_rgba8();
    let d = compare_images(&target.image(), &golden).unwrap();
    assert!(
        d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO,
        "{d:?}"
    );
}

#[test]
fn a_page_frame_runs_the_steps_its_time_owes() {
    let target = Target::new();
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &pack(IDLE));
    assert_eq!(game.advance(1000.0).unwrap().steps, 0);
    let mut steps = 0;
    for frame in 1..=60 {
        steps += game
            .advance(1000.0 + frame as f64 * 1000.0 / 60.0)
            .unwrap()
            .steps;
    }
    assert_eq!(steps, 60);
    assert_eq!(game.steps(), 60);
    game.pause();
    assert_eq!(game.advance(60_000.0).unwrap().steps, 0);
}

#[test]
fn a_game_that_never_returns_is_stopped_by_its_budget() {
    let target = Target::new();
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &pack(FOREVER));
    let start = Instant::now();
    game.advance(0.0).unwrap();
    let e = game.advance(1000.0 / 60.0).unwrap_err();
    assert_eq!(e.code, "E_GAME_BUDGET", "{e}");
    assert!(start.elapsed().as_secs() < 30, "{:?}", start.elapsed());
    assert_eq!(
        game.advance(2000.0 / 60.0).unwrap_err().code,
        "E_GAME_BUDGET"
    );
    assert_eq!(game.stats()["stopped"], "E_GAME_BUDGET");
}

#[test]
fn a_replay_goes_on_only_with_the_log_it_started_with() {
    let target = Target::new();
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &pack(IDLE));
    game.replay(NO_INPUT, 10).unwrap();
    let other = r#"{"changes":[{"step":12,"down":["action"]}]}"#;
    assert_eq!(game.replay(other, 20).unwrap_err().code, "E_USAGE");
    assert_eq!(game.steps(), 10);
    game.replay(NO_INPUT, 20).unwrap();
    assert_eq!(game.steps(), 20);
}

#[test]
fn a_replay_ends_at_the_step_that_stops_its_game() {
    let target = Target::new();
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &pack(LATE));
    let e = game.replay(NO_INPUT, 40).unwrap_err();
    assert_eq!(e.code, "E_GAME_BUDGET", "{e}");
    assert!(e.message.starts_with("step 29: "), "{e}");
    assert_eq!(game.steps(), 30);
    assert_eq!(game.replay(NO_INPUT, 40).unwrap_err().code, "E_GAME_BUDGET");
    assert_eq!(game.steps(), 30);
}

#[test]
fn a_new_renderer_takes_the_game_on_where_it_was() {
    let target = Target::new();
    let bytes = pack(IDLE);
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &bytes);
    game.replay(NO_INPUT, 45).unwrap();
    let before = game.hashes();
    let mut fresh = target.stage("");
    game.reload_view(fresh.renderer_mut()).unwrap();
    assert_eq!(game.hashes(), before);
    game.draw(fresh.renderer_mut(), &target.view).unwrap();
    game.replay(NO_INPUT, 46).unwrap();
    assert_eq!(game.steps(), 46);
}

#[cfg(feature = "alloc-count")]
#[test]
fn a_replayed_step_allocates_nothing_once_steady() {
    let target = Target::new();
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &pack(IDLE));
    game.replay(NO_INPUT, 600).unwrap();
    let steps = game.stats()["step_allocations"].clone();
    let late: Vec<_> = (steps.as_array().unwrap().iter())
        .filter(|s| s[0].as_u64().unwrap() >= 60)
        .collect();
    assert!(late.is_empty(), "{steps}");
}

/// Each with the step that sent it.
type Sent = Vec<(u64, HostOut)>;
type Echoed = Vec<(u64, String)>;

/// What the echo game sent its host, apart from its echoes, and the save
/// events it echoed.
fn taken(game: &mut GameHost) -> (Sent, Echoed) {
    let (mut sent, mut saves) = (Vec::new(), Vec::new());
    for (step, message) in game.take_host() {
        let HostOut::Message { topic, payload } = &message else {
            sent.push((step, message));
            continue;
        };
        assert_eq!(topic, "events");
        for event in records::<Event>(payload).map(Result::unwrap) {
            match event {
                Event::SaveLoaded { key, value } => saves.push((step, format!("{key}={value:?}"))),
                Event::SaveLoadDone {} => saves.push((step, "done".into())),
                _ => {}
            }
        }
    }
    (sent, saves)
}

/// Page frames of `steps` steps each, `frames` times, from where `game` is.
fn frames(game: &mut GameHost, frames: u32, steps: u32) {
    let at = game.steps() as f64 * 1000.0 / 60.0;
    game.advance(at).unwrap();
    for frame in 1..=frames {
        game.advance(at + f64::from(frame * steps) * 1000.0 / 60.0)
            .unwrap();
    }
}

#[test]
fn the_page_keeps_a_save_and_answers_the_load_steps_later() {
    let target = Target::new();
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &pack(ECHO));
    game.answers_saves(true);
    frames(&mut game, 10, 1);
    assert_eq!(game.steps(), 10);
    let (sent, saves) = taken(&mut game);
    assert_eq!(
        sent,
        [
            (1, HostOut::LoadSaves),
            (
                2,
                HostOut::Save {
                    key: "best".into(),
                    value: vec![12]
                }
            )
        ]
    );
    assert!(
        saves.is_empty(),
        "nothing before the page answers: {saves:?}"
    );
    assert!(game.answer_saves(vec![("best".into(), vec![12])]));
    frames(&mut game, 3, 1);
    let (_, saves) = taken(&mut game);
    assert_eq!(saves, [(10, "best=[12]".into()), (10, "done".into())]);
}

#[test]
fn a_load_the_page_never_answers_is_answered_as_no_save() {
    let target = Target::new();
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &pack(ECHO));
    game.answers_saves(true);
    frames(&mut game, 80, 4);
    assert_eq!(game.steps(), 320);
    let (_, saves) = taken(&mut game);
    assert_eq!(saves, [(302, "done".into())]);
}

#[test]
fn without_a_save_handler_a_load_is_answered_at_once() {
    let target = Target::new();
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &pack(ECHO));
    frames(&mut game, 5, 1);
    let (_, saves) = taken(&mut game);
    assert_eq!(saves, [(2, "done".into())]);
}

#[test]
fn a_replay_answers_loads_as_a_headless_run_without_saves_does() {
    let target = Target::new();
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &pack(ECHO));
    game.answers_saves(true);
    game.replay(NO_INPUT, 5).unwrap();
    let (sent, saves) = taken(&mut game);
    assert_eq!(sent[0], (1, HostOut::LoadSaves));
    assert_eq!(saves, [(2, "done".into())]);
}
