// SPDX-License-Identifier: Apache-2.0

//! Where a game's logic will plug in. The game runtime (`sariema-runtime`)
//! runs a game's WebAssembly module natively in an interpreter; on the web
//! the browser's own WebAssembly engine runs it instead, so the page
//! instantiates the module with the runtime's host functions as imports and
//! the engine calls it once per fixed step through this trait. Nothing
//! implements it yet: without a game the engine shows what the page loads
//! and poses.

use crate::input::InputEvent;

/// A game's logic, stepped by the engine.
pub trait Game {
    /// Advances the game by one fixed step of `step_s` seconds, after the
    /// input of the frames since the last step. Commands to the renderer,
    /// the audio and the UI go through the host functions the game was
    /// instantiated with, not through this call.
    fn step(&mut self, step_s: f64, input: &[InputEvent]);
}
