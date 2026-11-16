// SPDX-License-Identifier: Apache-2.0

//! Counts heap allocations made on the thread of the test, so the harness's
//! own threads do not count.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::*;
use sariema_anim::{JointMask, LayerBlend, Play};

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.with(|n| n.set(n.get() + 1));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations() -> u64 {
    ALLOCATIONS.with(Cell::get)
}

#[test]
fn stepping_and_evaluating_allocate_nothing() {
    let rig = arm();
    let mut a = animator(&rig);
    let wrist = rig.joint("wrist").unwrap();
    a.set_layer(
        1,
        LayerBlend::Override,
        Some(&JointMask::subtree(&rig, wrist)),
    );
    let (bend, wave, spin) = (clip(&rig, "bend"), clip(&rig, "wave"), clip(&rig, "spin"));
    let before = allocations();
    for step in 0..2000 {
        match step % 97 {
            0 => a.play(&rig, 0, bend, Play::default()),
            31 => a.play(&rig, 0, spin, Play::default()),
            33 => a.play(&rig, 1, wave, Play::default()),
            60 => a.stop(1, None),
            _ => {}
        }
        a.step(&rig);
        for alpha in [0.25, 0.75] {
            a.evaluate(&rig, alpha);
        }
        std::hint::black_box((a.palette(), a.events(), a.faces()));
    }
    assert_eq!(allocations() - before, 0);
}
