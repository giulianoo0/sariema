// SPDX-License-Identifier: Apache-2.0

mod common;

use common::{font, id, near, rect, screen, try_ui, ui};
use sariema_ui::{Quad, Ui, kind};
use serde_json::{Value, json};

fn one(text: &str, extra: Value) -> Ui {
    let mut node =
        json!({ "name": "t", "absolute": true, "text": text, "size": 17, "line_height": 1.18 });
    node.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let mut ui = ui(json!({ "fonts": font(), "nodes": [node] }));
    screen(&mut ui, 844.0, 390.0);
    ui.step();
    ui
}

fn glyphs(ui: &mut Ui) -> Vec<Quad> {
    let list = ui.draw(1.0);
    list.quads
        .iter()
        .filter(|q| q.kind == kind::GLYPH)
        .copied()
        .collect()
}

const LONG: &str = "Encurrale o guaxinim perto da casa na árvore";

#[test]
fn wraps_within_the_width_and_reserves_whole_lines() {
    let ui = one(LONG, json!({ "max_width": 300 }));
    let t = id(&ui, "t");
    let lines = ui.lines(t).unwrap();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines.iter().all(|l| l.width <= 300.0), "{lines:?}");
    assert!(lines[0].width > 250.0, "{lines:?}");
    let [_, _, w, h] = rect(&ui, "t");
    let widest = lines.iter().map(|l| l.width).fold(0.0, f32::max);
    assert!(near(w, widest, 1e-3), "shrinks to its widest line");
    assert!(near(h, 2.0 * 17.0 * 1.18, 1e-3));
    let unwrapped = one(LONG, json!({}));
    let line = unwrapped.lines(id(&unwrapped, "t")).unwrap();
    assert_eq!(line.len(), 1);
    assert!(near(line[0].width, rect(&unwrapped, "t")[2], 1e-3));
    assert!(line[0].width > 300.0);
}

#[test]
fn max_lines_and_newlines() {
    let ui = one(LONG, json!({ "max_width": 120, "max_lines": 2 }));
    assert_eq!(ui.lines(id(&ui, "t")).unwrap().len(), 2);
    assert!(near(rect(&ui, "t")[3], 2.0 * 17.0 * 1.18, 1e-3));
    let ui = one("Olá\nmundo\n", json!({}));
    let lines = ui.lines(id(&ui, "t")).unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[2].width, 0.0);
}

#[test]
fn alignment_places_lines_in_the_box() {
    // Glyph quads extend past the ink by the field's padding and the side
    // bearings; a few units of tolerance.
    let tol = 4.0;
    for (align, check) in [("left", 0), ("center", 1), ("right", 2)] {
        let mut ui = one("Feito!", json!({ "width": 300, "text_align": align }));
        let quads = glyphs(&mut ui);
        let left = quads.iter().map(|q| q.rect[0]).fold(f32::MAX, f32::min);
        let right = quads
            .iter()
            .map(|q| q.rect[0] + q.rect[2])
            .fold(f32::MIN, f32::max);
        match check {
            0 => assert!(left.abs() < tol, "{align}: {left}"),
            1 => assert!(
                (left - (300.0 - right)).abs() < tol,
                "{align}: {left} {right}"
            ),
            _ => assert!((300.0 - right).abs() < tol, "{align}: {right}"),
        }
    }
}

#[test]
fn kerning_pulls_pairs_together() {
    let width = |s: &str| {
        let ui = one(s, json!({}));
        ui.lines(id(&ui, "t")).unwrap()[0].width
    };
    for (pair, a, b) in [("AV", "A", "V"), ("To", "T", "o")] {
        assert!(width(pair) < width(a) + width(b) - 0.3, "{pair}");
    }
}

#[test]
fn covers_portuguese_and_full_latin() {
    let ui = one(
        "Ação, você, pôr-do-sol, mãe, à toa; Über, Ñandú — «ç» “ã” ‘õ’ …",
        json!({}),
    );
    assert_eq!(ui.missing_glyphs(id(&ui, "t")), Some(0));
    let mut ui = one("", json!({}));
    let t = id(&ui, "t");
    let missing: Vec<char> = ('\u{0020}'..='\u{007E}')
        .chain('\u{00A0}'..='\u{017F}')
        .filter(|c| {
            ui.set_text(t, &c.to_string()).unwrap();
            ui.missing_glyphs(t) != Some(0)
        })
        .collect();
    // Nunito lacks only the Dutch ĳ ligature and the long s.
    assert_eq!(missing, ['ĳ', 'ſ']);
    let ui = one("日本語", json!({}));
    assert_eq!(
        ui.missing_glyphs(id(&ui, "t")),
        Some(3),
        "Nunito has no CJK"
    );
}

#[test]
fn weight_axis_and_unknown_fonts() {
    let mut ui = ui(json!({
        "fonts": {
            "regular": { "file": "Nunito.ttf", "weight": 400 },
            "black": { "file": "Nunito.ttf", "weight": 900 }
        },
        "nodes": [
            { "name": "r", "absolute": true, "text": "Objetivo", "font": "regular" },
            { "name": "b", "absolute": true, "text": "Objetivo", "font": "black" }
        ]
    }));
    screen(&mut ui, 844.0, 390.0);
    ui.step();
    assert!(rect(&ui, "b")[2] > rect(&ui, "r")[2] + 2.0);
    let err = try_ui(json!({ "fonts": font(), "nodes": [{ "text": "x", "font": "nope" }] }));
    assert_eq!(err.err().map(|e| e.code()), Some("E_UI_UNKNOWN"));
    let err = try_ui(
        json!({ "fonts": { "f": { "file": "Nunito.ttf", "weight": 650 },
                                        "g": { "file": "Nunito.ttf" } },
                             "nodes": [{ "text": "x" }] }),
    );
    assert_eq!(err.err().map(|e| e.code()), Some("E_SCHEMA"));
}

#[test]
fn set_text_lays_out_again() {
    let mut ui = one("Olhar", json!({}));
    let t = id(&ui, "t");
    let before = rect(&ui, "t")[2];
    ui.set_text(t, "Empurrar").unwrap();
    ui.step();
    assert!(rect(&ui, "t")[2] > before);
    assert_eq!(ui.text(t), Some("Empurrar"));
}
