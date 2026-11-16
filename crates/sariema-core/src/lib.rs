// SPDX-License-Identifier: Apache-2.0

//! Renderer-independent core of the Sariema engine.

mod camera;

pub use camera::{Camera, pose_camera, room_camera};

/// Simulation steps per second. A simulation advances by exactly
/// [`STEP_SECONDS`] per step at any display rate.
pub const STEP_HZ: u32 = 60;
pub const STEP_SECONDS: f32 = 1.0 / STEP_HZ as f32;
