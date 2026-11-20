// SPDX-License-Identifier: Apache-2.0

//! `face` turns a still actor in place at its turn rate even when position
//! tweens left it posed: step for step as a free actor turns, its position
//! untouched. A cutscene role posed on its start mark turns to its start yaw
//! the same way.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glam::Vec3;
use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::script::{Command, Ease, Event, Property, name_id};
use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::input::Actions;
use sariema_runtime::sim::{Logic, Simulation};

use common::{assets, block, collision, entity, native, send, spawn_hero, wired};

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn count() {
    if ARMED.with(Cell::get) {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
    }
}

// SAFETY: every call is passed on to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Where the actor ends: tweened there, or spawned there.
const MARK: [f32; 3] = [1.0, 0.0, 0.0];
/// The step the game turns it, or plays the cutscene.
const TURN_AT: u64 = 60;
const STEPS: usize = 240;

const SEAT: &str = r#"{ "name": "seat", "roles": { "hero": { "start": { "position": [1, 0, 0], "yaw": 90 } } },
    "steps": [ { "call": "go" } ] }"#;

fn room() -> RoomAssets {
    let floor = block("floor", [-6.0, -0.1, -4.0], [8.0, 0.0, 4.0]);
    RoomAssets::collision("flat", collision("flat", vec![floor]))
}

/// The hero spawned at `MARK`, or at the origin and tweened to `MARK` in
/// x when `posed`.
fn place(n: u64, out: &mut Vec<u8>, posed: bool) {
    match n {
        0 => spawn_hero(out, if posed { [0.0; 3] } else { MARK }, 0.0),
        2 if posed => send(
            out,
            Command::Tween {
                tween: 1,
                entity: entity(0),
                property: Property::PositionX,
                var: 0,
                to: MARK[0],
                duration: 0.4,
                ease: Ease::InOutSine,
            },
        ),
        _ => {}
    }
}

fn face(out: &mut Vec<u8>) {
    send(
        out,
        Command::Face {
            entity: entity(0),
            yaw: 90.0,
        },
    );
}

fn play_seat(out: &mut Vec<u8>) {
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
            cutscene: name_id("seat"),
        },
    );
}

fn engine<L: Logic>(logic: L, cutscenes: &[&str]) -> Engine<L> {
    let cutscenes = (cutscenes.iter())
        .map(|c| compile_json(c).unwrap().0)
        .collect();
    let mut assets = assets();
    assets.rooms = vec![room()];
    let sim = Simulation::new(wired(), cutscenes, logic, 1);
    Engine::new(sim, assets, EngineSettings::default()).unwrap()
}

#[derive(Debug, Default)]
struct Trace {
    yaw: Vec<f32>,
    root: Vec<Vec3>,
    /// The step the game heard the cutscene's `go` call.
    go: Option<u64>,
}

/// Places the hero, posed or free, then at `TURN_AT` sends `face`, or
/// plays `seat` when `cutscene`.
fn run(posed: bool, cutscene: bool) -> Trace {
    let go = Rc::new(RefCell::new(None));
    let heard = go.clone();
    let logic = native(move |n, events, out| {
        if events
            .iter()
            .any(|e| matches!(e, Event::CutsceneCall { .. }))
        {
            heard.borrow_mut().get_or_insert(n);
        }
        place(n, out, posed);
        if n == TURN_AT {
            if cutscene { play_seat(out) } else { face(out) }
        }
    });
    let mut engine = engine(logic, &[SEAT]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut trace = Trace::default();
    for _ in 0..STEPS {
        engine.step(&actions);
        let node = engine.state().world.node(entity(0)).unwrap();
        trace.yaw.push(node.local.yaw);
        trace.root.push(node.local.position);
    }
    trace.go = *go.borrow();
    trace
}

fn faces(yaw: f32, to: f32) -> bool {
    ((yaw - to + 180.0).rem_euclid(360.0) - 180.0).abs() < 1e-3
}

#[test]
fn a_tween_posed_actor_turns_on_face_as_a_free_one_does() {
    let free = run(false, false);
    let posed = run(true, false);
    let at = TURN_AT as usize;
    let landed = posed.root[at - 1];
    assert_eq!(landed.x, MARK[0], "the tween has landed");
    assert_eq!(posed.yaw[at - 1], 0.0, "still facing the start");
    assert!(
        faces(posed.yaw[STEPS - 1], 90.0),
        "{}",
        posed.yaw[STEPS - 1]
    );
    assert_eq!(
        posed.yaw[at..],
        free.yaw[at..],
        "the same turn, step for step"
    );
    assert!(
        posed.yaw[at + 1] != posed.yaw[STEPS - 1],
        "turned over steps"
    );
    for (k, root) in posed.root[at..].iter().enumerate() {
        assert_eq!(*root, landed, "unmoved, step {}", at + k);
    }
}

#[test]
fn a_role_posed_on_its_start_mark_turns_to_its_start_yaw_as_a_free_one_does() {
    let free = run(false, true);
    let posed = run(true, true);
    assert!(
        faces(posed.yaw[STEPS - 1], 90.0),
        "{}",
        posed.yaw[STEPS - 1]
    );
    let at = TURN_AT as usize;
    assert_eq!(
        posed.yaw[at..],
        free.yaw[at..],
        "the same turn, step for step"
    );
    assert_eq!(
        posed.go, free.go,
        "the steps begin as they do for a free role"
    );
    assert!(posed.go.is_some_and(|go| go > TURN_AT));
}

#[test]
fn a_posed_actor_turning_allocates_nothing() {
    let logic = native(move |n, _, out| {
        place(n, out, true);
        if n == TURN_AT {
            face(out);
        }
    });
    let mut engine = engine(logic, &[]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..TURN_AT {
        engine.step(&actions);
    }
    ARMED.with(|a| a.set(true));
    for _ in 0..120 {
        engine.step(&actions);
    }
    ARMED.with(|a| a.set(false));
    let yaw = engine.state().world.node(entity(0)).unwrap().local.yaw;
    assert!(faces(yaw, 90.0), "{yaw}");
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
