// SPDX-License-Identifier: Apache-2.0

//! The engine's layer over the game's HUD, judged by what it draws: the
//! dialogue line sits above the cinema bar, the game's HUD steps aside
//! while a cutscene holds control, and the bars are drawn at the step's
//! height between steps.

mod common;

use sariema_engine::{Assets, Engine, EngineSettings};
use sariema_format::script::{Command, Controller, Event, Hud as HudAsset, name_id};
use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::input::Actions;
use sariema_runtime::sim::{Logic, Simulation};
use sariema_ui::{DrawList, kind};

use common::{assets, entity, native, send, spawn_hero, wired};

const TALK: &str = r#"{
  "name": "talk",
  "roles": { "hero": { "start": { "position": [2.5, 0, -1.25], "yaw": 0 } } },
  "steps": [
    { "letterbox": { "to": 1, "duration": 0.2 } },
    { "line": { "speaker": "hero", "text": "Mind the sill.", "advance": "auto", "duration": 1 } },
    { "letterbox": { "to": 0, "duration": 0.2 } }
  ]
}"#;

/// A box drawn: its fill and its bounds in pixels, left, top, right, bottom.
#[derive(Clone, Copy, Debug)]
struct Drawn {
    fill: [f32; 4],
    bounds: [f32; 4],
}

fn boxes(list: &DrawList) -> Vec<Drawn> {
    list.quads
        .iter()
        .filter(|q| q.kind == kind::RECT)
        .map(|q| {
            let [x, y, w, h] = q.rect;
            let [a, b, c, d, tx, ty] = q.xform;
            let corner = |x: f32, y: f32| (a * x + c * y + tx, b * x + d * y + ty);
            let corners = [
                corner(x, y),
                corner(x + w, y),
                corner(x, y + h),
                corner(x + w, y + h),
            ];
            let (mut l, mut t, mut r, mut bo) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (px, py) in corners {
                (l, t, r, bo) = (l.min(px), t.min(py), r.max(px), bo.max(py));
            }
            Drawn {
                fill: q.fill,
                bounds: [l, t, r, bo],
            }
        })
        .collect()
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

fn drawn<L: Logic>(engine: &mut Engine<L>, alpha: f32) -> Vec<Drawn> {
    boxes(engine.hud_mut().draw(alpha))
}

/// The bottom cinema bar: opaque black, the screen's width, on its bottom.
fn bottom_bar(boxes: &[Drawn], width: f32, height: f32) -> Option<Drawn> {
    boxes.iter().copied().find(|b| {
        near(b.fill, [0.0, 0.0, 0.0, 1.0])
            && (b.bounds[2] - b.bounds[0] - width).abs() < 0.5
            && (b.bounds[3] - height).abs() < 0.5
            && b.bounds[1] > height / 2.0
    })
}

/// Spawns the hero where the cutscene starts it, binds it and plays.
fn play_talk(out: &mut Vec<u8>) {
    spawn_hero(out, [2.5, 0.0, -1.25], 0.0);
    send(
        out,
        Command::CutsceneBind {
            role: name_id("hero"),
            entity: entity(0),
        },
    );
    send(
        out,
        Command::CutscenePlay {
            cutscene: name_id("talk"),
        },
    );
}

fn play_talk_at(at: u64) -> impl Logic {
    native(move |n, _, out| {
        if n == at {
            play_talk(out);
        }
    })
}

fn talk_engine<L: Logic>(logic: L, assets: Assets, settings: EngineSettings) -> Engine<L> {
    let cutscenes = vec![compile_json(TALK).unwrap().0];
    let sim = Simulation::new(wired(), cutscenes, logic, 1);
    Engine::new(sim, assets, settings).unwrap()
}

#[test]
fn the_dialogue_line_is_drawn_above_the_cinema_bar() {
    let mut engine = talk_engine(play_talk_at(0), assets(), EngineSettings::default());
    let actions = Actions::for_settings(&engine.state().settings.input);
    let screen = engine.hud().ui.screen();
    let (width, height) = (screen.width, screen.height);
    let mut checked = 0;
    for _ in 0..90 {
        engine.step(&actions);
        let s = engine.state();
        if s.dialogue.line.is_none() || s.screen.letterbox.value() < 1.0 {
            continue;
        }
        let boxes = drawn(&mut engine, 0.5);
        let bar = bottom_bar(&boxes, width, height).expect("the bottom bar is drawn");
        let line = boxes
            .iter()
            .find(|b| b.fill[0..3] == [0.0; 3] && b.fill[3] > 0.5 && b.fill[3] < 0.9)
            .expect("the dialogue box is drawn");
        assert!(
            line.bounds[3] <= bar.bounds[1],
            "the line's box {:?} reaches into the bar {:?}",
            line.bounds,
            bar.bounds
        );
        checked += 1;
    }
    assert!(checked > 30, "the line was shown over the bars: {checked}");
}

/// The `climb` HUD's action button face, #E0650F.
fn action_face(boxes: &[Drawn]) -> bool {
    let face = [224.0 / 255.0, 101.0 / 255.0, 15.0 / 255.0, 1.0];
    boxes.iter().any(|b| near(b.fill, face))
}

#[test]
fn the_games_hud_steps_aside_while_a_cutscene_holds_control() {
    let ended = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = ended.clone();
    let logic = native(move |n, events, out| {
        if events
            .iter()
            .any(|e| matches!(e, Event::CutsceneEnded { .. }))
        {
            seen.set(Some(n));
        }
        if n == 10 {
            play_talk(out);
        }
    });
    let mut engine = talk_engine(logic, assets(), EngineSettings::default());
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut held = 0;
    for _ in 0..140 {
        engine.step(&actions);
        let by = engine.state().control.by;
        let shown = action_face(&drawn(&mut engine, 0.5));
        match by {
            Controller::Cutscene => {
                assert!(!shown, "the HUD's button drawn during the cutscene");
                held += 1;
            }
            _ => assert!(shown, "the HUD's button missing at step {}", engine.steps()),
        }
    }
    assert!(held >= 60, "the cutscene held control: {held}");
    assert!(ended.get().is_some(), "the cutscene ended");
}

fn three_panels() -> Assets {
    let mut assets = assets();
    let panel = |name: &str, left: u32, fill: &str, groups: &str| {
        format!(
            r##"{{ "name": "{name}", "absolute": true, "left": {left}, "top": 200, "width": 50, "height": 50, "fill": "{fill}"{groups} }}"##
        )
    };
    let description = format!(
        r#"{{ "nodes": [ {}, {}, {} ] }}"#,
        panel("a", 100, "#FF0000", ""),
        panel("b", 200, "#00FF00", r#", "groups": ["cutscene"]"#),
        panel("c", 300, "#0000FF", ""),
    );
    assets.hud = Some(HudAsset {
        description,
        files: Vec::new(),
    });
    assets
}

fn panels(boxes: &[Drawn]) -> [bool; 3] {
    let has = |fill: [f32; 4]| boxes.iter().any(|b| near(b.fill, fill));
    [
        has([1.0, 0.0, 0.0, 1.0]),
        has([0.0, 1.0, 0.0, 1.0]),
        has([0.0, 0.0, 1.0, 1.0]),
    ]
}

#[test]
fn a_node_in_the_cutscene_group_stays_and_the_games_own_visibility_holds() {
    let logic = native(|n, _, out| match n {
        5 => play_talk(out),
        30 => send(
            out,
            Command::UiVisible {
                node: name_id("c"),
                visible: false,
            },
        ),
        _ => {}
    });
    let mut engine = talk_engine(logic, three_panels(), EngineSettings::default());
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut seen = Vec::new();
    for _ in 0..140 {
        engine.step(&actions);
        let cutscene = engine.state().control.by == Controller::Cutscene;
        seen.push((cutscene, panels(&drawn(&mut engine, 0.5))));
    }
    let before = seen[2];
    let during = seen.iter().find(|(c, _)| *c).expect("a cutscene played");
    let after = *seen.last().unwrap();
    assert_eq!(before, (false, [true, true, true]));
    assert_eq!(*during, (true, [false, true, false]));
    assert_eq!(
        after,
        (false, [true, true, false]),
        "c stays hidden by the game"
    );
}

#[test]
fn with_the_setting_off_the_games_hud_stays_through_a_cutscene() {
    let mut settings = EngineSettings::default();
    settings.hud.hide_in_cutscenes = false;
    let mut engine = talk_engine(play_talk_at(0), three_panels(), settings);
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..30 {
        engine.step(&actions);
        assert_eq!(engine.state().control.by, Controller::Cutscene);
        assert_eq!(panels(&drawn(&mut engine, 0.5)), [true; 3]);
    }
}

#[test]
fn the_cinema_bars_are_drawn_at_the_steps_height_between_steps() {
    let logic = native(|n, _, out| {
        if n == 0 {
            send(
                out,
                Command::Letterbox {
                    to: 1.0,
                    duration: 0.5,
                },
            );
        }
    });
    let mut engine = talk_engine(logic, assets(), EngineSettings::default());
    let actions = Actions::for_settings(&engine.state().settings.input);
    let screen = engine.hud().ui.screen();
    let fraction = EngineSettings::default().hud.bar_fraction;
    let mut checked = 0;
    for _ in 0..40 {
        engine.step(&actions);
        let want = engine.state().screen.letterbox.value() * fraction * screen.height;
        if want < 1.0 {
            continue;
        }
        for alpha in [0.0, 0.5, 1.0] {
            let boxes = drawn(&mut engine, alpha);
            let bar = bottom_bar(&boxes, screen.width, screen.height).expect("the bar is drawn");
            let height = bar.bounds[3] - bar.bounds[1];
            assert!(
                (height - want).abs() < 0.01,
                "step {} alpha {alpha}: the bar is {height} px, the step's is {want}",
                engine.steps()
            );
        }
        checked += 1;
    }
    assert!(checked > 20, "{checked}");
}
