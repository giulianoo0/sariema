// SPDX-License-Identifier: Apache-2.0

//! A game casts rays against the active room during its tick: walls and
//! actors as they stood when the step began, the nearest hit with its
//! normal, and no allocation once warmed up.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::script::ray::{RAY_HIT, RAY_INVALID, RAY_MISS, Ray, RayHit};
use sariema_format::script::{Entity, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::{Logic, Queries, Simulation, Stop};

use common::{assets, block, collision, entity, spawn_hero, wired};

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

/// A floor with a wall whose near face is at x = 4.6.
fn room() -> RoomAssets {
    let colliders = vec![
        block("floor", [-6.0, -0.1, -4.0], [8.0, 0.0, 4.0]),
        block("wall", [4.6, 0.0, -4.0], [4.8, 2.0, 4.0]),
    ];
    RoomAssets::collision("walled", collision("walled", colliders))
}

/// Spawns the hero at the origin, then casts `ray` every tick from the
/// third on and keeps the last answer.
struct Caster {
    step: u64,
    ray: Ray,
    query: [u8; Ray::BYTES],
    hit: [u8; RayHit::BYTES],
    answer: Option<(i32, Option<RayHit>)>,
}

impl Logic for Caster {
    fn tick(&mut self, _events: &[u8], _ends: &[u32], _out: &mut Vec<u8>) -> Result<(), Stop> {
        unreachable!("the engine ticks with its queries")
    }

    fn tick_with(
        &mut self,
        _events: &[u8],
        _ends: &[u32],
        out: &mut Vec<u8>,
        queries: &mut dyn Queries,
    ) -> Result<(), Stop> {
        out.clear();
        if self.step == 0 {
            spawn_hero(out, [0.0, 0.0, 0.0], 0.0);
        }
        if self.step >= 2 {
            self.ray.write(&mut self.query);
            let answer = queries.ray(&self.query, &mut self.hit);
            let hit = (answer == RAY_HIT).then(|| RayHit::read(&self.hit).unwrap());
            self.answer = Some((answer, hit));
        }
        self.step += 1;
        Ok(())
    }
}

fn engine(ray: Ray) -> Engine<Caster> {
    let caster = Caster {
        step: 0,
        ray,
        query: [0; Ray::BYTES],
        hit: [0; RayHit::BYTES],
        answer: None,
    };
    let mut assets = assets();
    assets.rooms = vec![room()];
    let sim = Simulation::new(wired(), Vec::new(), caster, 1);
    Engine::new(sim, assets, EngineSettings::default()).unwrap()
}

fn cast(ray: Ray) -> (i32, Option<RayHit>) {
    let mut engine = engine(ray);
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..3 {
        engine.step(&actions);
    }
    engine.simulation().logic().answer.unwrap()
}

/// Along +X at waist height from `x`, seeing everything but `exclude`.
fn east(x: f32, exclude: Entity) -> Ray {
    Ray {
        origin: [x, 0.5, 0.0],
        dir: [1.0, 0.0, 0.0],
        max: 20.0,
        colliders: true,
        actors: true,
        exclude,
    }
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
}

#[test]
fn a_ray_hits_the_wall_with_its_normal_and_distance() {
    let (answer, hit) = cast(east(1.0, Entity::NONE));
    let hit = hit.unwrap();
    assert_eq!(answer, RAY_HIT);
    assert!((hit.distance - 3.6).abs() < 1e-3, "{hit:?}");
    assert!(close(hit.point, [4.6, 0.5, 0.0]), "{hit:?}");
    assert!(close(hit.normal, [-1.0, 0.0, 0.0]), "{hit:?}");
    assert_eq!(hit.entity, Entity::NONE);
    assert_eq!(hit.collider, name_id("wall"));
}

#[test]
fn a_ray_hits_the_hero_in_its_way_unless_it_is_excluded() {
    let (_, hit) = cast(east(-3.0, Entity::NONE));
    let hit = hit.unwrap();
    assert_eq!(hit.entity, entity(0));
    assert_eq!(hit.collider, 0);
    assert!((hit.distance - 2.7).abs() < 1e-3, "{hit:?}");
    assert!(close(hit.normal, [-1.0, 0.0, 0.0]), "{hit:?}");

    let (_, hit) = cast(east(-3.0, entity(0)));
    let hit = hit.unwrap();
    assert_eq!(hit.collider, name_id("wall"));
    assert!((hit.distance - 7.6).abs() < 1e-3, "{hit:?}");
}

#[test]
fn a_ray_misses_what_is_out_of_reach_and_refuses_a_direction_of_zero() {
    let short = Ray {
        max: 3.0,
        ..east(1.0, Entity::NONE)
    };
    assert_eq!(cast(short), (RAY_MISS, None));
    let walls_only = Ray {
        actors: false,
        colliders: false,
        ..east(-3.0, Entity::NONE)
    };
    assert_eq!(cast(walls_only), (RAY_MISS, None));
    let nowhere = Ray {
        dir: [0.0; 3],
        ..east(1.0, Entity::NONE)
    };
    assert_eq!(cast(nowhere), (RAY_INVALID, None));
}

#[test]
fn casting_a_ray_every_tick_allocates_nothing() {
    let mut engine = engine(east(-3.0, Entity::NONE));
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..30 {
        engine.step(&actions);
    }
    ARMED.with(|a| a.set(true));
    for _ in 0..120 {
        engine.step(&actions);
    }
    ARMED.with(|a| a.set(false));
    assert_eq!(engine.simulation().logic().answer.unwrap().0, RAY_HIT);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
