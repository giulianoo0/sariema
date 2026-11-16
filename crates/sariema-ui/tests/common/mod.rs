// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use sariema_ui::{NodeId, SafeArea, Ui, UiDesc};
use serde_json::Value;

pub const NUNITO: &[u8] = include_bytes!("../../fonts/Nunito.ttf");

/// A UI from a description whose only font file is `Nunito.ttf`.
pub fn ui(desc: Value) -> Ui {
    try_ui(desc).unwrap()
}

pub fn try_ui(desc: Value) -> Result<Ui, sariema_ui::UiError> {
    let desc: UiDesc = serde_json::from_value(desc).expect("a valid description");
    Ui::from_desc(&desc, |file| match file {
        "Nunito.ttf" => Ok(NUNITO.to_vec()),
        _ => Err(format!("no file {file}")),
    })
}

pub fn font() -> Value {
    serde_json::json!({ "body": { "file": "Nunito.ttf", "weight": 650 } })
}

pub fn screen(ui: &mut Ui, w: f32, h: f32) {
    ui.set_screen(w, h, SafeArea::default());
}

pub fn id(ui: &Ui, name: &str) -> NodeId {
    ui.node(name).unwrap_or_else(|| panic!("no node {name}"))
}

pub fn rect(ui: &Ui, name: &str) -> [f32; 4] {
    ui.rect(id(ui, name)).unwrap()
}

pub fn near(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

pub fn assert_rect(actual: [f32; 4], expected: [f32; 4]) {
    for (a, e) in actual.iter().zip(expected) {
        assert!(near(*a, e, 1e-3), "{actual:?} is not {expected:?}");
    }
}
