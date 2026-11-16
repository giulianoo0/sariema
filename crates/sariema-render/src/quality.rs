// SPDX-License-Identifier: Apache-2.0

use crate::cluster::{MAX_CLUSTER_LIGHTS, MAX_INDEX_ENTRIES};
use crate::lights::GPU_LIGHT_SIZE;
use crate::scene::{DEPTH_FORMAT, HDR_FORMAT};

/// Renderer settings that trade cost for coverage. Every field is a default
/// a game may override; [`crate::Renderer::new`] clamps them to the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quality {
    /// Local lights shaded per frame; past it, the lowest priority ones are dropped.
    pub max_visible_lights: u32,
    pub max_lights_per_cluster: u32,
    /// Most clusters in the grid; the tile grows until the grid fits.
    pub cluster_budget: u32,
    pub cluster_tile_px: u32,
    pub depth_slices: u32,
    /// Samples per pixel of the scene target.
    pub msaa: u32,
    pub shadow_atlas_size: u32,
    pub max_shadowed_lights: u32,
    pub shadow_pcf_taps: u32,
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
            max_shadowed_lights: 4,
            shadow_pcf_taps: 1,
        }
    }

    pub fn medium() -> Quality {
        Quality {
            max_visible_lights: 256,
            max_lights_per_cluster: 32,
            cluster_budget: 3072,
            msaa: 4,
            shadow_atlas_size: 2048,
            max_shadowed_lights: 12,
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
            max_shadowed_lights: 32,
            shadow_pcf_taps: 16,
            ..Quality::medium()
        }
    }

    /// These settings reduced to what `device` and the cluster packing can hold.
    pub(crate) fn clamped(&self, device: &wgpu::Device) -> Quality {
        let limits = device.limits();
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
        Quality {
            max_visible_lights: self.max_visible_lights.min(entries(GPU_LIGHT_SIZE)),
            max_lights_per_cluster,
            cluster_budget,
            cluster_tile_px: self.cluster_tile_px.max(1),
            depth_slices: self.depth_slices.clamp(1, cluster_budget),
            msaa: supported_samples(device, self.msaa),
            shadow_atlas_size: self.shadow_atlas_size.min(limits.max_texture_dimension_2d),
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
