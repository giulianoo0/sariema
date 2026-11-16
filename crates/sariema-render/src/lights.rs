// SPDX-License-Identifier: Apache-2.0

//! A room light as the scene shader reads it.

use bytemuck::{Pod, Zeroable};
use sariema_format::{Light, LightKind};

use crate::cluster::spot_axis;

pub(crate) const GPU_LIGHT_SIZE: u64 = size_of::<GpuLight>() as u64;
const SPOT: u32 = 1;
/// The game creates every spot with penumbra 0.5 (`casa.ts`) and the pack has
/// no penumbra field.
const SPOT_PENUMBRA: f32 = 0.5;

/// Mirrors `Light` in scene.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuLight {
    position: [f32; 3],
    range: f32,
    color: [f32; 3],
    cone_cos: f32,
    direction: [f32; 3],
    penumbra_cos: f32,
    shadow: i32,
    flags: u32,
    _pad: [u32; 2],
}

/// `light`, the `index`-th of its room, at `time_s`, with its shadow slot.
pub(crate) fn gpu_light(light: &Light, index: usize, time_s: f32, shadow: Option<u32>) -> GpuLight {
    let flicker = if light.flicker.is_some() {
        flicker(time_s, index)
    } else {
        1.0
    };
    let (direction, cone_cos, penumbra_cos, flags) = match light.kind {
        LightKind::Point => ([0.0; 3], -1.0, 1.0, 0),
        LightKind::Spot { target, cone_deg } => {
            let half = (cone_deg * 0.5).to_radians();
            (
                (-spot_axis(light.position, target)).into(),
                half.cos(),
                (half * (1.0 - SPOT_PENUMBRA)).cos(),
                SPOT,
            )
        }
    };
    GpuLight {
        position: light.position,
        range: light.range,
        color: light.color.map(|c| c * light.intensity * flicker),
        cone_cos,
        direction,
        penumbra_cos,
        shadow: shadow.map_or(-1, |slot| slot as i32),
        flags,
        _pad: [0; 2],
    }
}

/// Smooth multiplier between 0.85 and 1.0 near 8 Hz, the curve the game
/// gives its TV light; `index` shifts the phase so lights do not pulse together.
fn flicker(time_s: f32, index: usize) -> f32 {
    let t = time_s + index as f32 * 0.37;
    0.85 + 0.15 * (0.5 + 0.5 * (t * 50.0 + (t * 13.0).sin() * 3.0).sin())
}

#[cfg(test)]
mod tests {
    #[test]
    fn flicker_stays_between_0_85_and_1_and_moves() {
        let values: Vec<f32> = (0..120)
            .map(|i| super::flicker(i as f32 / 60.0, 3))
            .collect();
        assert!(
            values.iter().all(|v| (0.85..=1.0).contains(v)),
            "{values:?}"
        );
        assert!(values.iter().any(|&v| v < 0.9) && values.iter().any(|&v| v > 0.95));
    }
}
