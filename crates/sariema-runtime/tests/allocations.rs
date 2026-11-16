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
    let settings = Settings::default();
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
