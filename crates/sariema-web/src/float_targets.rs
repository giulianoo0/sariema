// SPDX-License-Identifier: Apache-2.0

//! The renderer draws its scene, view depth and occlusion into float
//! textures. WebGL2 renders into those only with `EXT_color_buffer_float`;
//! without it the engine refuses the device rather than draw black.

use sariema_render::FLOAT_TARGETS;

/// The renderer's float formats that `usages` says cannot be drawn into.
pub fn unrenderable(
    usages: impl Fn(wgpu::TextureFormat) -> wgpu::TextureUsages,
) -> Vec<wgpu::TextureFormat> {
    (FLOAT_TARGETS.into_iter())
        .filter(|&f| !usages(f).contains(wgpu::TextureUsages::RENDER_ATTACHMENT))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_without_float_targets_is_named_by_format() {
        let sampled_only = |_| wgpu::TextureUsages::TEXTURE_BINDING;
        assert_eq!(unrenderable(sampled_only), FLOAT_TARGETS);
        let half_floats = |f: wgpu::TextureFormat| match f {
            wgpu::TextureFormat::R32Float => wgpu::TextureUsages::TEXTURE_BINDING,
            _ => wgpu::TextureUsages::RENDER_ATTACHMENT,
        };
        assert_eq!(unrenderable(half_floats), [wgpu::TextureFormat::R32Float]);
        assert!(unrenderable(|_| wgpu::TextureUsages::all()).is_empty());
    }
}
