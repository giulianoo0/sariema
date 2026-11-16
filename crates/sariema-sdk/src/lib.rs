// SPDX-License-Identifier: Apache-2.0

//! The SDK a Sariema game is written with. The game is a `cdylib` built for
//! `wasm32-unknown-unknown`; it implements [`Game`] and names its type in
//! [`game!`]. Each step the engine calls the module's `sariema_tick`, which
//! hands the game this step's events and submits the commands it sent.
//!
//! The command and event types are generated from the same table the
//! engine's decoder is generated from (`sariema-format/src/script/table.rs`),
//! so the two can never disagree.
//!
//! Needs `alloc`: a game built with std has it; a no_std game provides a
//! global allocator.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[macro_use]
#[path = "../../sariema-format/src/script/codegen.rs"]
mod codegen;
#[path = "../../sariema-format/src/script/table.rs"]
mod table;
#[path = "../../sariema-format/src/script/wire.rs"]
mod wire;

#[path = "../../sariema-runtime/src/rng.rs"]
mod rng;

mod ctx;
pub mod host;
mod machine;
#[cfg(all(feature = "panic-handler", target_arch = "wasm32"))]
mod panic;
mod region;
mod run;
pub mod testing;

pub use ctx::{Ctx, Entities};
pub use machine::Machine;
pub use region::Transforms;
pub use rng::Pcg32 as Rng;
pub use run::{EventIter, Events, Game, Runner};
pub use table::{BATCH_VERSION, COMMANDS, Command, ENUMS, EVENTS, Event};
pub use wire::{
    Advance, Anchor, Bus, ClipMode, Controller, Ease, Entity, Kind, Orientation, Property, Reader,
    RoomTransition, Value, WireError, name_id, put_scene_value,
};

/// The id of a name (flag, cutscene, role, clip, sound, signal), computed at
/// compile time when the name is a constant.
pub const fn id(name: &str) -> u32 {
    name_id(name)
}

/// Sends a panic's message to the engine's log as an error. A game built
/// with std calls it from its panic hook; the `panic-handler` feature does
/// it for a no_std game.
pub fn report_panic(message: &dyn core::fmt::Display) {
    use core::fmt::Write;
    let mut line = Line::default();
    let _ = write!(line, "{message}");
    host::Host::log(&mut host::Wasm, 3, line.as_str());
}

/// A panic message, cut at 512 bytes.
struct Line {
    bytes: [u8; 512],
    len: usize,
}

impl Default for Line {
    fn default() -> Line {
        Line {
            bytes: [0; 512],
            len: 0,
        }
    }
}

impl Line {
    fn as_str(&self) -> &str {
        match core::str::from_utf8(&self.bytes[..self.len]) {
            Ok(s) => s,
            Err(e) => core::str::from_utf8(&self.bytes[..e.valid_up_to()]).unwrap_or_default(),
        }
    }
}

impl core::fmt::Write for Line {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let n = s.len().min(self.bytes.len() - self.len);
        self.bytes[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// Exports `sariema_tick` and `sariema_batch_version` for the game type.
#[macro_export]
macro_rules! game {
    ($game:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn sariema_tick() {
            static RUNNER: $crate::Runner<$game> = $crate::Runner::new();
            // SAFETY: a wasm32 module runs on one thread and the engine never
            // calls `sariema_tick` again before it returns.
            unsafe { RUNNER.tick(&mut $crate::host::Wasm) }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn sariema_batch_version() -> i32 {
            $crate::BATCH_VERSION as i32
        }
    };
}
