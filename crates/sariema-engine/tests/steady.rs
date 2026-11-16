// SPDX-License-Identifier: Apache-2.0

//! A steady step allocates nothing, on every host: the hero walking under
//! the player's input with collision, its clip playing, music streaming and
//! mixing, a chime ringing and the HUD stepping. The music's decoder, which
//! allocates as it reads, runs between steps in the audio feed or on a
//! thread of its own; decoded at load, it allocates nowhere.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use glam::Vec2;
use sariema_format::script::{Bus, Command, Entity, Kind, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::{Logic, Stop};

use common::{assets, entity, send, wired};
use sariema_engine::{Engine, EngineSettings};
use sariema_runtime::sim::Simulation;

struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// Per thread, so tests running side by side count only their own.
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

/// Allocations counted in 600 steady steps and in the audio feeds after
/// them, with where the hero got to and what the mixer did.
struct Steady {
    steps: u64,
    feeds: u64,
    moved: glam::Vec3,
    mixed: sariema_audio::MixStats,
}

fn steady(settings: EngineSettings) -> Steady {
    let sim = Simulation::new(wired(), Vec::new(), Walker { step: 0 }, 1);
    let mut engine = Engine::new(sim, assets(), settings).unwrap();
    let mut actions = Actions::for_settings(&engine.state().settings.input);
    let walk = |actions: &mut Actions, n: u64| {
        let east = (n / 90).is_multiple_of(2);
        actions.axes[0] = Vec2::new(if east { 1.0 } else { -1.0 }, 0.0);
    };
    for n in 0..240 {
        walk(&mut actions, n);
        engine.step(&actions);
        engine.feed_audio();
    }
    let counted = |f: &mut dyn FnMut()| {
        let before = ALLOCATIONS.with(Cell::get);
        ARMED.with(|a| a.set(true));
        f();
        ARMED.with(|a| a.set(false));
        ALLOCATIONS.with(Cell::get) - before
    };
    let (mut steps, mut feeds) = (0, 0);
    for n in 240..840 {
        walk(&mut actions, n);
        steps += counted(&mut || engine.step(&actions));
        feeds += counted(&mut || engine.feed_audio());
    }
    let moved = engine.state().world.node(entity(0)).unwrap().local.position;
    let mixed = engine.sound().unwrap().audio.stats();
    drop(engine);
    assert!((moved.x - 2.5).abs() > 0.1, "the hero walked: {moved}");
    assert!(
        mixed.voices >= 1 && mixed.now >= 840 * 800,
        "the music played and was mixed: {mixed:?}"
    );
    Steady {
        steps,
        feeds,
        moved,
        mixed,
    }
}

#[test]
fn a_steady_step_allocates_nothing_with_streams_decoded_on_a_thread() {
    let mut settings = EngineSettings::default();
    settings.audio.stream_thread = true;
    let run = steady(settings);
    assert_eq!(run.steps, 0, "allocations over 600 steady steps");
}

#[test]
fn a_headless_steady_step_allocates_nothing_and_its_feed_decodes() {
    let run = steady(EngineSettings::default());
    assert_eq!(run.steps, 0, "allocations over 600 steady steps");
    assert!(run.feeds > 0, "the feeds decoded the music: {}", run.feeds);
    assert_eq!(run.mixed.underruns, 0, "{:?}", run.mixed);
}

#[test]
fn predecoded_streams_allocate_nothing_in_the_steps_or_the_feeds() {
    let mut settings = EngineSettings::default();
    settings.audio.predecode_streams = true;
    let run = steady(settings);
    assert_eq!(
        (run.steps, run.feeds),
        (0, 0),
        "{:?} {:?}",
        run.moved,
        run.mixed
    );
    assert_eq!(run.mixed.underruns, 0, "{:?}", run.mixed);
}

/// Slides the cinema bars in and out, fades the screen to black and back,
/// and plays the HUD's `enter` and `done` animations, twice; it sends
/// nothing else.
struct Animator {
    step: u64,
}

const CUES: [u64; 4] = [10, 60, 110, 160];

impl Logic for Animator {
    fn tick(&mut self, _events: &[u8], _ends: &[u32], out: &mut Vec<u8>) -> Result<(), Stop> {
        out.clear();
        let (to, play) = match self.step {
            10 | 110 => (1.0, "enter"),
            60 | 160 => (0.0, "done"),
            _ => {
                self.step += 1;
                return Ok(());
            }
        };
        send(out, Command::Letterbox { to, duration: 0.5 });
        send(
            out,
            Command::Fade {
                to,
                duration: 0.5,
                color: [0.0; 3],
            },
        );
        send(
            out,
            Command::UiPlay {
                animation: name_id(play),
            },
        );
        self.step += 1;
        Ok(())
    }
}

/// The first cues grow the buffers commands pass through, once; after
/// that, cueing allocates nothing either.
#[test]
fn a_step_that_only_animates_the_hud_bars_and_fade_allocates_nothing() {
    let sim = Simulation::new(wired(), Vec::new(), Animator { step: 0 }, 1);
    let mut engine = Engine::new(sim, assets(), EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..10 {
        engine.step(&actions);
    }
    let (mut animating, mut cued, mut later, mut bars) = (Vec::new(), Vec::new(), 0, 0);
    for step in 10..230 {
        let before = ALLOCATIONS.with(Cell::get);
        ARMED.with(|a| a.set(true));
        engine.step(&actions);
        ARMED.with(|a| a.set(false));
        let n = ALLOCATIONS.with(Cell::get) - before;
        if CUES.contains(&step) {
            cued.push((step, n));
            later += if step >= CUES[2] { n } else { 0 };
        } else if n > 0 {
            animating.push((step, n));
        }
        let s = engine.state();
        if s.screen.letterbox.value() > 0.0 && s.screen.letterbox.value() < 1.0 {
            bars += 1;
        }
    }
    assert!(bars >= 100, "the bars slid in and out: {bars} steps");
    assert_eq!(animating, [], "allocations on steps that only animate");
    assert_eq!(later, 0, "allocations on the cues: {cued:?}");
}

#[test]
fn a_resized_screen_is_laid_out_before_the_next_step() {
    let sim = Simulation::new(wired(), Vec::new(), Animator { step: 0 }, 1);
    let mut engine = Engine::new(sim, assets(), EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..5 {
        engine.step(&actions);
    }
    engine.resize(1280.0, 720.0, sariema_ui::SafeArea::default());
    let before = ALLOCATIONS.with(Cell::get);
    ARMED.with(|a| a.set(true));
    engine.step(&actions);
    ARMED.with(|a| a.set(false));
    assert_eq!(ALLOCATIONS.with(Cell::get) - before, 0);
}
