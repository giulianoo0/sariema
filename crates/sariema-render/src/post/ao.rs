// SPDX-License-Identifier: Apache-2.0

//! Screen-space ambient occlusion from the resolved view depth: estimated at
//! 1/`Quality::ao_divisor` of the target's size, blurred along each axis
//! without crossing silhouettes, then multiplied into the scene by the
//! composite.
//!
//! The composite darkens direct light in a crease along with ambient light.
//! Darkening the ambient term alone would need the depth before shading, a
//! depth pre-pass that tile-based phone GPUs pay for with a second geometry pass.

use bytemuck::{Pod, Zeroable};
use sariema_core::Camera;
use wgpu::BindingResource;

use super::{Pass, buffer_entry, group, layout, mip, pipeline, texture, texture_entry};
use crate::{Gpu, Quality};

pub(crate) const MAX_SAMPLES: u32 = 32;
pub(crate) const MAX_DIVISOR: u32 = 8;
/// The fraction of ambient light left, 1 where nothing occludes, and the
/// view depth it is for, which the blur and the composite compare.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// Mirrors `Params` in ao.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParamsUniform {
    scale: [f32; 2],
    radius: f32,
    far: f32,
}

pub(super) struct Occlusion {
    estimate: wgpu::RenderPipeline,
    /// Horizontal, then vertical.
    blur: [wgpu::RenderPipeline; 2],
    estimate_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    divisor: u32,
}

impl Occlusion {
    /// `None` when `quality` turns the occlusion off. `module` builds a
    /// full-screen shader from its fragment source.
    pub fn new(
        gpu: &Gpu,
        quality: &Quality,
        module: impl Fn(&str, &str) -> wgpu::ShaderModule,
    ) -> Option<Occlusion> {
        if quality.ao_samples == 0 {
            return None;
        }
        let estimate = module("ao", include_str!("../../shaders/ao.wgsl"));
        let blur = module(
            "ao blur",
            &[
                include_str!("../../shaders/ao_blur.wgsl"),
                include_str!("../../shaders/ao_surface.wgsl"),
            ]
            .concat(),
        );
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let estimate_layout = layout(
            gpu,
            "ao",
            &[
                texture_entry(0, unfiltered, false),
                buffer_entry(1, wgpu::BufferBindingType::Uniform),
            ],
        );
        let blur_layout = layout(gpu, "ao blur", &[texture_entry(0, unfiltered, false)]);
        let blur_pipeline = |vertical: bool| {
            let constants = [
                ("VERTICAL", vertical as u8 as f64),
                ("TAPS", quality.ao_blur_taps as f64),
            ];
            pipeline(gpu, &blur_layout, &blur, "fs", &constants, FORMAT)
        };
        let constants = [
            ("AO_DIVISOR", quality.ao_divisor as f64),
            ("SAMPLES", quality.ao_samples as f64),
            ("MAX_RADIUS_TEXELS", quality.ao_max_radius_texels as f64),
        ];
        Some(Occlusion {
            estimate: pipeline(gpu, &estimate_layout, &estimate, "fs", &constants, FORMAT),
            blur: [blur_pipeline(false), blur_pipeline(true)],
            params: gpu.buffer(
                "ao",
                size_of::<ParamsUniform>() as u64,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            ),
            estimate_layout,
            blur_layout,
            divisor: quality.ao_divisor,
        })
    }

    /// The passes that estimate and blur the occlusion of the full-size
    /// `view_depth`, and the texture they leave it in.
    pub fn passes(
        &self,
        gpu: &Gpu,
        view_depth: &wgpu::TextureView,
    ) -> (Vec<Pass>, wgpu::TextureView) {
        let full = view_depth.texture().size();
        let size = (
            full.width.div_ceil(self.divisor),
            full.height.div_ceil(self.divisor),
        );
        let target = |label| mip(&texture(gpu, label, FORMAT, size, 1), 0);
        let (occlusion, scratch) = (target("ao"), target("ao blur scratch"));
        let pass = |pipeline: &wgpu::RenderPipeline,
                    layout,
                    resources: Vec<_>,
                    target: &wgpu::TextureView| {
            Pass {
                pipeline: pipeline.clone(),
                group: group(gpu, layout, resources),
                target: target.clone(),
            }
        };
        let blur = |pipeline, source, target| {
            let resources = vec![BindingResource::TextureView(source)];
            pass(pipeline, &self.blur_layout, resources, target)
        };
        let estimate = vec![
            BindingResource::TextureView(view_depth),
            self.params.as_entire_binding(),
        ];
        let passes = vec![
            pass(&self.estimate, &self.estimate_layout, estimate, &occlusion),
            blur(&self.blur[0], &occlusion, &scratch),
            blur(&self.blur[1], &scratch, &occlusion),
        ];
        (passes, occlusion)
    }

    /// Uploads what the passes need from the frame's `camera` and the look's radius.
    pub fn prepare(&self, gpu: &Gpu, camera: &Camera, radius_m: f32) {
        let proj = camera.proj;
        let params = ParamsUniform {
            scale: [proj.x_axis.x, proj.y_axis.y],
            radius: radius_m,
            // Where a depth of 1, the cleared background, lands; see resolve_depth.wgsl.
            far: proj.w_axis.z / (1.0 + proj.z_axis.z),
        };
        gpu.queue
            .write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }
}
