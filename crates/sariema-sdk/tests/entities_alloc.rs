// SPDX-License-Identifier: Apache-2.0

//! A game that reserves its entity table for the count it knows at setup
//! spawns that many entities, and despawns them, without allocating.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use sariema_sdk::testing::TestHost;
use sariema_sdk::{Ctx, Entity, Event, Events, Game, Kind, Runner, id};

struct Counting;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards to the system allocator, only counting the calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const PIECES: usize = 300;

struct Board {
    pieces: [Entity; PIECES],
    allocated: Option<usize>,
}

impl Game for Board {
    fn new(ctx: &mut Ctx<'_>) -> Board {
        ctx.reserve_entities(PIECES);
        Board {
            pieces: [Entity::NONE; PIECES],
            allocated: None,
        }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, _: Events<'_>) {
        ALLOCATIONS.store(0, Ordering::Relaxed);
        COUNTING.store(true, Ordering::Relaxed);
        for piece in &mut self.pieces {
            *piece = ctx.spawn(Kind::Empty, id("piece"));
        }
        for &piece in &self.pieces {
            ctx.despawn(piece);
        }
        COUNTING.store(false, Ordering::Relaxed);
        self.allocated = Some(ALLOCATIONS.load(Ordering::Relaxed));
    }
}

#[test]
fn spawning_and_despawning_the_entities_reserved_allocates_nothing() {
    let mut host = TestHost::new();
    host.event(Event::Started { seed: 1 });
    let mut runner = Runner::<Board>::new();
    runner.run(&mut host);
    assert_eq!(runner.game().unwrap().allocated, Some(0));
}
