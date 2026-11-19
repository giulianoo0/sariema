// SPDX-License-Identifier: Apache-2.0

//! A traversal or a clip path that holds its end leaves the actor exactly
//! there, collider off, with no gravity and no push-out, until its next
//! movement command; one that does not lands it as before.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glam::Vec3;
use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::collision::{Spot, Traversal};
use sariema_format::script::{ClipMode, ClipSpan, Command, Ease, Event, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

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

/// The top of the rope, 1.2 m over the floor.
const TOP: [f32; 3] = [0.0, 1.2, 0.0];
/// Inside the window leaf, where a reach ends with the hands on it.
const ON_LEAF: [f32; 3] = [2.15, 0.0, 0.0];

/// A floor, a window leaf across x = 2.0 to 2.3, and a rope climbed from
/// the origin to `TOP`: `rope` holds its end, `rope_down` lands it.
fn room() -> RoomAssets {
    let colliders = vec![
        block("floor", [-6.0, -0.1, -4.0], [8.0, 0.0, 4.0]),
        block("leaf", [2.0, 0.0, -0.5], [2.3, 2.0, 0.5]),
    ];
    let mut pack = collision("rig", colliders);
    pack.spots = vec![Spot {
        name: "rope_foot".into(),
        actor: 0,
        position: [0.0; 3],
        yaw: 0.0,
    }];
    let rope = |name: &str, hold_end: bool| Traversal {
        name: name.into(),
        from: 0,
        points: vec![[0.0, 0.6, 0.0], TOP],
        yaw: 0.0,
        contacts: Vec::new(),
        hold_end,
        ease: 0,
    };
    pack.traversals = vec![rope("rope", true), rope("rope_down", false)];
    RoomAssets::collision("rig", pack)
}

/// What one step left: the hero's root, and the `located` its tick heard,
/// which answers a `locate` sent the step before.
#[derive(Clone, Copy, Debug, Default)]
struct Step {
    root: Vec3,
    located: Option<([f32; 3], f32, bool)>,
}

type Script = Box<dyn FnMut(u64, &mut Vec<u8>, f32)>;

/// Runs `steps` steps of a game whose hero spawns at `at`, `script`
/// writing each tick's further commands, given the length of the hero's
/// `wave` clip.
fn run(
    at: [f32; 3],
    steps: usize,
    mut script: Script,
) -> (Vec<Step>, Engine<impl sariema_runtime::sim::Logic>) {
    let length = Rc::new(Cell::new(0.0f32));
    let heard = Rc::new(RefCell::new(None));
    let (len, got) = (length.clone(), heard.clone());
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::Located {
                position,
                yaw,
                held,
                ..
            } = *e
            {
                *got.borrow_mut() = Some((position, yaw, held));
            }
        }
        if n == 0 {
            spawn_hero(out, at, 0.0);
        }
        script(n, out, len.get());
    });
    let mut assets = assets();
    assets.rooms = vec![room()];
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let model = &engine.animation().models[0];
    let clip = model.rig.clip("wave").expect("the hero has a wave");
    length.set(model.rig.clip_info(clip).duration);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut trace = Vec::with_capacity(steps);
    for _ in 0..steps {
        engine.step(&actions);
        trace.push(Step {
            root: engine.state().world.node(entity(0)).unwrap().local.position,
            located: heard.borrow_mut().take(),
        });
    }
    (trace, engine)
}

fn climb(out: &mut Vec<u8>, traversal: &str) {
    send(
        out,
        Command::Traverse {
            entity: entity(0),
            traversal: name_id(traversal),
            duration: 1.5,
        },
    );
}

/// A `wave` that carries the root to `to` over its middle third.
fn path_to(out: &mut Vec<u8>, length: f32, to: [f32; 3], hold_end: bool) {
    let mut spans = Vec::new();
    ClipSpan {
        from: length / 3.0,
        until: 2.0 * length / 3.0,
        rate: 1.0,
        to: Some(to),
        yaw: None,
        ease: Ease::Linear,
        arc: 0.0,
    }
    .put(&mut spans);
    send(
        out,
        Command::ClipPath {
            entity: entity(0),
            clip: name_id("wave"),
            mode: ClipMode::Hold,
            blend: 0.1,
            from: 0.0,
            hold: 0.0,
            hold_end,
            spans: &spans,
        },
    );
}

/// Writes a command that should release a held actor, given the length of
/// the `wave` clip.
type Release = fn(&mut Vec<u8>, f32);

fn locate(out: &mut Vec<u8>) {
    send(out, Command::Locate { entity: entity(0) });
}

#[test]
fn a_held_rope_end_stays_up_and_the_next_path_starts_from_it() {
    let (trace, _) = run(
        [0.0; 3],
        400,
        Box::new(|n, out, length| match n {
            2 => climb(out, "rope"),
            150 => path_to(out, length, [0.6, 1.2, 0.0], true),
            _ => {}
        }),
    );
    for (k, s) in trace[100..150].iter().enumerate() {
        assert_eq!(s.root, Vec3::from(TOP), "held at the top, step {}", 100 + k);
    }
    for (k, s) in trace[150..].iter().enumerate() {
        assert!(
            (s.root.y - 1.2).abs() < 1e-6,
            "the path goes on from the held top, step {}: {}",
            150 + k,
            s.root
        );
    }
    assert_eq!(
        trace[399].root,
        Vec3::new(0.6, 1.2, 0.0),
        "held where the path ended"
    );
}

#[test]
fn a_held_end_inside_a_collider_is_not_pushed_out_and_an_unheld_one_is() {
    let reach = |hold_end: bool| {
        run(
            [1.0, 0.0, 0.0],
            300,
            Box::new(move |n, out, length| {
                if n == 2 {
                    path_to(out, length, ON_LEAF, hold_end);
                }
            }),
        )
        .0
    };
    let held = reach(true);
    for (k, s) in held[200..].iter().enumerate() {
        assert_eq!(
            s.root,
            Vec3::from(ON_LEAF),
            "held on the leaf, step {}",
            200 + k
        );
    }
    let landed = reach(false);
    assert!(
        landed[299].root.distance(Vec3::from(ON_LEAF)) > 0.05,
        "an unheld end is pushed out of the leaf as before: {}",
        landed[299].root
    );
}

#[test]
fn every_movement_command_releases_the_hold() {
    let releases: [(&str, Release); 7] = [
        ("move_to", |out, _| {
            send(
                out,
                Command::MoveTo {
                    entity: entity(0),
                    target: [0.0, 0.0, 2.0],
                    speed: 1.0,
                },
            )
        }),
        ("move_intent", |out, _| {
            send(
                out,
                Command::MoveIntent {
                    entity: entity(0),
                    velocity: [0.0, 0.0, 1.0],
                },
            )
        }),
        ("stop", |out, _| {
            send(out, Command::Stop { entity: entity(0) })
        }),
        ("knockback", |out, _| {
            send(
                out,
                Command::Knockback {
                    entity: entity(0),
                    impulse: [0.0, 0.0, 1.0],
                },
            )
        }),
        ("teleport", |out, _| {
            send(
                out,
                Command::Teleport {
                    entity: entity(0),
                    position: [0.0, 1.0, 0.0],
                    yaw: 0.0,
                    force: true,
                    hold: false,
                },
            )
        }),
        ("clip_path", |out, length| {
            path_to(out, length, [0.3, 1.2, 0.0], false)
        }),
        ("traverse", |out, _| climb(out, "rope_down")),
    ];
    for (name, release) in releases {
        let (trace, _) = run(
            [0.0; 3],
            500,
            Box::new(move |n, out, length| match n {
                2 => climb(out, "rope"),
                100 => locate(out),
                101 => release(out, length),
                102 => locate(out),
                _ => {}
            }),
        );
        let before = trace[101].located.expect("located before");
        assert!(before.2, "{name}: held before the command");
        let after = trace[103].located.expect("located after");
        assert!(!after.2, "{name}: released by the command");
        assert!(
            trace[499].root.y < 0.05,
            "{name}: down on the floor once released: {}",
            trace[499].root
        );
    }
}

#[test]
fn a_held_actor_saved_and_put_back_with_teleport_hold_stays_held() {
    let (saved, _) = run(
        [0.0; 3],
        110,
        Box::new(|n, out, _| match n {
            2 => climb(out, "rope"),
            100 => locate(out),
            _ => {}
        }),
    );
    let (position, yaw, held) = saved[101].located.expect("located");
    assert!(held, "a held end says so");
    let (resumed, _) = run(
        [0.0; 3],
        200,
        Box::new(move |n, out, _| match n {
            2 => send(
                out,
                Command::Teleport {
                    entity: entity(0),
                    position,
                    yaw,
                    force: true,
                    hold: held,
                },
            ),
            150 => locate(out),
            _ => {}
        }),
    );
    for s in &resumed[10..] {
        assert_eq!(s.root, Vec3::from(position), "put back held");
    }
    assert!(resumed[151].located.expect("located").2, "held again");
}

#[test]
fn a_held_actor_steps_without_allocating() {
    let (_, mut engine) = run(
        [0.0; 3],
        120,
        Box::new(|n, out, _| {
            if n == 2 {
                climb(out, "rope");
            }
        }),
    );
    let actions = Actions::for_settings(&engine.state().settings.input);
    ARMED.with(|a| a.set(true));
    for _ in 0..120 {
        engine.step(&actions);
    }
    ARMED.with(|a| a.set(false));
    assert_eq!(
        engine.state().world.node(entity(0)).unwrap().local.position,
        Vec3::from(TOP)
    );
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
