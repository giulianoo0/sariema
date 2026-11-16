// SPDX-License-Identifier: Apache-2.0

//! The Sariema engine in a web page or an app's WebView. A [`Stage`] holds
//! the renderer, actors, UI, sounds, the fixed-step clock and the input
//! queue and draws into any texture; on `wasm32` the `Engine` class built
//! with `wasm-bindgen` puts it on a canvas through WebGPU, or WebGL2 where
//! the browser has no WebGPU, and the page drives it from its
//! `requestAnimationFrame` loop. See `docs/web.md`.

pub mod clock;
pub mod feed;
pub mod game;
pub mod input;
pub mod pose;
pub mod settings;
pub mod stage;
pub mod textures;
#[cfg(target_arch = "wasm32")]
mod web;

pub use stage::{Stage, StageError};
