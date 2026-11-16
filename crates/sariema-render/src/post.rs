// SPDX-License-Identifier: Apache-2.0

//! Post-processing: the scene's depth resolved to view depth, a chain of
//! blurred half-size levels feeding the miniature blur and the bloom, and one
//! pass that composites, tone maps, grades and dithers into the target.

use bytemuck::{Pod, Zeroable};
use sariema_core::Camera;
use sariema_format::Look;

use crate::scene::{HDR_FORMAT, Scene};
use crate::{Gpu, Quality};

const VIEW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// The miniature blur mixes in the chain's 1/2 and 1/4 levels.
const CHAIN_LEVELS: u32 = 2;

/// Mirrors `Projection` in resolve_depth.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ProjectionUniform {
    a: f32,
    b: f32,
    _pad: [f32; 2],
}

/// Mirrors `Bright` in downsample.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BrightUniform {
    tint: [f32; 3],
    threshold: f32,
}

/// Mirrors `Params` in composite.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CompositeUniform {
    exposure: f32,
    bloom_strength: f32,
    real: f32,
    night: f32,
    tilt: f32,
    focus: f32,
    srgb_target: u32,
    _pad: u32,
}

struct Layouts {
    resolve: wgpu::BindGroupLayout,
    downsample: wgpu::BindGroupLayout,
    blur: wgpu::BindGroupLayout,
    composite: wgpu::BindGroupLayout,
}

/// A full-screen draw into `target`.
struct Pass {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    target: wgpu::TextureView,
}

pub(crate) struct Post {
    stages: Stages,
    frame: Frame,
}

/// What does not depend on the target's size.
struct Stages {
    layouts: Layouts,
    resolve: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    bright: wgpu::RenderPipeline,
    /// Horizontal, then vertical.
    blur: [wgpu::RenderPipeline; 2],
    composite: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    kernel: wgpu::Buffer,
    projection: wgpu::Buffer,
    bright_params: wgpu::Buffer,
    params: wgpu::Buffer,
    srgb_target: bool,
    bloom_levels: u32,
}

/// The chain's textures at the target's size, bound for each pass.
struct Frame {
    /// Every pass before the composite, in order.
    passes: Vec<Pass>,
    composite: wgpu::BindGroup,
}

impl Post {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, quality: &Quality, scene: &Scene) -> Post {
        let stages = Stages::new(gpu, format, quality);
        Post {
            frame: stages.frame(gpu, scene),
            stages,
        }
    }

    /// Rebuilds the chain's textures at the size of the scene's.
    pub fn resize(&mut self, gpu: &Gpu, scene: &Scene) {
        self.frame = self.stages.frame(gpu, scene);
    }

    pub fn set_look(&self, gpu: &Gpu, look: &Look) {
        let bright = BrightUniform {
            tint: look.tint,
            threshold: look.bloom_threshold,
        };
        let params = CompositeUniform {
            exposure: look.exposure,
            bloom_strength: look.bloom_strength,
            real: look.real,
            night: look.night,
            tilt: look.tilt,
            focus: look.focus,
            srgb_target: self.stages.srgb_target as u32,
            _pad: 0,
        };
        let queue = &gpu.queue;
        queue.write_buffer(&self.stages.bright_params, 0, bytemuck::bytes_of(&bright));
        queue.write_buffer(&self.stages.params, 0, bytemuck::bytes_of(&params));
    }

    /// Records the chain, and the composite into `target`, for a frame seen
    /// by `camera`.
    pub fn draw(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        camera: &Camera,
        target: &wgpu::TextureView,
    ) {
        let projection = ProjectionUniform {
            a: camera.proj.z_axis.z,
            b: camera.proj.w_axis.z,
            _pad: [0.0; 2],
        };
        gpu.queue
            .write_buffer(&self.stages.projection, 0, bytemuck::bytes_of(&projection));
        for pass in &self.frame.passes {
            fullscreen(encoder, &pass.pipeline, &pass.group, &pass.target);
        }
        let composite = &self.stages.composite;
        fullscreen(encoder, composite, &self.frame.composite, target);
    }
}

impl Stages {
    fn new(gpu: &Gpu, format: wgpu::TextureFormat, quality: &Quality) -> Stages {
        let device = &gpu.device;
        let module = |label, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(
                    [include_str!("../shaders/fullscreen.wgsl"), source]
                        .concat()
                        .into(),
                ),
            })
        };
        let multisampled = quality.msaa > 1;
        let scene_depth = if multisampled {
            "alias SceneDepth = texture_depth_multisampled_2d;\n"
        } else {
            "alias SceneDepth = texture_depth_2d;\n"
        };
        let resolve = module(
            "resolve depth",
            &[scene_depth, include_str!("../shaders/resolve_depth.wgsl")].concat(),
        );
        let downsample = module("downsample", include_str!("../shaders/downsample.wgsl"));
        let blur = module("blur", include_str!("../shaders/blur.wgsl"));
        let composite = module("composite", include_str!("../shaders/composite.wgsl"));

        let colour = wgpu::TextureSampleType::Float { filterable: true };
        let uniform = wgpu::BufferBindingType::Uniform;
        let layouts = Layouts {
            resolve: layout(
                gpu,
                "resolve depth",
                &[
                    texture_entry(0, wgpu::TextureSampleType::Depth, multisampled),
                    buffer_entry(1, uniform),
                ],
            ),
            downsample: layout(
                gpu,
                "downsample",
                &[
                    texture_entry(0, colour, false),
                    sampler_entry(1),
                    buffer_entry(2, uniform),
                ],
            ),
            blur: layout(
                gpu,
                "blur",
                &[
                    texture_entry(0, colour, false),
                    sampler_entry(1),
                    buffer_entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
                ],
            ),
            composite: layout(
                gpu,
                "composite",
                &[
                    texture_entry(0, colour, false),
                    texture_entry(
                        1,
                        wgpu::TextureSampleType::Float { filterable: false },
                        false,
                    ),
                    texture_entry(2, colour, false),
                    texture_entry(3, colour, false),
                    sampler_entry(4),
                    buffer_entry(5, uniform),
                ],
            ),
        };
        let blur_pipeline = |vertical: bool| {
            let constants = [("VERTICAL", vertical as u8 as f64)];
            pipeline(gpu, &layouts.blur, &blur, "fs", &constants, HDR_FORMAT)
        };
        let uniform_buffer = |label, size: usize| {
            gpu.buffer(
                label,
                size as u64,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            )
        };
        let samples = [("SAMPLES", quality.msaa as f64)];
        Stages {
            resolve: pipeline(
                gpu,
                &layouts.resolve,
                &resolve,
                "fs",
                &samples,
                VIEW_DEPTH_FORMAT,
            ),
            downsample: pipeline(gpu, &layouts.downsample, &downsample, "fs", &[], HDR_FORMAT),
            bright: pipeline(
                gpu,
                &layouts.downsample,
                &downsample,
                "fs_bright",
                &[],
                HDR_FORMAT,
            ),
            blur: [blur_pipeline(false), blur_pipeline(true)],
            composite: pipeline(gpu, &layouts.composite, &composite, "fs", &[], format),
            layouts,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("post"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            kernel: gpu.buffer_with(
                "blur kernel",
                bytemuck::cast_slice(&kernel(quality.blur_taps)),
                wgpu::BufferUsages::STORAGE,
            ),
            projection: uniform_buffer("depth projection", size_of::<ProjectionUniform>()),
            bright_params: uniform_buffer("bright", size_of::<BrightUniform>()),
            params: uniform_buffer("composite", size_of::<CompositeUniform>()),
            srgb_target: format.is_srgb(),
            bloom_levels: quality.bloom_levels,
        }
    }

    /// The chain's textures at the size of the scene's, and the passes over them.
    fn frame(&self, gpu: &Gpu, scene: &Scene) -> Frame {
        let full = scene.output().texture().size();
        let full = (full.width, full.height);
        let half = |(w, h): (u32, u32)| ((w / 2).max(1), (h / 2).max(1));
        // Level k is 1/2^(k+1) of the full size.
        let level = |k: u32| (0..=k).fold(full, |size, _| half(size));
        let view_depth = mip(&texture(gpu, "view depth", VIEW_DEPTH_FORMAT, full, 1), 0);
        let chain = texture(gpu, "post chain", HDR_FORMAT, level(0), CHAIN_LEVELS);
        let bloom = texture(gpu, "bloom", HDR_FORMAT, level(1), self.bloom_levels);
        let scratch: Vec<wgpu::TextureView> =
            (0..chain.mip_level_count().max(bloom.mip_level_count() + 1))
                .map(|k| mip(&texture(gpu, "blur scratch", HDR_FORMAT, level(k), 1), 0))
                .collect();

        let resolve = group(
            gpu,
            &self.layouts.resolve,
            [
                wgpu::BindingResource::TextureView(scene.depth()),
                self.projection.as_entire_binding(),
            ],
        );
        let mut passes = vec![Pass {
            pipeline: self.resolve.clone(),
            group: resolve,
            target: view_depth.clone(),
        }];
        // `first` reduces `source` into `target`, then two passes blur it there.
        let mut push_level = |first: &wgpu::RenderPipeline,
                              source: &wgpu::TextureView,
                              target: wgpu::TextureView,
                              scratch: &wgpu::TextureView| {
            let sampled = |layout, source, extra| {
                group(
                    gpu,
                    layout,
                    [
                        wgpu::BindingResource::TextureView(source),
                        wgpu::BindingResource::Sampler(&self.sampler),
                        extra,
                    ],
                )
            };
            let blur =
                |source| sampled(&self.layouts.blur, source, self.kernel.as_entire_binding());
            let reduce = sampled(
                &self.layouts.downsample,
                source,
                self.bright_params.as_entire_binding(),
            );
            passes.push(Pass {
                pipeline: first.clone(),
                group: reduce,
                target: target.clone(),
            });
            passes.push(Pass {
                pipeline: self.blur[0].clone(),
                group: blur(&target),
                target: scratch.clone(),
            });
            passes.push(Pass {
                pipeline: self.blur[1].clone(),
                group: blur(scratch),
                target,
            });
        };
        for k in 0..chain.mip_level_count() {
            let source = match k {
                0 => scene.output().clone(),
                _ => mip(&chain, k - 1),
            };
            push_level(
                &self.downsample,
                &source,
                mip(&chain, k),
                &scratch[k as usize],
            );
        }
        for k in 0..bloom.mip_level_count() {
            let (first, source) = match k {
                0 => (&self.bright, mip(&chain, 0)),
                _ => (&self.downsample, mip(&bloom, k - 1)),
            };
            push_level(first, &source, mip(&bloom, k), &scratch[k as usize + 1]);
        }
        let whole = |t: &wgpu::Texture| t.create_view(&Default::default());
        let composite = group(
            gpu,
            &self.layouts.composite,
            [
                wgpu::BindingResource::TextureView(scene.output()),
                wgpu::BindingResource::TextureView(&view_depth),
                wgpu::BindingResource::TextureView(&whole(&chain)),
                wgpu::BindingResource::TextureView(&whole(&bloom)),
                wgpu::BindingResource::Sampler(&self.sampler),
                self.params.as_entire_binding(),
            ],
        );
        Frame { passes, composite }
    }
}

/// Binds `resources` to bindings 0, 1, 2…
fn group<'a>(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    resources: impl IntoIterator<Item = wgpu::BindingResource<'a>>,
) -> wgpu::BindGroup {
    let entries: Vec<wgpu::BindGroupEntry> = resources
        .into_iter()
        .enumerate()
        .map(|(binding, resource)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource,
        })
        .collect();
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("post"),
        layout,
        entries: &entries,
    })
}

/// The centre and one side of a blur of `taps` bilinear samples, as (offset
/// in texels, weight). The weights are binomial, of row 2r + 4 less the two
/// smallest at each end, r = taps − 1, as in Rákos's linear-sampling Gaussian
/// (rastergrid.com, 2010); 5 taps give the Origem game's kernel.
fn kernel(taps: u32) -> Vec<[f32; 2]> {
    let radius = taps.max(1) as usize - 1;
    let middle = (radius + 2) as f64;
    // C(2r + 4, r + 2 + j) / C(2r + 4, r + 2)
    let mut texels = vec![1.0_f64];
    for j in 1..=radius {
        texels.push(texels[j - 1] * (middle - j as f64 + 1.0) / (middle + j as f64));
    }
    let total = texels[0] + 2.0 * texels[1..].iter().sum::<f64>();
    let mut kernel = vec![[0.0, (texels[0] / total) as f32]];
    for i in 1..=radius / 2 {
        let (near, far) = (texels[2 * i - 1], texels[2 * i]);
        let offset = ((2 * i - 1) as f64 * near + (2 * i) as f64 * far) / (near + far);
        kernel.push([offset as f32, ((near + far) / total) as f32]);
    }
    kernel
}

fn texture(
    gpu: &Gpu,
    label: &str,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    mips: u32,
) -> wgpu::Texture {
    let most_mips = u32::BITS - width.max(height).leading_zeros();
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips.clamp(1, most_mips),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn mip(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

fn layout(gpu: &Gpu, label: &str, entries: &[wgpu::BindGroupLayoutEntry]) -> wgpu::BindGroupLayout {
    gpu.device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries,
        })
}

fn fragment_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    }
}

fn texture_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    multisampled: bool,
) -> wgpu::BindGroupLayoutEntry {
    fragment_entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled,
        },
    )
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    fragment_entry(
        binding,
        wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
    )
}

fn buffer_entry(binding: u32, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    fragment_entry(
        binding,
        wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}

fn pipeline(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    module: &wgpu::ShaderModule,
    entry_point: &str,
    constants: &[(&str, f64)],
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
    let compilation_options = wgpu::PipelineCompilationOptions {
        constants,
        ..Default::default()
    };
    gpu.pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry_point),
            compilation_options,
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn fullscreen(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
    target: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("post"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_taps_are_the_classic_linear_sampled_gaussian() {
        let expected = [
            [0.0, 0.227_027_03],
            [1.384_615_4, 0.316_216_22],
            [3.230_769_2, 0.070_270_27],
        ];
        let got = kernel(5);
        assert_eq!(got.len(), expected.len());
        for (g, e) in got.iter().zip(expected) {
            assert!(
                (g[0] - e[0]).abs() < 1e-5 && (g[1] - e[1]).abs() < 1e-6,
                "{got:?}"
            );
        }
    }

    #[test]
    fn kernels_sum_to_one() {
        for taps in [1, 3, 5, 9, 31] {
            let k = kernel(taps);
            let sum = k[0][1] + 2.0 * k[1..].iter().map(|t| t[1]).sum::<f32>();
            assert!((sum - 1.0).abs() < 1e-5, "{taps} taps sum to {sum}");
        }
    }
}
