// SPDX-License-Identifier: Apache-2.0

//! A steady step of a game playing to a device allocates nothing: the hero
//! walking under the player's input with collision, its clip playing, music
//! streaming and mixing, a chime ringing and the HUD stepping. The music's
//! decoder, which allocates as it reads, runs on its own thread there.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

use glam::Vec2;
use sariema_format::script::{Bus, Command, Entity, Kind, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::{Logic, Stop};

use common::{assets, entity, send, wired};
use sariema_engine::{Engine, EngineSettings};
use sariema_runtime::sim::Simulation;

struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

// SAFETY: every call is passed on to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.with(Cell::get) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.with(Cell::get) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Spawns the hero, hands it to the player and starts the music; then
/// rings a chime at it every second. Writes commands without allocating.
struct Walker {
    step: u64,
}

impl Logic for Walker {
    fn tick(&mut self, _events: &[u8], _ends: &[u32], out: &mut Vec<u8>) -> Result<(), Stop> {
        out.clear();
        let hero = entity(0);
        if self.step == 0 {
            send(
                out,
                Command::Spawn {
                    entity: hero,
                    kind: Kind::Actor,
                    asset: name_id("hero"),
                },
            );
            send(
                out,
                Command::SetTransform {
                    entity: hero,
                    position: [2.5, 0.0, -1.25],
                    yaw: 0.0,
                },
            );
            send(
                out,
                Command::SetHero {
                    entity: hero,
                    speed: 1.5,
                },
            );
            send(
                out,
                Command::PlayMusic {
                    music: name_id("theme"),
                    fade: 0.5,
                },
            );
        }
        if self.step % 60 == 30 {
            send(
                out,
                Command::PlaySound {
                    sound: name_id("chime"),
                    bus: Bus::Effects,
                    emitter: Entity::NONE,
                    volume: 0.5,
                },
            );
        }
        self.step += 1;
        Ok(())
    }
}

#[test]
fn a_steady_step_allocates_nothing() {
    let sim = Simulation::new(wired(), Vec::new(), Walker { step: 0 }, 1);
    let mut settings = EngineSettings::default();
    settings.audio.stream_thread = true;
    let mut engine = Engine::new(sim, assets(), settings).unwrap();
    let mut actions = Actions::for_settings(&engine.state().settings.input);
    let walk = |actions: &mut Actions, n: u64| {
        let east = (n / 90).is_multiple_of(2);
        actions.axes[0] = Vec2::new(if east { 1.0 } else { -1.0 }, 0.0);
    };
    for n in 0..240 {
        walk(&mut actions, n);
        engine.step(&actions);
    }
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    ARMED.with(|a| a.set(true));
    for n in 240..840 {
        walk(&mut actions, n);
        engine.step(&actions);
    }
    ARMED.with(|a| a.set(false));
    let allocations = ALLOCATIONS.load(Ordering::Relaxed) - before;
    let moved = engine.state().world.node(entity(0)).unwrap().local.position;
    let mixed = engine.sound().unwrap().audio.stats();
    drop(engine);
    assert_eq!(allocations, 0, "allocations over 600 steady steps");
    assert!((moved.x - 2.5).abs() > 0.1, "the hero walked: {moved}");
    assert!(
        mixed.voices >= 1 && mixed.now >= 840 * 800,
        "the music played and was mixed: {mixed:?}"
    );
}
