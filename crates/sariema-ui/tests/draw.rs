// SPDX-License-Identifier: Apache-2.0

mod common;

use common::{NUNITO, near, screen, ui};
use sariema_ui::{
    ATLAS_TEXTURE, Color, Gradient, GradientKind, NO_TEXTURE, NodeDesc, Ui, UiSettings, kind,
};
use serde_json::{Value, json};

fn node(v: Value) -> NodeDesc {
    serde_json::from_value(v).unwrap()
}

fn rows(n: usize) -> Value {
    let rows: Vec<Value> = (0..n)
        .map(|i| {
            json!({ "fill": "#FFF8EC", "border": { "width": 2.5, "color": "#33261C" },
                         "radius": 16, "shadow": { "y": 3, "color": "#33261C61" }, "padding": 4,
                         "children": [ { "text": format!("Linha {i}: Olá"), "size": 13 } ] })
        })
        .collect();
    json!({ "fonts": common::font(), "nodes": [ { "direction": "column", "wrap": true, "children": rows } ] })
}

#[test]
fn nodes_sharing_the_atlas_draw_in_one_batch() {
    let mut ui = ui(rows(40));
    screen(&mut ui, 844.0, 390.0);
    let list = ui.draw(1.0);
    assert_eq!(list.batches.len(), 1);
    let b = list.batches[0];
    assert_eq!(b.texture, ATLAS_TEXTURE);
    assert_eq!(b.count as usize, list.quads.len());
    let glyphs = list.quads.iter().filter(|q| q.kind == kind::GLYPH).count();
    assert_eq!(list.quads.len(), 80 + glyphs, "a shadow and a box per row");
    assert_eq!(list.quads[0].params[1], 0.0, "a hard shadow, first");
}

fn with_image() -> Ui {
    let mut ui = Ui::new(UiSettings::default()).unwrap();
    ui.add_font("body", NUNITO.to_vec(), None).unwrap();
    let texel = [255u8, 255, 255, 255];
    ui.add_image("sheet", 30, 30, texel.repeat(900)).unwrap();
    screen(&mut ui, 844.0, 390.0);
    ui
}

#[test]
fn a_texture_or_scissor_change_starts_a_batch() {
    let mut ui = with_image();
    let root = ui.root();
    for desc in [
        json!({ "text": "Olhar" }),
        json!({ "image": "sheet", "width": 30, "height": 30 }),
        json!({ "text": "Soprar", "fill": "#fff" }),
        json!({ "width": 20, "height": 20, "fill": "#E0650F" }),
        json!({ "width": 50, "height": 50, "clip": true, "children": [ { "text": "Empurrar" } ] }),
    ] {
        ui.create(root, &node(desc)).unwrap();
    }
    let list = ui.draw(1.0);
    let textures: Vec<u32> = list.batches.iter().map(|b| b.texture).collect();
    assert_eq!(textures, [ATLAS_TEXTURE, 1, ATLAS_TEXTURE, ATLAS_TEXTURE]);
    assert_eq!(list.batches[2].scissor, [0, 0, 844, 390]);
    // The clip sits at a fractional x after the texts: rounded out.
    let clipped = list.batches[3].scissor;
    assert!(
        (50..=51).contains(&clipped[2]) && clipped[3] == 50,
        "{clipped:?}"
    );
}

#[test]
fn scissors_are_pixels_and_shapes_alone_have_no_texture() {
    let mut ui = with_image();
    ui.set_screen(1688.0, 780.0, Default::default());
    let root = ui.root();
    ui.create(root, &node(json!({ "absolute": true, "left": 100, "top": 100, "width": 50, "height": 50,
                                  "clip": true, "children": [ { "width": 80, "height": 80, "fill": "#000" } ] })))
        .unwrap();
    let list = ui.draw(1.0);
    assert_eq!(list.batches.len(), 1);
    assert_eq!(list.batches[0].texture, NO_TEXTURE);
    assert_eq!(list.batches[0].scissor, [200, 200, 100, 100]);
}

#[test]
fn nine_slice_keeps_its_corners() {
    let mut ui = with_image();
    let root = ui.root();
    ui.create(
        root,
        &node(
            json!({ "image": "sheet", "slice": [10, 10, 10, 10], "slice_scale": 0.5,
                                  "width": 200, "height": 60 }),
        ),
    )
    .unwrap();
    let list = ui.draw(1.0);
    assert_eq!(list.quads.len(), 9);
    assert_eq!(list.quads[0].rect, [0.0, 0.0, 5.0, 5.0]);
    assert_eq!(list.quads[4].rect, [5.0, 5.0, 190.0, 50.0]);
    assert_eq!(
        list.quads[4].uv,
        [10.0 / 30.0, 10.0 / 30.0, 20.0 / 30.0, 20.0 / 30.0]
    );
    assert_eq!(list.quads[8].rect, [195.0, 55.0, 5.0, 5.0]);
}

#[test]
fn shapes_carry_their_parameters() {
    let mut ui = with_image();
    let root = ui.root();
    for desc in [
        json!({ "width": 40, "height": 20, "radius": 100, "fill": "#fff", "opacity": 0.5 }),
        json!({ "width": 80, "height": 80, "ring": { "width": 5, "start": 90, "track": "#ffffff47" },
                "progress": 0.25 }),
        json!({ "width": 20, "height": 20, "stroke": { "points": [[0, 0], [10, 0], [10, 10]], "width": 2 },
                "progress": 0.75 }),
        json!({ "width": 10, "height": 10, "stroke": { "points": [[0, 0], [10, 0]], "width": 2 },
                "progress": 0 }),
    ] {
        ui.create(root, &node(desc)).unwrap();
    }
    let q = ui.draw(1.0).quads.clone();
    assert_eq!(q.len(), 4, "a box, a ring and two segments");
    assert_eq!(
        q[0].radii, [10.0; 4],
        "radii shrink together to fit, as in CSS"
    );
    assert_eq!(q[0].radii_y, [10.0; 4]);
    assert_eq!(q[0].fill[3], 0.5);
    assert_eq!(q[1].kind, kind::RING);
    assert!(near(q[1].params[1], std::f32::consts::FRAC_PI_2, 1e-6));
    assert_eq!(q[1].params[2], 0.25);
    assert_eq!(q[2].uv, [0.0, 0.0, 10.0, 0.0]);
    assert_eq!(
        q[3].uv,
        [10.0, 0.0, 10.0, 5.0],
        "the second segment, half drawn"
    );
    assert_eq!(q[3].params[0], 1.0);
}

#[test]
fn gradients_ride_in_the_box_quad() {
    let mut ui = with_image();
    let root = ui.root();
    let mut ids = Vec::new();
    for desc in [
        json!({ "width": 100, "height": 50, "opacity": 0.5,
                "gradient": { "type": "linear", "angle": 90, "from": "#ff0000", "to": "#0000ff" } }),
        json!({ "width": 100, "height": 50,
                "gradient": { "type": "linear", "from": "#fff", "to": "#000", "stops": [0.25, 0.75] } }),
        json!({ "width": 100, "height": 100, "border": { "width": 2.5, "color": "#33261C" },
                "gradient": { "type": "radial", "center": [0.5, 0.32], "from": "#FFD86B",
                              "to": "#F5A623", "stops": [0, 0.7] } }),
        json!({ "width": 844, "height": 390,
                "gradient": { "type": "radial", "center": [0.5, 0.46], "radius": [0.7, 0.62],
                              "from": "#0000", "to": "#080C1E8C", "stops": [0.55, 1] } }),
    ] {
        let mut desc = desc;
        desc["absolute"] = json!(true);
        ids.push(ui.create(root, &node(desc)).unwrap());
    }
    let list = ui.draw(1.0);
    assert_eq!(list.batches.len(), 1, "gradients are shapes: no texture");
    let q = list.quads.clone();
    assert_eq!(q.len(), 4);

    assert_eq!(q[0].kind, kind::LINEAR);
    assert_eq!(q[0].fill, [1.0, 0.0, 0.0, 0.5], "from, faded");
    assert_eq!(q[0].uv, [0.0, 0.0, 1.0, 0.5], "to, faded");
    let ends = |q: &sariema_ui::Quad| [q.params[2], q.params[3], q.gradient[0], q.gradient[1]];
    let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| near(*a, b, 1e-3));
    assert!(
        close(ends(&q[0]), [0.0, 25.0, 100.0, 25.0]),
        "{:?}",
        ends(&q[0])
    );
    assert!(
        close(ends(&q[1]), [50.0, 12.5, 50.0, 37.5]),
        "downwards by default, stops along the line: {:?}",
        ends(&q[1])
    );

    assert_eq!(q[2].kind, kind::RADIAL);
    assert_eq!(q[2].params[0], 2.5, "the border stays");
    assert_eq!([q[2].params[2], q[2].params[3]], [50.0, 32.0]);
    let farthest = (50.0f32 * 50.0 + 68.0 * 68.0).sqrt() * 0.7;
    assert!(near(q[2].gradient[0], farthest, 1e-3) && near(q[2].gradient[1], farthest, 1e-3));
    assert_eq!(q[2].gradient[2], 0.0);
    assert!(near(q[3].params[3], 179.4, 1e-3));
    assert!(near(q[3].gradient[0], 590.8, 1e-3) && near(q[3].gradient[1], 241.8, 1e-3));
    assert!(
        near(q[3].gradient[2], 0.55, 1e-6),
        "the inner stop over the outer"
    );

    let card = ids[1];
    let red = Color::parse("#E0650F").unwrap();
    ui.set_gradient(
        card,
        Some(Gradient {
            kind: GradientKind::Radial,
            from: Color::WHITE,
            to: red,
            ..Gradient::default()
        }),
    )
    .unwrap();
    let q = ui.draw(1.0).quads[1];
    assert_eq!((q.kind, q.uv), (kind::RADIAL, red.0));
    ui.set_gradient(card, None).unwrap();
    ui.set_fill(card, red).unwrap();
    let q = ui.draw(1.0).quads[1];
    assert_eq!((q.kind, q.fill), (kind::RECT, red.0));
}
