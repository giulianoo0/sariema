// SPDX-License-Identifier: Apache-2.0

//! A game that reserves room for its largest `scene_tween` sends it without
//! allocating, even when smaller ones came first.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use sariema_sdk::testing::TestHost;
use sariema_sdk::{Ctx, Ease, Event, Events, Game, Runner, id};

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

const LARGEST: usize = 64;
const WARM_UP: u64 = 10;

struct Dusk {
    values: Vec<(u32, f32)>,
    allocated: Option<usize>,
}

impl Game for Dusk {
    fn new(ctx: &mut Ctx<'_>) -> Dusk {
        ctx.reserve_scene_tween(LARGEST);
        Dusk {
            values: (0..LARGEST as u32)
                .map(|k| (id("fog.density") + k, 0.5))
                .collect(),
            allocated: None,
        }
    }

    fn tick(&mut self, ctx: &mut Ctx<'_>, _: Events<'_>) {
        if ctx.step() < WARM_UP {
            ctx.scene_tween(1, &self.values[..2], 1.0, Ease::Linear);
            return;
        }
        ALLOCATIONS.store(0, Ordering::Relaxed);
        COUNTING.store(true, Ordering::Relaxed);
        ctx.scene_tween(1, &self.values, 1.0, Ease::Linear);
        COUNTING.store(false, Ordering::Relaxed);
        self.allocated = Some(ALLOCATIONS.load(Ordering::Relaxed));
    }
}

#[test]
fn the_largest_scene_tween_reserved_is_sent_after_warm_up_without_allocating() {
    let mut host = TestHost::new();
    host.event(Event::Started { seed: 1 });
    let mut runner = Runner::<Dusk>::new();
    for _ in 0..=WARM_UP {
        runner.run(&mut host);
        host.take_commands();
    }
    let allocated = runner.game().unwrap().allocated;
    assert_eq!(allocated, Some(0), "allocations sending the largest tween");
}
