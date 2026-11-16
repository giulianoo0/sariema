// SPDX-License-Identifier: Apache-2.0

mod common;

use common::{assert_rect, id, rect, screen, ui};
use sariema_ui::{SafeArea, Ui};
use serde_json::json;

fn hud() -> Ui {
    ui(json!({
        "nodes": [
            { "name": "backdrop", "absolute": true, "left": 0, "top": 0, "right": 0, "bottom": 0 },
            { "name": "hud", "safe": true, "children": [
                { "name": "chip", "absolute": true, "left": 16, "top": 12, "width": 200, "height": 64 },
                { "name": "pause", "absolute": true, "right": 16, "top": 12, "width": 52, "height": 52 },
                { "name": "stick", "absolute": true, "left": 16, "bottom": 16, "width": 150, "height": 180 },
                { "name": "band", "absolute": true, "left": 0, "right": 0, "bottom": 16,
                  "justify": "center", "children": [ { "name": "talk", "width": 450, "height": 90 } ] }
            ]}
        ]
    }))
}

fn notch(scale: f32) -> SafeArea {
    SafeArea {
        left: 47.0 * scale,
        top: 0.0,
        right: 47.0 * scale,
        bottom: 21.0 * scale,
    }
}

#[test]
fn safe_area_holds_the_hud_at_any_density() {
    for scale in [1.0, 2.0, 3.0] {
        let mut ui = hud();
        ui.set_screen(844.0 * scale, 390.0 * scale, notch(scale));
        ui.step();
        assert_eq!(ui.screen().scale, scale);
        assert_rect(rect(&ui, "backdrop"), [0.0, 0.0, 844.0, 390.0]);
        assert_rect(rect(&ui, "hud"), [47.0, 0.0, 750.0, 369.0]);
        assert_rect(rect(&ui, "chip"), [63.0, 12.0, 200.0, 64.0]);
        assert_rect(rect(&ui, "pause"), [729.0, 12.0, 52.0, 52.0]);
        assert_rect(rect(&ui, "stick"), [63.0, 173.0, 150.0, 180.0]);
        assert_rect(rect(&ui, "talk"), [197.0, 263.0, 450.0, 90.0]);
        let pause = id(&ui, "pause");
        let px = ui.screen_rect(pause).unwrap();
        assert_rect(
            px,
            [729.0 * scale, 12.0 * scale, 52.0 * scale, 52.0 * scale],
        );
    }
}

#[test]
fn reference_fits_and_the_longer_side_grows() {
    // 16:9, 4:3 and 21:9 landscape screens, without insets.
    let cases = [
        (
            667.0,
            375.0,
            667.0 / 844.0,
            [844.0, 375.0 / (667.0 / 844.0)],
        ),
        (
            1024.0,
            768.0,
            1024.0 / 844.0,
            [844.0, 768.0 / (1024.0 / 844.0)],
        ),
        (
            2560.0,
            1080.0,
            1080.0 / 390.0,
            [2560.0 / (1080.0 / 390.0), 390.0],
        ),
    ];
    for (w, h, scale, [uw, uh]) in cases {
        let mut ui = hud();
        screen(&mut ui, w, h);
        ui.step();
        assert!((ui.screen().scale - scale).abs() < 1e-5, "{w}×{h}");
        assert_rect(rect(&ui, "backdrop"), [0.0, 0.0, uw, uh]);
        assert_rect(rect(&ui, "pause"), [uw - 16.0 - 52.0, 12.0, 52.0, 52.0]);
        assert_rect(rect(&ui, "stick"), [16.0, uh - 16.0 - 180.0, 150.0, 180.0]);
        assert_rect(
            rect(&ui, "talk"),
            [(uw - 450.0) / 2.0, uh - 16.0 - 90.0, 450.0, 90.0],
        );
    }
}

#[test]
fn portrait_turns_the_reference() {
    for scale in [1.0, 3.0] {
        let mut ui = hud();
        screen(&mut ui, 390.0 * scale, 844.0 * scale);
        ui.step();
        assert_eq!(ui.screen().scale, scale);
        assert_rect(rect(&ui, "backdrop"), [0.0, 0.0, 390.0, 844.0]);
        assert_rect(rect(&ui, "pause"), [322.0, 12.0, 52.0, 52.0]);
        assert_rect(rect(&ui, "stick"), [16.0, 648.0, 150.0, 180.0]);
        // Wider than the screen: it shrinks, as flex items do.
        assert_rect(rect(&ui, "talk"), [0.0, 738.0, 390.0, 90.0]);
    }
}

#[test]
fn flex_row_with_padding_gap_grow_and_centring() {
    let mut ui = ui(json!({
        "nodes": [ { "name": "row", "width": 400, "height": 100, "padding": [10, 20], "gap": 8,
                     "align": "center", "children": [
            { "name": "a", "width": 50, "height": 30 },
            { "name": "b", "grow": 1, "height": 40 },
            { "name": "c", "width": 60, "height": 80 }
        ]}]
    }));
    screen(&mut ui, 844.0, 390.0);
    ui.step();
    assert_rect(rect(&ui, "a"), [20.0, 35.0, 50.0, 30.0]);
    assert_rect(rect(&ui, "b"), [78.0, 30.0, 234.0, 40.0]);
    assert_rect(rect(&ui, "c"), [320.0, 10.0, 60.0, 80.0]);
}

#[test]
fn column_wraps_and_justifies() {
    let mut ui = ui(json!({
        "nodes": [ { "name": "col", "direction": "column", "width": 100, "height": 100,
                     "justify": "between", "children": [
            { "name": "top", "height": 20 },
            { "name": "bottom", "height": 30 }
        ]},
        { "name": "wrap", "wrap": true, "width": 100, "gap": 10, "align_self": "start", "children": [
            { "name": "w1", "width": 60, "height": 10 },
            { "name": "w2", "width": 60, "height": 10 }
        ]}]
    }));
    screen(&mut ui, 844.0, 390.0);
    ui.step();
    assert_rect(rect(&ui, "top"), [0.0, 0.0, 100.0, 20.0]);
    assert_rect(rect(&ui, "bottom"), [0.0, 70.0, 100.0, 30.0]);
    assert_rect(rect(&ui, "w1"), [100.0, 0.0, 60.0, 10.0]);
    assert_rect(rect(&ui, "w2"), [100.0, 20.0, 60.0, 10.0]);
}

#[test]
fn removed_ids_stay_dead_and_names_free() {
    let mut ui = hud();
    let chip = id(&ui, "chip");
    let before = ui.len();
    let hud_id = id(&ui, "hud");
    ui.remove(hud_id).unwrap();
    assert_eq!(ui.len(), before - 6);
    assert!(ui.rect(chip).is_none());
    assert!(ui.node("chip").is_none());
    let again = ui
        .create(
            ui.root(),
            &serde_json::from_value(json!({ "name": "chip", "width": 5, "height": 5 })).unwrap(),
        )
        .unwrap();
    assert_ne!(again, chip, "a reused slot gets a new id");
    ui.step();
    assert_rect(ui.rect(again).unwrap(), [0.0, 0.0, 5.0, 5.0]);
}

#[test]
fn a_snapped_node_and_its_children_are_drawn_where_they_are_until_the_next_step() {
    let mut ui = ui(json!({
        "nodes": [
            { "name": "dock", "absolute": true, "left": 0, "right": 0, "bottom": 0, "direction": "column",
              "children": [
                { "name": "line", "height": 20, "fill": "#FFFFFF" },
                { "name": "bar", "height": 0, "fill": "#000000" }
              ]}
        ]
    }));
    ui.set_screen(844.0, 390.0, SafeArea::default());
    ui.step();
    ui.step();
    let (dock, bar, line) = (id(&ui, "dock"), id(&ui, "bar"), id(&ui, "line"));
    assert_eq!(ui.children(dock).collect::<Vec<_>>(), [line, bar]);
    ui.set(bar, sariema_ui::Prop::Height, 40.0).unwrap();
    let top = |ui: &mut Ui, alpha: f32| {
        let list = ui.draw(alpha);
        list.quads
            .iter()
            .map(|q| q.xform[5] + q.rect[1])
            .fold(f32::MAX, f32::min)
    };
    let moving = top(&mut ui, 0.5);
    ui.snap(dock).unwrap();
    let snapped = [top(&mut ui, 0.0), top(&mut ui, 0.5), top(&mut ui, 1.0)];
    assert_rect(rect(&ui, "line"), [0.0, 330.0, 844.0, 20.0]);
    assert!((moving - 350.0).abs() < 0.01, "interpolated: {moving}");
    assert_eq!(snapped, [330.0; 3]);
}
