// SPDX-License-Identifier: Apache-2.0

mod common;

use common::{id, near, screen, ui};
use sariema_ui::{Event, Prop, Ui, stick_curve};
use serde_json::json;

fn targets() -> Ui {
    let mut ui = ui(json!({ "nodes": [
        { "name": "a", "absolute": true, "left": 10, "top": 10, "width": 100, "height": 100, "interactive": true },
        { "name": "b", "absolute": true, "left": 60, "top": 60, "width": 100, "height": 100, "interactive": true },
        { "name": "clip", "absolute": true, "left": 300, "top": 10, "width": 50, "height": 50, "clip": true,
          "children": [ { "name": "inner", "absolute": true, "left": 0, "top": 0, "width": 200, "height": 200,
                          "interactive": true } ] },
        { "name": "turned", "absolute": true, "left": 500, "top": 100, "width": 100, "height": 20,
          "rotation": 90, "interactive": true },
        { "name": "grown", "absolute": true, "left": 700, "top": 300, "width": 20, "height": 20,
          "scale": 2, "interactive": true },
        { "name": "ghost", "absolute": true, "left": 600, "top": 300, "width": 50, "height": 50,
          "interactive": true, "opacity": 0 }
    ]}));
    screen(&mut ui, 844.0, 390.0);
    ui
}

fn hit(ui: &mut Ui, x: f32, y: f32) -> Option<(String, [f32; 2])> {
    let (node, at) = ui.hit_test(x, y)?;
    Some((ui.name(node).unwrap().to_string(), at))
}

#[test]
fn topmost_wins_where_nodes_overlap() {
    let mut ui = targets();
    assert_eq!(hit(&mut ui, 20.0, 20.0).unwrap().0, "a");
    assert_eq!(hit(&mut ui, 80.0, 80.0).unwrap().0, "b");
    assert_eq!(hit(&mut ui, 150.0, 150.0).unwrap().0, "b");
    assert!(hit(&mut ui, 5.0, 5.0).is_none());
}

#[test]
fn clipping_hides_what_it_cuts() {
    let mut ui = targets();
    assert_eq!(hit(&mut ui, 305.0, 15.0).unwrap().0, "inner");
    assert!(
        hit(&mut ui, 320.0, 100.0).is_none(),
        "inner is there, but clipped"
    );
}

#[test]
fn transforms_move_what_is_hit_and_local_points() {
    let mut ui = targets();
    let (name, at) = hit(&mut ui, 550.0, 70.0).unwrap();
    assert_eq!(name, "turned");
    assert!(near(at[0], 10.0, 1e-3) && near(at[1], 10.0, 1e-3), "{at:?}");
    assert!(
        hit(&mut ui, 520.0, 110.0).is_none(),
        "where it was before turning"
    );
    let (name, at) = hit(&mut ui, 695.0, 295.0).unwrap();
    assert_eq!(name, "grown");
    assert!(near(at[0], 2.5, 1e-3) && near(at[1], 2.5, 1e-3), "{at:?}");
    assert!(
        hit(&mut ui, 625.0, 325.0).is_none(),
        "transparent nodes are not hit"
    );
}

#[test]
fn click_needs_the_release_over_the_pressed_node() {
    let mut ui = targets();
    let a = id(&ui, "a");
    ui.pointer_down(1, 20.0, 20.0);
    ui.pointer_up(1, 30.0, 30.0);
    ui.pointer_down(2, 20.0, 20.0);
    ui.pointer_move(2, 400.0, 300.0);
    ui.pointer_up(2, 400.0, 300.0);
    let events = ui.step().to_vec();
    let clicks = events
        .iter()
        .filter(|e| matches!(e, Event::Click { .. }))
        .count();
    assert_eq!(clicks, 1);
    assert!(events.contains(&Event::Click { node: a }));
    assert!(
        matches!(events[4], Event::Drag { node, pointer: 2, x, .. } if node == a && near(x, 390.0, 1e-3))
    );
}

fn pad() -> Ui {
    let mut ui = ui(json!({ "nodes": [
        { "name": "zone", "absolute": true, "left": 0, "top": 0, "width": 422, "height": 390,
          "widget": { "joystick": { "base": "base", "pin": "pin" } },
          "children": [
            { "name": "base", "absolute": true, "left": 32, "bottom": 32, "width": 128, "height": 128,
              "opacity": 0.45, "radius": 64, "fill": "#ffffff33",
              "children": [ { "name": "pin", "absolute": true, "left": 36, "top": 36, "width": 56,
                              "height": 56, "radius": 28, "fill": "#fff" } ] }
          ] },
        { "name": "button", "absolute": true, "right": 40, "bottom": 40, "width": 80, "height": 80,
          "widget": { "button": { "face": "face", "ring": "ring" } },
          "children": [
            { "name": "face", "absolute": true, "left": 4, "top": 4, "width": 72, "height": 72,
              "radius": 36, "fill": "#E0650F" },
            { "name": "ring", "absolute": true, "left": 0, "top": 0, "width": 80, "height": 80,
              "ring": { "width": 5, "track": "#ffffff47" }, "progress": 0 }
          ] }
    ]}));
    screen(&mut ui, 844.0, 390.0);
    ui
}

#[test]
fn joystick_and_button_follow_their_own_fingers() {
    let mut ui = pad();
    let (zone, button) = (id(&ui, "zone"), id(&ui, "button"));
    let (base, pin, ring) = (id(&ui, "base"), id(&ui, "pin"), id(&ui, "ring"));
    ui.pointer_down(1, 100.0, 300.0);
    ui.pointer_down(2, 764.0, 310.0);
    ui.pointer_move(1, 128.0, 300.0);
    let events = ui.step().to_vec();
    assert!(matches!(events[0], Event::Press { node, pointer: 1, .. } if node == zone));
    assert!(matches!(events[1], Event::Press { node, pointer: 2, .. } if node == button));
    let Some(Event::Stick { x, y, .. }) = events.iter().find(|e| matches!(e, Event::Stick { .. }))
    else {
        panic!("no stick event in {events:?}");
    };
    assert!(
        near(*x, 1.0, 1e-6) && *y == 0.0,
        "0.7 of the radius is full speed"
    );
    // The base jumped under the thumb: its rest centre is (96, 294).
    assert_eq!(ui.value(base, Prop::X), Some(4.0));
    assert_eq!(ui.value(base, Prop::Y), Some(6.0));
    assert_eq!(ui.value(pin, Prop::X), Some(28.0));
    ui.pointer_move(1, 300.0, 300.0);
    assert_eq!(
        ui.value(pin, Prop::X),
        Some(40.0),
        "the pin stops at the radius"
    );

    let mut held_at = None;
    for step in 2..=90 {
        if ui.step().contains(&Event::Held { node: button }) {
            held_at = Some(step);
        }
        assert_eq!(
            ui.stick(zone),
            Some([1.0, 0.0]),
            "holding the button leaves the stick alone"
        );
    }
    // 1.2 s at 60 steps per second, counting the step of the press.
    assert_eq!(held_at, Some(72));
    assert_eq!(ui.value(ring, Prop::Progress), Some(1.0));

    ui.pointer_up(1, 300.0, 300.0);
    ui.pointer_up(2, 764.0, 310.0);
    let events = ui.step().to_vec();
    assert!(events.contains(&Event::Stick {
        node: zone,
        x: 0.0,
        y: 0.0
    }));
    assert!(
        !events.iter().any(|e| matches!(e, Event::Tap { .. })),
        "a hold is not a tap"
    );
    for _ in 0..60 {
        ui.step();
    }
    for (node, prop) in [(pin, Prop::X), (base, Prop::X), (base, Prop::Y)] {
        assert_eq!(
            ui.value(node, prop),
            Some(0.0),
            "springs settle within a second"
        );
    }
    assert!(near(ui.value(base, Prop::Opacity).unwrap(), 0.45, 1e-6));
}

#[test]
fn a_short_press_taps_and_the_ring_empties() {
    let mut ui = pad();
    let (button, ring) = (id(&ui, "button"), id(&ui, "ring"));
    ui.pointer_down(7, 764.0, 310.0);
    for _ in 0..18 {
        ui.step();
    }
    let filled = ui.value(ring, Prop::Progress).unwrap();
    assert!(near(filled, 0.25, 1e-3), "{filled}");
    ui.pointer_up(7, 764.0, 310.0);
    assert!(ui.step().contains(&Event::Tap { node: button }));
    for _ in 0..3 {
        ui.step();
    }
    assert_eq!(
        ui.value(ring, Prop::Progress),
        Some(0.0),
        "empties in 0.2 s × 0.25"
    );
}

#[test]
fn stick_curve_at_the_dead_zone_and_saturation() {
    let r = 40.0;
    let at = |m: f32| stick_curve([m * r, 0.0], r, 0.15, 0.7)[0];
    assert_eq!(at(0.0), 0.0);
    assert_eq!(at(0.15), 0.0, "the dead zone's edge is still dead");
    assert!(near(at(0.16), 0.01 / 0.55, 1e-5));
    assert!(near(at(0.425), 0.5, 1e-5));
    assert!(near(at(0.7), 1.0, 1e-6));
    assert_eq!(at(1.0), 1.0);
    let d = stick_curve([0.7 * r * 0.6, 0.7 * r * 0.8], r, 0.15, 0.7);
    assert!(
        near(d[0], 0.6, 1e-5) && near(d[1], 0.8, 1e-5),
        "keeps the direction"
    );
}

#[test]
fn focus_follows_the_order_and_wraps() {
    let mut ui = ui(json!({ "nodes": [
        { "name": "two", "focus": 2 }, { "name": "one", "focus": 1 }, { "name": "three", "focus": 3 },
        { "name": "none" }
    ]}));
    let order: Vec<String> = (0..4)
        .map(|_| {
            ui.focus_next();
            ui.name(ui.focused().unwrap()).unwrap().to_string()
        })
        .collect();
    assert_eq!(order, ["one", "two", "three", "one"]);
    ui.focus_prev();
    assert_eq!(ui.name(ui.focused().unwrap()), Some("three"));
    ui.activate();
    let three = id(&ui, "three");
    assert!(ui.step().contains(&Event::Click { node: three }));
}
