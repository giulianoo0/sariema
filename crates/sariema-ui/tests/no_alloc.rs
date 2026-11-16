// SPDX-License-Identifier: Apache-2.0

//! A frame of a screen that does not lay out again must not allocate: an
//! allocator that counts calls made on this thread while armed.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::{id, screen, ui};
use sariema_ui::{Animation, Ease, Prop};
use serde_json::{Value, json};

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<u64> = const { Cell::new(0) };
}

fn note() {
    if ARMED.with(Cell::get) {
        CALLS.with(|c| c.set(c.get() + 1));
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn armed<R>(f: impl FnOnce() -> R) -> (R, u64) {
    CALLS.with(|c| c.set(0));
    ARMED.with(|a| a.set(true));
    let r = f();
    ARMED.with(|a| a.set(false));
    (r, CALLS.with(Cell::get))
}

fn screen_of(n: usize) -> Value {
    let items: Vec<Value> = (0..n)
        .map(|i| {
            json!({ "name": format!("n{i}"), "fill": "#FFF8EC", "radius": 12,
                         "border": { "width": 2, "color": "#33261C" },
                         "children": [ { "text": format!("Item {i}"), "size": 13 } ] })
        })
        .collect();
    json!({ "fonts": common::font(), "nodes": [
        { "wrap": true, "children": items },
        { "name": "line", "absolute": true, "bottom": 10, "left": 10, "text": "Hello? Anyone home? Olá!",
          "reveal": 0 }
    ]})
}

#[test]
fn a_static_screen_allocates_nothing() {
    let mut ui = ui(screen_of(60));
    screen(&mut ui, 844.0, 390.0);
    ui.step();
    ui.draw(1.0);
    let (quads, calls) = armed(|| {
        let mut quads = 0;
        for _ in 0..120 {
            ui.step();
            quads += ui.draw(0.5).quads.len();
        }
        quads
    });
    assert!(quads > 0);
    assert_eq!(calls, 0);
}

#[test]
fn animating_transforms_and_text_allocates_nothing() {
    let mut ui = ui(screen_of(60));
    screen(&mut ui, 844.0, 390.0);
    ui.step();
    ui.draw(1.0);
    for i in 0..50 {
        let n = id(&ui, &format!("n{i}"));
        ui.animate(
            n,
            Prop::Y,
            Animation::tween(0.0, 0.6, Ease::OUT)
                .from(8.0)
                .delay(i as f32 * 0.01),
        )
        .unwrap();
        ui.animate(n, Prop::Opacity, Animation::spring(1.0, 0.3, 0.8).from(0.0))
            .unwrap();
    }
    let line = id(&ui, "line");
    ui.animate(line, Prop::Reveal, Animation::typewriter(30.0, 0.07))
        .unwrap();
    let (_, calls) = armed(|| {
        for _ in 0..90 {
            ui.step();
            ui.draw(0.5);
        }
    });
    assert_eq!(calls, 0);
    assert_eq!(ui.animating(), 0);
}

#[test]
fn playing_a_named_animation_allocates_nothing() {
    let mut desc = screen_of(4);
    desc["animations"] = json!({ "pop": [
        { "nodes": ["n0", "n1", "n2"], "stagger": 0.05, "prop": "scale", "from": 0.8, "to": 1, "duration": 0.2 },
        { "node": "line", "prop": "opacity", "from": 0, "to": 1, "duration": 0.2, "after": true }
    ]});
    let mut ui = ui(desc);
    screen(&mut ui, 844.0, 390.0);
    ui.step();
    let (played, calls) = armed(|| ui.play("pop"));
    played.unwrap();
    assert_eq!(calls, 0);
    assert_eq!(ui.animating(), 7);
}

#[test]
fn setting_a_prepared_text_shapes_and_grows_nothing() {
    let said = "Mind the sill, it is older than the house.";
    let mut desc = screen_of(4);
    desc["nodes"].as_array_mut().unwrap().push(
        json!({ "name": "say", "absolute": true, "top": 10, "left": 10, "width": 300, "text": "" }),
    );
    let mut ui = ui(desc.clone());
    screen(&mut ui, 844.0, 390.0);
    let line = id(&ui, "say");
    ui.prepare_text(line, said).unwrap();
    ui.step();
    let (set, calls) = armed(|| ui.set_text(line, said));
    set.unwrap();
    assert_eq!(calls, 0);
    ui.step();
    let prepared: Vec<_> = ui.draw(1.0).quads.clone();

    let mut fresh = self::ui(desc);
    screen(&mut fresh, 844.0, 390.0);
    fresh.step();
    fresh.set_text(id(&fresh, "say"), said).unwrap();
    fresh.step();
    let glyphs = said.chars().filter(|c| !c.is_whitespace()).count();
    assert!(prepared.len() > glyphs, "{} quads", prepared.len());
    assert_eq!(
        format!("{prepared:?}"),
        format!("{:?}", fresh.draw(1.0).quads)
    );
}
