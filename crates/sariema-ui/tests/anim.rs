// SPDX-License-Identifier: Apache-2.0

mod common;

use common::{font, id, near, screen, ui};
use sariema_ui::{Animation, Ease, Prop, Ui};
use serde_json::json;

fn boxes() -> Ui {
    let mut ui = ui(json!({
        "fonts": font(),
        "nodes": [
            { "name": "a", "absolute": true, "width": 10, "height": 10, "fill": "#fff" },
            { "name": "b", "absolute": true, "width": 10, "height": 10 },
            { "name": "c", "absolute": true, "width": 10, "height": 10 },
            { "name": "line", "absolute": true, "text": "Olá. Tudo bem?", "reveal": 0 }
        ],
        "animations": {
            "words": [ { "nodes": ["a", "b", "c"], "stagger": 0.045, "prop": "y", "from": 5, "to": 0,
                         "duration": 0.24 } ],
            "chain": [ { "node": "a", "prop": "opacity", "from": 0, "to": 1, "duration": 0.2 },
                       { "node": "a", "prop": "x", "to": 30, "duration": 0.1, "after": true } ],
            "type": [ { "node": "line", "prop": "reveal", "typewriter": { "rate": 30, "pause": 0.07 } } ],
            "paint": [ { "node": "a", "prop": "fill", "to": "#2F9E41", "duration": 0.18, "ease": "in_out" } ]
        }
    }));
    screen(&mut ui, 844.0, 390.0);
    ui
}

/// The property after each of `steps` steps.
fn run(ui: &mut Ui, name: &str, prop: Prop, steps: usize) -> Vec<f32> {
    let node = id(ui, name);
    (0..steps)
        .map(|_| {
            ui.step();
            ui.value(node, prop).unwrap()
        })
        .collect()
}

#[test]
fn tweens_end_exactly_on_their_target() {
    let mut ui = boxes();
    let a = id(&ui, "a");
    ui.animate(
        a,
        Prop::Y,
        Animation::tween(0.0, 0.26, Ease::OUT).from(-14.0),
    )
    .unwrap();
    let ys = run(&mut ui, "a", Prop::Y, 20);
    assert!(ys.windows(2).all(|w| w[1] >= w[0]), "{ys:?}");
    assert!(ys[14] < 0.0, "still moving at 0.25 s");
    assert_eq!(ys[15], 0.0, "done at 0.26 s, rounded up to the step");
    assert_eq!(ui.animating(), 0);
    ui.play("paint").unwrap();
    run(&mut ui, "a", Prop::Red, 11);
    let green = [0x2F, 0x9E, 0x41].map(|c| c as f32 / 255.0);
    for (p, c) in [Prop::Red, Prop::Green, Prop::Blue].into_iter().zip(green) {
        assert_eq!(ui.value(a, p), Some(c));
    }
}

#[test]
fn springs_settle_within_bounds() {
    let mut ui = boxes();
    let a = id(&ui, "a");
    ui.set(a, Prop::X, 100.0).unwrap();
    ui.animate(a, Prop::X, Animation::spring(0.0, 0.35, 1.0))
        .unwrap();
    let xs = run(&mut ui, "a", Prop::X, 60);
    assert!(
        xs.iter().all(|&x| x >= 0.0),
        "critically damped: no overshoot"
    );
    assert!(xs[29] < 5.0, "most of the way in half a second: {}", xs[29]);
    assert_eq!(xs[59], 0.0, "at rest within a second");

    ui.set(a, Prop::ScaleX, 0.5).unwrap();
    ui.animate(a, Prop::ScaleX, Animation::spring(1.0, 0.5, 0.62))
        .unwrap();
    let s = run(&mut ui, "a", Prop::ScaleX, 120);
    // Damping ratio 0.62 overshoots by e^(−π·0.62/√(1−0.62²)) = 8.35 % of
    // the 0.5 travelled.
    let peak = s.iter().copied().fold(0.0, f32::max);
    assert!(near(peak - 1.0, 0.0418, 0.004), "{peak}");
    assert_eq!(s[119], 1.0, "at rest within two seconds");
}

#[test]
fn interrupting_continues_from_where_it_is() {
    let mut ui = boxes();
    let a = id(&ui, "a");
    ui.animate(a, Prop::X, Animation::tween(100.0, 1.0, Ease::LINEAR))
        .unwrap();
    let before = run(&mut ui, "a", Prop::X, 30);
    ui.animate(a, Prop::X, Animation::tween(0.0, 1.0, Ease::LINEAR))
        .unwrap();
    let after = run(&mut ui, "a", Prop::X, 60);
    let step = 100.0 / 60.0;
    assert!(near(before[29], 50.0, 1e-3));
    assert!(
        (after[0] - before[29]).abs() <= step + 1e-3,
        "{} then {}",
        before[29],
        after[0]
    );
    assert_eq!(after[59], 0.0);

    // A spring taking over a spring keeps its velocity: the value keeps
    // going the old way for a moment.
    ui.animate(a, Prop::Y, Animation::spring(100.0, 0.4, 1.0))
        .unwrap();
    let ys = run(&mut ui, "a", Prop::Y, 10);
    ui.animate(a, Prop::Y, Animation::spring(0.0, 0.4, 1.0))
        .unwrap();
    let back = run(&mut ui, "a", Prop::Y, 3);
    assert!(back[0] > ys[9], "{} then {}", ys[9], back[0]);
    assert!(back[2] < back[0]);
}

#[test]
fn staggers_and_sequences_start_on_their_steps() {
    let mut ui = boxes();
    ui.play("words").unwrap();
    let first_move = |ys: &[f32]| ys.iter().position(|&y| y != 5.0).unwrap();
    let mut steps = Vec::new();
    let (na, nb, nc) = (id(&ui, "a"), id(&ui, "b"), id(&ui, "c"));
    for _ in 0..30 {
        ui.step();
        steps.push([na, nb, nc].map(|n| ui.value(n, Prop::Y).unwrap()));
    }
    let col = |k: usize| steps.iter().map(|s| s[k]).collect::<Vec<_>>();
    // Starts at 0, 0.045 and 0.09 s: steps 1, 3 and 6 (counting from 1).
    assert_eq!(first_move(&col(0)), 0);
    assert_eq!(col(1)[1], 5.0, "b waits at its from value");
    assert_eq!(first_move(&col(1)), 2);
    assert_eq!(first_move(&col(2)), 5);
    assert_eq!(col(2)[29], 0.0);

    let mut ui = boxes();
    ui.play("chain").unwrap();
    let xs = run(&mut ui, "a", Prop::X, 20);
    assert_eq!(xs[11], 0.0, "x waits for the 0.2 s fade");
    assert!(xs[12] > 0.0);
    assert_eq!(xs[18], 30.0);
}

#[test]
fn typewriter_pauses_after_punctuation() {
    let mut ui = boxes();
    ui.play("type").unwrap();
    let r = run(&mut ui, "line", Prop::Reveal, 40);
    // "Olá." is 4 characters: 4/30 s = 8 steps, then 0.07 s of pause.
    assert!(near(r[7], 4.0, 1e-4), "{r:?}");
    assert!(near(r[10], 4.0, 1e-4), "pausing: {r:?}");
    assert!(r[12] > 4.0);
    // 14 characters at 30/s plus the pause: 0.537 s, step 33.
    assert!(r[31] < 14.0);
    assert_eq!(r[32], 14.0);
}

#[test]
fn draw_interpolates_between_steps() {
    let mut ui = boxes();
    let a = id(&ui, "a");
    ui.step();
    ui.animate(a, Prop::X, Animation::tween(60.0, 1.0, Ease::LINEAR))
        .unwrap();
    ui.step();
    let x = |ui: &mut Ui, alpha: f32| ui.draw(alpha).quads[0].xform[4];
    assert!(near(x(&mut ui, 0.0), 0.0, 1e-4));
    assert!(near(x(&mut ui, 0.5), 0.5, 1e-4));
    assert!(near(x(&mut ui, 1.0), 1.0, 1e-4));
}

#[test]
fn same_inputs_same_frames() {
    let session = || {
        let mut ui = boxes();
        let a = id(&ui, "a");
        ui.play("words").unwrap();
        ui.play("type").unwrap();
        ui.animate(a, Prop::Rotation, Animation::spring(45.0, 0.3, 0.7))
            .unwrap();
        let mut frames = Vec::new();
        for step in 0..90 {
            if step == 20 {
                ui.animate(a, Prop::Rotation, Animation::spring(-10.0, 0.3, 0.7))
                    .unwrap();
            }
            ui.step();
            frames.push(ui.draw(0.37).quads.clone());
        }
        frames
    };
    assert_eq!(session(), session());
}

#[test]
fn reduced_motion_keeps_fades_and_drops_moves() {
    let mut ui = ui(json!({ "settings": { "reduced_motion": true },
                            "nodes": [ { "name": "a", "width": 10, "height": 10 } ] }));
    let a = id(&ui, "a");
    ui.animate(a, Prop::Y, Animation::tween(0.0, 0.3, Ease::OUT).from(14.0))
        .unwrap();
    ui.animate(
        a,
        Prop::Opacity,
        Animation::tween(1.0, 0.3, Ease::OUT).from(0.0),
    )
    .unwrap();
    ui.step();
    assert_eq!(ui.value(a, Prop::Y), Some(0.0));
    let o = ui.value(a, Prop::Opacity).unwrap();
    assert!(o > 0.0 && o < 1.0);
}
