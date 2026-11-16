// SPDX-License-Identifier: Apache-2.0

//! Everything that bounds or tunes the UI.

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiSettings {
    /// Size, in UI units, the screen is designed for, given in landscape;
    /// a portrait screen uses it turned. The UI is scaled so this size fits
    /// and the longer axis gets the extra room, like Godot's `expand`
    /// stretch aspect. Default 844 × 390, a phone held sideways.
    pub reference: [f32; 2],
    /// Seconds per simulation step. Default 1/60.
    pub step_s: f32,
    /// Side of the square glyph atlas, in texels. Default 1024.
    pub atlas_size: u32,
    /// Pixels per em glyphs are rasterised at before their distance field
    /// is computed. Default 48.
    pub sdf_px: f32,
    /// Distance, in rasterised pixels, the field reaches on each side of an
    /// outline; bounds the widest text outline and softest edge. Default 6.
    pub sdf_spread: f32,
    /// Animations that may run or wait at once; more are refused. Default 512.
    pub animations: usize,
    /// Spring integration steps per second; fixed so springs are
    /// deterministic. Default 240.
    pub spring_hz: f32,
    /// A spring is at rest closer than this to its target, in the
    /// property's units. Default 0.001.
    pub spring_rest_delta: f32,
    /// And slower than this, in units per second. Default 0.01.
    pub spring_rest_speed: f32,
    /// Pointers that may be down at once; more are ignored. Default 10.
    pub pointers: usize,
    /// Animations of position, scale, rotation and size jump to their end;
    /// opacity, colour, rings, strokes and text still animate, and the
    /// typewriter shows the whole text. Default false.
    pub reduced_motion: bool,
}

impl Default for UiSettings {
    fn default() -> UiSettings {
        UiSettings {
            reference: [844.0, 390.0],
            step_s: 1.0 / 60.0,
            atlas_size: 1024,
            sdf_px: 48.0,
            sdf_spread: 6.0,
            animations: 512,
            spring_hz: 240.0,
            spring_rest_delta: 0.001,
            spring_rest_speed: 0.01,
            pointers: 10,
            reduced_motion: false,
        }
    }
}

/// Insets, in physical pixels, the system keeps for notches and home
/// indicators.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
pub struct SafeArea {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
