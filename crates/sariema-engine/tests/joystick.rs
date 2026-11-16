// SPDX-License-Identifier: Apache-2.0

//! The HUD's joystick is the player's move axis, and its button an action.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_format::script::{Command, Event};
use sariema_runtime::input::Actions;

use common::{engine, entity, native, send, spawn_hero, wired};

#[test]
fn the_hud_joystick_moves_the_hero_and_its_button_presses_an_action() {
    let pressed = Rc::new(RefCell::new(Vec::new()));
    let log = pressed.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::Button { action, down } = e {
                log.borrow_mut().push((*action, *down));
            }
        }
        if n == 0 {
            spawn_hero(out, [0.75, 0.0, -0.4], 0.0);
            send(
                out,
                Command::SetHero {
                    entity: entity(0),
                    speed: 1.5,
                },
            );
        }
    });
    let mut settings = wired();
    settings
        .input
        .buttons
        .push(sariema_runtime::input::ButtonBinding {
            name: "action".into(),
            keys: Vec::new(),
            pad: Vec::new(),
        });
    let action = settings.input.button("action").unwrap();
    let mut engine = engine(logic, settings, &[]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..5 {
        engine.step(&actions);
    }
    engine.draw_hud(1.0);
    let hud = &engine.hud().ui;
    let scale = hud.screen().scale;
    let centre = |name: &str| {
        let [x, y, w, h] = hud.rect(hud.node(name).unwrap()).unwrap();
        ((x + w / 2.0) * scale, (y + h / 2.0) * scale)
    };
    let (bx, by) = centre("base");
    let (ax, ay) = centre("action");
    let start = engine.state().world.node(entity(0)).unwrap().local.position;

    engine.pointer_down(1, bx, by);
    engine.pointer_move(1, bx, by - 60.0 * scale);
    engine.pointer_down(2, ax, ay);
    for _ in 0..60 {
        engine.step(&actions);
    }
    engine.pointer_up(1, bx, by - 60.0 * scale);
    engine.pointer_up(2, ax, ay);
    for _ in 0..10 {
        engine.step(&actions);
    }
    let end = engine.state().world.node(entity(0)).unwrap().local.position;
    assert!(
        start.z - end.z > 0.8 && (end.x - start.x).abs() < 0.1,
        "pushed up, the hero walks north: from {start} to {end}"
    );
    assert_eq!(*pressed.borrow(), vec![(action, true), (action, false)]);
}

const TWO_STICKS: &str = r#"{ "nodes": [
  { "name": "walk_zone", "absolute": true, "left": 0, "top": 0, "bottom": 0, "width": "50%",
    "widget": { "joystick": { "base": "walk_base", "pin": "walk_pin" } },
    "children": [
      { "name": "walk_base", "absolute": true, "left": 16, "bottom": 16, "width": 128, "height": 128,
        "children": [ { "name": "walk_pin", "absolute": true, "left": 36, "top": 36, "width": 56, "height": 56 } ] }
    ] },
  { "name": "look_zone", "absolute": true, "right": 0, "top": 0, "bottom": 0, "width": "50%",
    "widget": { "joystick": { "base": "look_base", "pin": "look_pin" } },
    "children": [
      { "name": "look_base", "absolute": true, "right": 16, "bottom": 16, "width": 128, "height": 128,
        "children": [ { "name": "look_pin", "absolute": true, "left": 36, "top": 36, "width": 56, "height": 56 } ] }
    ] }
] }"#;

/// How far the hero walks while the joystick whose base is `base` is held up.
fn walked_with(base: &str) -> f32 {
    let logic = native(|n, _, out| {
        if n == 0 {
            spawn_hero(out, [0.75, 0.0, -0.4], 0.0);
            send(
                out,
                Command::SetHero {
                    entity: entity(0),
                    speed: 1.5,
                },
            );
        }
    });
    let mut assets = common::assets();
    assets.hud = Some(sariema_format::script::Hud {
        description: TWO_STICKS.into(),
        files: Vec::new(),
    });
    let sim = sariema_runtime::sim::Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine =
        sariema_engine::Engine::new(sim, assets, sariema_engine::EngineSettings::default())
            .unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..5 {
        engine.step(&actions);
    }
    engine.draw_hud(1.0);
    let hud = &engine.hud().ui;
    let scale = hud.screen().scale;
    let [x, y, w, h] = hud.rect(hud.node(base).unwrap()).unwrap();
    let (bx, by) = ((x + w / 2.0) * scale, (y + h / 2.0) * scale);
    let start = engine.state().world.node(entity(0)).unwrap().local.position;
    engine.pointer_down(1, bx, by);
    engine.pointer_move(1, bx, by - 60.0 * scale);
    for _ in 0..30 {
        engine.step(&actions);
    }
    let end = engine.state().world.node(entity(0)).unwrap().local.position;
    start.distance(end)
}

#[test]
fn of_two_joysticks_the_first_described_moves_the_hero_every_run() {
    for run in 0..16 {
        let walked = walked_with("walk_base");
        assert!(
            walked > 0.4,
            "run {run}: the first joystick walked {walked}"
        );
        let looked = walked_with("look_base");
        assert!(
            looked < 1e-4,
            "run {run}: the second joystick walked {looked}"
        );
    }
}
