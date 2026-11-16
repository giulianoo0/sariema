// SPDX-License-Identifier: Apache-2.0

//! Cutscenes: authored JSON compiled into the game pack, checked against
//! the no-teleport rules at build time, and played by the [`Sequencer`].

pub mod author;
pub mod check;
mod play;

pub use play::{LINE, RIG, Sequencer, TWEENS};
