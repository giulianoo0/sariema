// SPDX-License-Identifier: Apache-2.0

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use sariema_format::script::{Command, Ease, Event, Kind, Property};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation};

use common::{entity, native, send};

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

struct Counting;

// SAFETY: forwards every call to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        // SAFETY: the caller's layout contract, passed on.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Allocations in 100 steps of `logic` after 10 to settle.
fn steady_allocations<L: Logic>(logic: L) -> u64 {
    steady_allocations_with(Settings::default(), logic)
}

fn steady_allocations_with<L: Logic>(settings: Settings, logic: L) -> u64 {
    let actions = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, Vec::new(), logic, 1);
    for _ in 0..10 {
        sim.step(&actions);
    }
    let before = ALLOCATIONS.with(Cell::get);
    for _ in 0..100 {
        sim.step(&actions);
    }
    ALLOCATIONS.with(Cell::get) - before
}

#[test]
fn a_steady_step_with_two_thousand_tweens_allocates_nothing() {
    let logic = native(|_, events: &[Event<'_>], out| {
        if events.iter().any(|e| matches!(e, Event::Started { .. })) {
            for i in 0..2_000 {
                send(
                    out,
                    Command::Spawn {
                        entity: entity(i),
                        kind: Kind::Empty,
                        asset: 0,
                    },
                );
                send(
                    out,
                    Command::Tween {
                        tween: i,
                        entity: entity(i),
                        property: Property::PositionX,
                        var: 0,
                        to: 10.0,
                        duration: 100.0,
                        ease: Ease::InOutCubic,
                    },
                );
            }
        }
    });
    assert_eq!(
        steady_allocations(logic),
        0,
        "allocations in 100 steady steps"
    );
}

#[test]
fn a_steady_step_with_two_thousand_visible_actors_walking_allocates_nothing() {
    let logic = native(|_, events: &[Event<'_>], out| {
        if events.iter().any(|e| matches!(e, Event::Started { .. })) {
            for i in 0..2_000 {
                send(
                    out,
                    Command::Spawn {
                        entity: entity(i),
                        kind: Kind::Actor,
                        asset: 0,
                    },
                );
                send(
                    out,
                    Command::MoveIntent {
                        entity: entity(i),
                        velocity: [3.0, 0.0, 1.0],
                    },
                );
            }
        }
    });
    assert_eq!(
        steady_allocations(logic),
        0,
        "allocations in 100 steady steps"
    );
}

#[test]
fn steps_with_emitters_running_bursting_and_stopping_allocate_nothing() {
    use sariema_format::script::{Entity, PointList, name_id};
    use sariema_runtime::fx::Effect;
    let mut settings = Settings::default();
    settings.fx.effects = vec![Effect {
        name: "spark".into(),
        rate: 120.0,
        puff: 4,
        every: 0.1,
        life: [0.2, 0.6],
        speed: [0.5, 1.0],
        spread: 60.0,
        drift: 0.2,
        color: vec![[0.0, 1.0, 1.0, 1.0, 0.0], [1.0, 1.0, 0.5, 0.2, 0.0]],
        ..Effect::default()
    }];
    const PATH: [[f32; 3]; 3] = [[0.0; 3], [2.0, 0.0, 0.0], [2.0, 0.0, 2.0]];
    let logic = native(|step, _: &[Event<'_>], out| {
        let start = |emitter, points| Command::FxStart {
            emitter,
            effect: name_id("spark"),
            entity: Entity::NONE,
            position: [0.0; 3],
            points,
            seed: step as u32,
        };
        if step % 20 == 0 {
            send(out, start(1, PointList::EMPTY));
            send(out, start(2, PointList::Slice(&PATH)));
        }
        if step % 20 == 10 {
            send(
                out,
                Command::FxStop {
                    emitter: 2,
                    fade: 0.1,
                },
            );
            send(
                out,
                Command::FxBurst {
                    effect: name_id("spark"),
                    entity: Entity::NONE,
                    position: [1.0, 0.0, 0.0],
                    count: 30,
                    seed: 4,
                },
            );
        }
    });
    assert_eq!(steady_allocations_with(settings, logic), 0);
}

#[test]
fn steps_with_trails_and_highlights_coming_and_going_allocate_nothing() {
    use sariema_format::script::{PointList, name_id};
    const ROUTE: [[f32; 3]; 3] = [[0.0; 3], [3.0, 0.0, 0.0], [3.0, 0.0, -3.0]];
    let hero = entity(0);
    let logic = native(move |step, _: &[Event<'_>], out| {
        if step == 0 {
            send(
                out,
                Command::Spawn {
                    entity: hero,
                    kind: Kind::Empty,
                    asset: 0,
                },
            );
        }
        let trail = |trail| Command::Trail {
            trail,
            from: hero,
            points: PointList::Slice(&ROUTE),
            spacing: 0.5,
            size: 0.2,
            color: [1.0, 0.6, 0.2],
            intensity: 0.55,
            wave_speed: 2.0,
            fade_from: 0.5,
            fade_to: 0.5,
            fade: 0.1,
        };
        let highlight = |fill| Command::Highlight {
            entity: hero,
            node: name_id(""),
            fill,
            rim: fill,
            pulse: 1.0,
            fade: 0.1,
        };
        match step % 20 {
            0 => {
                send(out, trail(1));
                send(out, trail(2));
                send(out, highlight(0.3));
            }
            10 => {
                send(
                    out,
                    Command::TrailOff {
                        trail: 2,
                        fade: 0.05,
                    },
                );
                send(out, highlight(0.0));
            }
            _ => {}
        }
        send(
            out,
            Command::SetTransform {
                entity: hero,
                position: [0.01 * (step % 20) as f32, 0.0, 0.0],
                yaw: 0.0,
            },
        );
    });
    assert_eq!(steady_allocations_with(Settings::default(), logic), 0);
}
