// SPDX-License-Identifier: Apache-2.0

//! The engine reads only logic that says it was built against this batch
//! version, and tells the author how to fix one that was not.

use sariema_format::script::{BATCH_VERSION, GameAssets, GamePack, decode_game, encode_game};
use sariema_runtime::game::start;

fn pack(batch_version: u32, export: Option<u32>) -> GamePack {
    let version = export
        .map(|v| format!(r#"(func (export "sariema_batch_version") (result i32) (i32.const {v}))"#))
        .unwrap_or_default();
    GamePack {
        name: "test".into(),
        batch_version,
        wasm: wat::parse_str(format!(
            r#"(module (memory (export "memory") 1) (func (export "sariema_tick")) {version})"#
        ))
        .unwrap(),
        cutscenes: Vec::new(),
        names: Vec::new(),
        settings: "{}".into(),
        assets: GameAssets::default(),
    }
}

fn refusal(pack: GamePack) -> String {
    let error = start(pack, 1).err().expect("refused");
    assert_eq!(error.code, "E_BATCH_VERSION");
    error.message
}

#[test]
fn logic_built_against_another_version_is_refused_with_how_to_fix_it() {
    let message = refusal(pack(BATCH_VERSION, Some(BATCH_VERSION - 1)));
    assert!(message.contains("rebuild the game"), "{message}");
}

#[test]
fn logic_that_does_not_say_its_version_is_refused() {
    let message = refusal(pack(BATCH_VERSION, None));
    assert!(message.contains("sariema_batch_version"), "{message}");
    assert!(message.contains("rebuild the game"), "{message}");
}

#[test]
fn a_pack_stamped_with_another_version_says_how_to_fix_it() {
    let error = decode_game(&encode_game(&pack(BATCH_VERSION + 1, Some(BATCH_VERSION))))
        .expect_err("refused");
    assert_eq!(error.code(), "E_BATCH_VERSION");
    assert!(error.to_string().contains("rebuild the game"), "{error}");
}

#[test]
fn logic_built_against_this_version_starts() {
    assert!(start(pack(BATCH_VERSION, Some(BATCH_VERSION)), 1).is_ok());
}
