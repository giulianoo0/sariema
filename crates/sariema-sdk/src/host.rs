// SPDX-License-Identifier: Apache-2.0

//! The four functions the engine gives a game, behind a trait so a game's
//! logic can run in host tests too ([`crate::testing`]).

pub trait Host {
    fn submit(&mut self, bytes: &[u8]);
    /// Copies the next whole events into `dst`: the bytes copied, 0 once
    /// all were read, or minus the size of an event larger than `dst`.
    fn events(&mut self, dst: &mut [u8]) -> i32;
    /// Registers `bytes` as a bulk region of `kind`; the engine reads them
    /// in place after every tick. The region's id, or −1.
    fn register(&mut self, kind: u32, bytes: &[u8]) -> i32;
    /// `level` 0 debug, 1 info, 2 warning, 3 error.
    fn log(&mut self, level: u32, text: &str);
}

/// The engine, through the module's imports.
pub struct Wasm;

#[cfg(target_arch = "wasm32")]
mod imports {
    #[link(wasm_import_module = "sariema")]
    unsafe extern "C" {
        pub fn submit(ptr: *const u8, len: usize);
        pub fn events(dst: *mut u8, cap: usize) -> i32;
        pub fn register(kind: u32, ptr: *const u8, len: usize) -> i32;
        pub fn log(level: u32, ptr: *const u8, len: usize);
    }
}

#[cfg(target_arch = "wasm32")]
impl Host for Wasm {
    fn submit(&mut self, bytes: &[u8]) {
        // SAFETY: the pointer and length describe a live slice.
        unsafe { imports::submit(bytes.as_ptr(), bytes.len()) }
    }

    fn events(&mut self, dst: &mut [u8]) -> i32 {
        // SAFETY: the engine writes at most `dst.len()` bytes into `dst`.
        unsafe { imports::events(dst.as_mut_ptr(), dst.len()) }
    }

    fn register(&mut self, kind: u32, bytes: &[u8]) -> i32 {
        // SAFETY: the engine only reads the region, and checks its bounds.
        unsafe { imports::register(kind, bytes.as_ptr(), bytes.len()) }
    }

    fn log(&mut self, level: u32, text: &str) {
        // SAFETY: the pointer and length describe a live string.
        unsafe { imports::log(level, text.as_ptr(), text.len()) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Host for Wasm {
    fn submit(&mut self, _: &[u8]) {
        unreachable!("the engine's imports exist only on wasm32")
    }

    fn events(&mut self, _: &mut [u8]) -> i32 {
        unreachable!("the engine's imports exist only on wasm32")
    }

    fn register(&mut self, _: u32, _: &[u8]) -> i32 {
        unreachable!("the engine's imports exist only on wasm32")
    }

    fn log(&mut self, _: u32, _: &str) {
        unreachable!("the engine's imports exist only on wasm32")
    }
}
