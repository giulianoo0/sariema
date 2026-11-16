// SPDX-License-Identifier: Apache-2.0

//! A game pack played by the web host, natively: replays hash alike, a
//! frame draws the game's room, a page frame runs the steps its time owes,
//! a game that never returns is stopped by its budget, and a new renderer
//! takes the game on where it was.

mod common;

use std::time::Instant;

use common::{H, Target, W};
use sariema_format::demo::demo_room;
use sariema_format::script::{BATCH_VERSION, GameAssets, GamePack, encode_game};
use sariema_render::{GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, compare_images};
use sariema_web::feed::FeedSettings;
use sariema_web::game::GameHost;
use sariema_web::stage::Stage;

const IDLE: &str = r#"(module (memory (export "memory") 1) (func (export "sariema_tick")))"#;
const FOREVER: &str =
    r#"(module (memory (export "memory") 1) (func (export "sariema_tick") (loop $l (br $l))))"#;
const NO_INPUT: &str = r#"{"changes":[]}"#;

fn pack(wat: &str) -> Vec<u8> {
    encode_game(&GamePack {
        name: "test".into(),
        batch_version: BATCH_VERSION,
        wasm: wat::parse_str(wat).unwrap(),
        cutscenes: Vec::new(),
        names: Vec::new(),
        settings: "{}".into(),
        assets: GameAssets {
            room: Some(sariema_format::encode(&demo_room())),
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
fn a_new_renderer_takes_the_game_on_where_it_was() {
    let target = Target::new();
    let bytes = pack(IDLE);
    let mut stage = target.stage("");
    let mut game = host(&mut stage, &bytes);
    game.replay(NO_INPUT, 45).unwrap();
    let before = game.hashes();
    let mut fresh = target.stage("");
    game.reload_view(&bytes, fresh.renderer_mut()).unwrap();
    assert_eq!(game.hashes(), before);
    game.draw(fresh.renderer_mut(), &target.view).unwrap();
    game.replay(NO_INPUT, 46).unwrap();
    assert_eq!(game.steps(), 46);
}
