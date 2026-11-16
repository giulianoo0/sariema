// SPDX-License-Identifier: Apache-2.0

//! With the `alloc-count` feature, the global allocator: the system's,
//! counting every allocation and `realloc` on the thread that makes it, so
//! a test build can report what a step allocates.

#![cfg(feature = "alloc-count")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn count() {
    ALLOCATIONS.with(|n| n.set(n.get() + 1));
}

// SAFETY: every call is forwarded unchanged to `System`.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// Allocations this thread has made so far.
pub fn so_far() -> u64 {
    ALLOCATIONS.with(Cell::get)
}
