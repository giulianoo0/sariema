// SPDX-License-Identifier: Apache-2.0

use crate::cluster::{MAX_CLUSTER_LIGHTS, MAX_INDEX_ENTRIES};
use crate::lights::GPU_LIGHT_SIZE;
use crate::post::{MAX_BLUR_TAPS, ao};
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
    /// A blur over a level of at most this many texels runs in one pass of
    /// `blur_taps`² samples instead of two of `blur_taps`; past it the
    /// samples cost more than the pass saves. The bloom's first level, whose
    /// one-pass form takes twice the samples, always takes two.
    pub blur_one_pass_max_texels: u32,
    /// Depth samples per texel of the screen-space ambient occlusion, at most
    /// 32; 0 turns the occlusion off whatever the look asks.
    pub ao_samples: u32,
    /// The occlusion is computed at 1/`ao_divisor` of the target's size, 1 to 8.
    pub ao_divisor: u32,
    /// Texels per direction of the occlusion's depth-aware blur, rounded to
    /// 5 or 9: the counts that average away the 4×4 pattern that turns its
    /// samples.
    pub ao_blur_taps: u32,
    /// Most texels of the occlusion target the radius may span; a surface
    /// near enough to the camera to need more gets a smaller radius.
    pub ao_max_radius_texels: u32,
    /// Leaves out the room draws and actors whose bounds lie outside the
    /// view. Off draws everything, for comparing images.
    pub frustum_culling: bool,
    /// Shadow slots of a frame, the sun's first and then the highest-ranked
    /// lights', that also hold actors; each is redrawn in the frames an actor
    /// within its reach moves. An actor that none holds gets a disc shadow,
    /// so 0 gives every actor a disc.
    pub actor_shadow_slots: u32,
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
            // 1/4 of 1920×1080 is 129 600 texels; its 1/2 level, 518 400,
            // blurs faster in two passes on an Apple M5.
            blur_one_pass_max_texels: 1 << 17,
            ao_samples: 6,
            ao_divisor: 4,
            ao_blur_taps: 5,
            ao_max_radius_texels: 24,
            frustum_culling: true,
            actor_shadow_slots: 0,
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
            ao_samples: 8,
            ao_max_radius_texels: 32,
            actor_shadow_slots: 1,
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
            ao_samples: 12,
            ao_blur_taps: 9,
            actor_shadow_slots: 2,
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
        let max_shadowed_lights = self
            .max_shadowed_lights
            .min(min_slots)
            .min(entries(view_stride(limits).max(GPU_SHADOW_SIZE)).saturating_sub(1));
        Quality {
            max_visible_lights: self.max_visible_lights.min(entries(GPU_LIGHT_SIZE)),
            max_lights_per_cluster,
            cluster_budget,
            cluster_tile_px: self.cluster_tile_px.max(1),
            depth_slices: self.depth_slices.clamp(1, cluster_budget),
            shadow_atlas_size,
            // The slot buffers also hold the sun's.
            max_shadowed_lights,
            actor_shadow_slots: self.actor_shadow_slots.min(max_shadowed_lights + 1),
            shadow_min_slot_px,
            bloom_levels: self.bloom_levels.max(1),
            blur_taps: odd(blur_taps),
            ao_samples: self.ao_samples.min(ao::MAX_SAMPLES),
            ao_divisor: self.ao_divisor.clamp(1, ao::MAX_DIVISOR),
            ao_max_radius_texels: self.ao_max_radius_texels.max(1),
            ao_blur_taps: if self.ao_blur_taps < 7 { 5 } else { 9 },
            ..self.clone()
        }
    }
}

/// `taps`, or the odd count below it.
fn odd(taps: u32) -> u32 {
    if taps.is_multiple_of(2) {
        taps - 1
    } else {
        taps
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

    #[test]
    fn occlusion_settings_are_held_to_usable_values() {
        let limits = wgpu::Limits::default();
        let held = |samples, divisor, blur_taps, radius| {
            let q = Quality {
                ao_samples: samples,
                ao_divisor: divisor,
                ao_blur_taps: blur_taps,
                ao_max_radius_texels: radius,
                ..Quality::high()
            }
            .clamped_to(&limits);
            (
                q.ao_samples,
                q.ao_divisor,
                q.ao_blur_taps,
                q.ao_max_radius_texels,
            )
        };
        assert_eq!(
            held(u32::MAX, u32::MAX, u32::MAX, u32::MAX),
            (ao::MAX_SAMPLES, ao::MAX_DIVISOR, 9, u32::MAX)
        );
        assert_eq!(held(0, 0, 0, 0), (0, 1, 5, 1));
        // Only 5 and 9 taps cancel the pattern that turns the samples.
        for (asked, kept) in [(1, 5), (3, 5), (6, 5), (7, 9), (8, 9), (11, 9)] {
            assert_eq!(held(8, 4, asked, 32).2, kept, "{asked} taps");
        }
    }
}
