// SPDX-License-Identifier: Apache-2.0

//! Step and draw-list time of a 200-node screen: static, with 50 nodes
//! moving and fading, and with 50 nodes changing size (a layout per step).
//!
//! `cargo run --release -p sariema-ui --example ui_bench`

use std::time::Instant;

use sariema_ui::{Animation, Ease, NodeId, Prop, SafeArea, Ui, UiDesc};
use serde_json::{Value, json};

const FONT: &[u8] = include_bytes!("../fonts/Nunito.ttf");
const FRAMES: u32 = 3000;

fn screen() -> Ui {
    let cards: Vec<Value> = (0..50)
        .map(|i| {
            json!({ "name": format!("card{i}"), "width": 150, "height": 30, "align": "center", "gap": 6,
                    "padding": 4, "fill": "#FFF8EC", "border": { "width": 2, "color": "#33261C" },
                    "radius": { "x": [15, 19, 16, 18], "y": [18, 15, 19, 16] },
                    "shadow": { "y": 3, "color": "#33261C61" },
                    "children": [
                        { "name": format!("disc{i}"), "width": 20, "height": 20, "radius": 10, "fill": "#2F9E41" },
                        { "text": format!("Item {i}: olá"), "size": 13, "fill": "#33261C", "grow": 1 },
                        { "width": 18, "height": 18, "ring": { "width": 3, "track": "#33261C33" },
                          "progress": 0.6, "fill": "#E0650F" }
                    ] })
        })
        .collect();
    let desc: UiDesc = serde_json::from_value(json!({
        "fonts": { "body": { "file": "Nunito.ttf", "weight": 650 } },
        "nodes": [ { "wrap": true, "gap": 4, "padding": 4, "children": cards } ]
    }))
    .expect("a valid description");
    let mut ui = Ui::from_desc(&desc, |_| Ok(FONT.to_vec())).expect("loads");
    ui.set_screen(844.0, 390.0, SafeArea::default());
    ui
}

/// Mean microseconds per step + draw over `FRAMES` frames; `kick` runs
/// before each frame.
fn time(ui: &mut Ui, mut kick: impl FnMut(&mut Ui, u32)) -> f64 {
    for f in 0..60 {
        kick(ui, f);
        ui.step();
        ui.draw(0.5);
    }
    let start = Instant::now();
    for f in 0..FRAMES {
        kick(ui, f);
        ui.step();
        std::hint::black_box(ui.draw(0.5));
    }
    start.elapsed().as_secs_f64() * 1e6 / f64::from(FRAMES)
}

fn ids(ui: &Ui, prefix: &str) -> Vec<NodeId> {
    (0..50)
        .map(|i| ui.node(&format!("{prefix}{i}")).unwrap())
        .collect()
}

fn main() {
    let mut ui = screen();
    let start = Instant::now();
    let list = ui.draw(1.0);
    let first = start.elapsed().as_secs_f64() * 1e6;
    let (quads, batches) = (list.quads.len(), list.batches.len());
    println!(
        "nodes {}, quads {quads}, batches (draw calls) {batches}",
        ui.len()
    );
    println!("first layout + draw list: {first:.0} µs");

    let still = time(&mut ui, |_, _| {});
    println!("static: {still:.1} µs per frame");

    let mut ui = screen();
    let cards = ids(&ui, "card");
    let moving = time(&mut ui, |ui, f| {
        if f % 40 == 0 {
            let up = (f / 40) % 2 == 0;
            for &c in &cards {
                let to = if up { -6.0 } else { 0.0 };
                ui.animate(c, Prop::Y, Animation::tween(to, 0.5, Ease::IN_OUT))
                    .unwrap();
                ui.animate(
                    c,
                    Prop::Opacity,
                    Animation::spring(if up { 0.6 } else { 1.0 }, 0.4, 0.8),
                )
                .unwrap();
            }
        }
    });
    println!("50 nodes moving and fading: {moving:.1} µs per frame");

    let mut ui = screen();
    let discs = ids(&ui, "disc");
    let sizing = time(&mut ui, |ui, f| {
        if f % 40 == 0 {
            let to = if (f / 40) % 2 == 0 { 28.0 } else { 20.0 };
            for &d in &discs {
                ui.animate(d, Prop::Width, Animation::tween(to, 0.5, Ease::IN_OUT))
                    .unwrap();
            }
        }
    });
    println!("50 nodes changing width (layout every step): {sizing:.1} µs per frame");
}
