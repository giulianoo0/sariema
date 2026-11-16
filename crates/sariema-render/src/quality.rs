// SPDX-License-Identifier: Apache-2.0

use crate::cluster::{MAX_CLUSTER_LIGHTS, MAX_INDEX_ENTRIES};
use crate::lights::GPU_LIGHT_SIZE;
use crate::post::MAX_BLUR_TAPS;
use crate::scene::{DEPTH_FORMAT, HDR_FORMAT};
use crate::shadow::{GPU_SHADOW_SIZE, view_stride};

/// Renderer settings that trade cost for coverage. Every field is a default
/// a game may override; [`crate::Renderer::new`] clamps them to the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quality {
    /// Local lights shaded per frame; past it, the lowest priority ones are dropped.
    pub max_visible_lights: u32,
    pub max_lights_per_cluster: u32,
    /// Most clusters in the grid; tiles grow from `cluster_tile_px` to fit.
    pub cluster_budget: u32,
    pub cluster_tile_px: u32,
    pub depth_slices: u32,
    /// Samples per pixel of the scene target.
    pub msaa: u32,
    /// Side in texels of the one depth texture every shadow map shares.
    pub shadow_atlas_size: u32,
    /// Most memory the shadow atlas may take; a larger `shadow_atlas_size`
    /// is reduced to fit.
    pub shadow_atlas_max_bytes: u64,
    /// Local lights with a shadow map; the sun's does not count.
    pub max_shadowed_lights: u32,
    /// Smallest shadow map side in texels; a light that would get less is
    /// left unshadowed.
    pub shadow_min_slot_px: u32,
    /// Depth comparisons per shadow lookup, as the largest square grid that
    /// fits; each one filters 2×2 texels.
    pub shadow_pcf_taps: u32,
    /// Blurred levels summed into the bloom, from 1/4 of the target's size
    /// down, each half the size of the one before.
    pub bloom_levels: u32,
    /// Bilinear samples per direction of each post-processing blur, odd and
    /// at most 9; each one past the centre weighs two texels.
    pub blur_taps: u32,
}

impl Quality {
    pub fn low() -> Quality {
        Quality {
            max_visible_lights: 64,
            max_lights_per_cluster: 16,
            cluster_budget: 1536,
            cluster_tile_px: 64,
            depth_slices: 16,
            msaa: 2,
            shadow_atlas_size: 1024,
            shadow_atlas_max_bytes: 4 << 20,
            max_shadowed_lights: 4,
            shadow_min_slot_px: 64,
            shadow_pcf_taps: 1,
            // The same in every tier: they set how far the bloom and the
            // miniature blur reach, which is part of the look.
            bloom_levels: 4,
            blur_taps: 5,
        }
    }

    pub fn medium() -> Quality {
        Quality {
            max_visible_lights: 256,
            max_lights_per_cluster: 32,
            cluster_budget: 3072,
            msaa: 4,
            shadow_atlas_size: 2048,
            shadow_atlas_max_bytes: 16 << 20,
            max_shadowed_lights: 12,
            shadow_min_slot_px: 128,
            shadow_pcf_taps: 4,
            ..Quality::low()
        }
    }

    pub fn high() -> Quality {
        Quality {
            max_visible_lights: 1024,
            max_lights_per_cluster: 64,
            cluster_budget: 6144,
            cluster_tile_px: 32,
            shadow_atlas_size: 4096,
            shadow_atlas_max_bytes: 64 << 20,
            max_shadowed_lights: 32,
            shadow_min_slot_px: 256,
            shadow_pcf_taps: 16,
            ..Quality::medium()
        }
    }

    /// These settings reduced to what `device` and the cluster packing can hold.
    pub(crate) fn clamped(&self, device: &wgpu::Device) -> Quality {
        Quality {
            msaa: supported_samples(device, self.msaa),
            ..self.clamped_to(&device.limits())
        }
    }

    /// [`Quality::clamped`] for everything but `msaa`, which needs the device.
    fn clamped_to(&self, limits: &wgpu::Limits) -> Quality {
        let storage = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        let entries = |size: u64| (storage / size).min(u32::MAX as u64) as u32;
        let max_lights_per_cluster = self.max_lights_per_cluster.clamp(1, MAX_CLUSTER_LIGHTS);
        let indices = entries(4).min(MAX_INDEX_ENTRIES);
        let cluster_budget = self
            .cluster_budget
            .min(indices / max_lights_per_cluster)
            .max(1);
        let texel_bytes = DEPTH_FORMAT.block_copy_size(None).expect("one aspect") as u64;
        let atlas_side_in_budget = (self.shadow_atlas_max_bytes / texel_bytes).isqrt();
        let shadow_atlas_size = (self.shadow_atlas_size as u64)
            .min(atlas_side_in_budget)
            .clamp(1, limits.max_texture_dimension_2d as u64)
            as u32;
        let shadow_min_slot_px = self.shadow_min_slot_px.max(1);
        let min_slots_per_side = (shadow_atlas_size / shadow_min_slot_px) as u64;
        let min_slots = (min_slots_per_side * min_slots_per_side).min(u32::MAX as u64) as u32;
        let blur_taps = self.blur_taps.clamp(1, MAX_BLUR_TAPS);
        Quality {
            max_visible_lights: self.max_visible_lights.min(entries(GPU_LIGHT_SIZE)),
            max_lights_per_cluster,
            cluster_budget,
            cluster_tile_px: self.cluster_tile_px.max(1),
            depth_slices: self.depth_slices.clamp(1, cluster_budget),
            shadow_atlas_size,
            // The slot buffers also hold the sun's.
            max_shadowed_lights: self
                .max_shadowed_lights
                .min(min_slots)
                .min(entries(view_stride(limits).max(GPU_SHADOW_SIZE)).saturating_sub(1)),
            shadow_min_slot_px,
            bloom_levels: self.bloom_levels.max(1),
            blur_taps: if blur_taps.is_multiple_of(2) {
                blur_taps - 1
            } else {
                blur_taps
            },
            ..self.clone()
        }
    }
}

/// The largest power of two up to `wanted` that the scene's colour and depth
/// formats accept, found by creating a probe texture of each.
fn supported_samples(device: &wgpu::Device, wanted: u32) -> u32 {
    let accepts = |count: u32| {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        for format in [HDR_FORMAT, DEPTH_FORMAT] {
            drop(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("sample count probe"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }));
        }
        pollster::block_on(scope.pop()).is_none()
    };
    (1..=6)
        .rev()
        .map(|log2| 1 << log2)
        .find(|&count| count <= wanted && accepts(count))
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_fit_their_own_budgets() {
        for tier in [Quality::low(), Quality::medium(), Quality::high()] {
            assert_eq!(tier.clamped_to(&wgpu::Limits::default()), tier);
        }
    }

    #[test]
    fn shadow_settings_are_held_to_memory_and_slot_size() {
        let greedy = Quality {
            shadow_atlas_size: u32::MAX,
            max_shadowed_lights: u32::MAX,
            shadow_pcf_taps: 0,
            ..Quality::high()
        };
        let q = greedy.clamped_to(&wgpu::Limits::default());
        let side = q.shadow_atlas_size as u64;
        assert!(side * side * 4 <= q.shadow_atlas_max_bytes, "{q:?}");
        assert_eq!(q.shadow_atlas_size, Quality::high().shadow_atlas_size);
        let per_side = q.shadow_atlas_size / q.shadow_min_slot_px;
        assert_eq!(q.max_shadowed_lights, per_side * per_side);
    }

    #[test]
    fn post_settings_are_held_to_usable_values() {
        let limits = wgpu::Limits::default();
        let odd = Quality {
            bloom_levels: 0,
            blur_taps: 4,
            ..Quality::high()
        }
        .clamped_to(&limits);
        assert_eq!((odd.bloom_levels, odd.blur_taps), (1, 3));
        let wide = Quality {
            blur_taps: u32::MAX,
            ..Quality::high()
        }
        .clamped_to(&limits);
        assert_eq!(wide.blur_taps, MAX_BLUR_TAPS);
    }
}
