// SPDX-License-Identifier: Apache-2.0

//! Post-processing. The scene is reduced to 1/2 and 1/4 of its size; the
//! bloom blurs the light past its threshold from 1/4 down, and the miniature
//! blur blurs the 1/2 and 1/4 levels. The miniature blur and the ambient
//! occlusion share one resolve of the depth. One pass then composites, tone
//! maps, grades and dithers into the target. An effect the look turns off
//! draws none of its passes.

use std::cell::Cell;

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use sariema_core::Camera;
use sariema_format::Look;
use wgpu::BindingResource;

use crate::scene::{HDR_FORMAT, Scene};
use crate::{Gpu, Quality};

pub(crate) mod ao;

const VIEW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Past 9 taps the kernel's outer pairs weigh under 0.02% each; a wider blur
/// is the next level's job.
pub(crate) const MAX_BLUR_TAPS: u32 = 9;

/// Mirrors `Projection` in resolve_depth.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ProjectionUniform {
    a: f32,
    b: f32,
    _pad: [f32; 2],
}

/// Mirrors `Bright` in blur.wgsl.
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
    ao_strength: f32,
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
    /// Whether the last look turned the miniature blur, the bloom and the
    /// occlusion on.
    tilt: bool,
    bloom: bool,
    ao: bool,
    ao_radius_m: f32,
    /// The projection the depth passes' uniforms were last written for.
    uploaded_for: Cell<Option<Mat4>>,
}

/// What does not depend on the target's size.
struct Stages {
    layouts: Layouts,
    resolve: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    /// Horizontal, then vertical.
    blur: [wgpu::RenderPipeline; 2],
    /// The horizontal blur keeping only the light past the bloom threshold.
    bright_blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    /// `None` where the quality turns the occlusion off.
    occlusion: Option<ao::Occlusion>,
    sampler: wgpu::Sampler,
    /// Bound as the bloom while it is off, and as the occlusion where the
    /// quality turns it off.
    black: wgpu::TextureView,
    kernel: wgpu::Buffer,
    projection: wgpu::Buffer,
    bright: wgpu::Buffer,
    params: wgpu::Buffer,
    srgb_target: bool,
    bloom_levels: u32,
}

/// The chain's textures at one target size.
struct Targets {
    /// View depth in metres.
    depth: wgpu::TextureView,
    half: wgpu::TextureView,
    quarter: wgpu::TextureView,
    /// Mip k is the bloom level at 1/2^(k+2) of the size.
    bloom: wgpu::Texture,
    /// `scratch[k]`, at 1/2^(k+1) of the size, holds a blur's first pass.
    scratch: Vec<wgpu::TextureView>,
}

/// The passes over the current size's textures, by the effect they serve.
struct Frame {
    /// The scene's depth to view depth, for the miniature blur and the occlusion.
    resolve: Pass,
    occlusion: Vec<Pass>,
    /// The scene to 1/2, then 1/2 to 1/4; the bloom and the miniature blur start there.
    reduce: Vec<Pass>,
    bloom: Vec<Pass>,
    /// The 1/2 and 1/4 levels blurred in place.
    tilt: Vec<Pass>,
    composite: wgpu::BindGroup,
    composite_without_bloom: wgpu::BindGroup,
}

impl Post {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, quality: &Quality, scene: &Scene) -> Post {
        let stages = Stages::new(gpu, format, quality);
        Post {
            frame: stages.frame(gpu, scene),
            stages,
            tilt: false,
            bloom: false,
            ao: false,
            ao_radius_m: 0.0,
            uploaded_for: Cell::new(None),
        }
    }

    /// Rebuilds the chain's textures at the size of the scene's.
    pub fn resize(&mut self, gpu: &Gpu, scene: &Scene) {
        self.frame = self.stages.frame(gpu, scene);
    }

    pub fn set_look(&mut self, gpu: &Gpu, look: &Look) {
        self.tilt = look.tilt > 0.0;
        self.bloom = look.bloom_strength > 0.0;
        let radius = look.ao_radius_m;
        self.ao = self.stages.occlusion.is_some()
            && look.ao_strength > 0.0
            && radius.is_finite()
            && radius > 0.0;
        self.ao_radius_m = radius;
        self.uploaded_for.set(None);
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
            ao_strength: if self.ao { look.ao_strength } else { 0.0 },
        };
        let queue = &gpu.queue;
        queue.write_buffer(&self.stages.bright, 0, bytemuck::bytes_of(&bright));
        queue.write_buffer(&self.stages.params, 0, bytemuck::bytes_of(&params));
    }

    /// Whether the next frame reads the scene's depth.
    pub fn reads_depth(&self) -> bool {
        self.tilt || self.ao
    }

    /// Records the passes the look needs, then the composite into `target`,
    /// for a frame seen by `camera`; returns how many passes that is.
    pub fn draw(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        camera: &Camera,
        target: &wgpu::TextureView,
    ) -> u32 {
        if self.reads_depth() && self.uploaded_for.get() != Some(camera.proj) {
            self.uploaded_for.set(Some(camera.proj));
            let projection = ProjectionUniform {
                a: camera.proj.z_axis.z,
                b: camera.proj.w_axis.z,
                _pad: [0.0; 2],
            };
            gpu.queue
                .write_buffer(&self.stages.projection, 0, bytemuck::bytes_of(&projection));
            if let Some(occlusion) = self.stages.occlusion.as_ref().filter(|_| self.ao) {
                occlusion.prepare(gpu, camera, self.ao_radius_m);
            }
        }
        let frame = &self.frame;
        // The bloom reads the 1/4 level before the miniature blur blurs it in place.
        let groups = [
            (self.reads_depth(), std::slice::from_ref(&frame.resolve)),
            (self.ao, &frame.occlusion),
            (self.tilt || self.bloom, &frame.reduce),
            (self.bloom, &frame.bloom),
            (self.tilt, &frame.tilt),
        ];
        let mut drawn = 1;
        for (_, passes) in groups.into_iter().filter(|(on, _)| *on) {
            for pass in passes {
                fullscreen(encoder, &pass.pipeline, &pass.group, &pass.target);
            }
            drawn += passes.len() as u32;
        }
        let group = match self.bloom {
            true => &frame.composite,
            false => &frame.composite_without_bloom,
        };
        fullscreen(encoder, &self.stages.composite, group, target);
        drawn
    }
}

impl Stages {
    fn new(gpu: &Gpu, format: wgpu::TextureFormat, quality: &Quality) -> Stages {
        use wgpu::util::DeviceExt;
        let device = &gpu.device;
        let module = |label: &str, source: &str| {
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
        let composite = module(
            "composite",
            &[
                include_str!("../shaders/composite.wgsl"),
                include_str!("../shaders/ao_surface.wgsl"),
            ]
            .concat(),
        );

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
                &[texture_entry(0, colour, false), sampler_entry(1)],
            ),
            blur: layout(
                gpu,
                "blur",
                &[
                    texture_entry(0, colour, false),
                    sampler_entry(1),
                    buffer_entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
                    buffer_entry(3, uniform),
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
                    texture_entry(4, colour, false),
                    sampler_entry(5),
                    buffer_entry(6, uniform),
                    texture_entry(
                        7,
                        wgpu::TextureSampleType::Float { filterable: false },
                        false,
                    ),
                ],
            ),
        };
        let blur_pipeline = |vertical: bool, bright: bool| {
            let constants = [
                ("VERTICAL", vertical as u8 as f64),
                ("BRIGHT", bright as u8 as f64),
            ];
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
        let black = device.create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("no bloom"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[0; 8],
        );
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
            blur: [blur_pipeline(false, false), blur_pipeline(true, false)],
            bright_blur: blur_pipeline(false, true),
            composite: pipeline(
                gpu,
                &layouts.composite,
                &composite,
                "fs",
                &[("AO_DIVISOR", quality.ao_divisor as f64)],
                format,
            ),
            occlusion: ao::Occlusion::new(gpu, quality, module),
            layouts,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("post"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            black: black.create_view(&Default::default()),
            kernel: gpu.buffer_with(
                "blur kernel",
                bytemuck::cast_slice(&kernel(quality.blur_taps)),
                wgpu::BufferUsages::STORAGE,
            ),
            projection: uniform_buffer("depth projection", size_of::<ProjectionUniform>()),
            bright: uniform_buffer("bright", size_of::<BrightUniform>()),
            params: uniform_buffer("composite", size_of::<CompositeUniform>()),
            srgb_target: format.is_srgb(),
            bloom_levels: quality.bloom_levels,
        }
    }

    fn frame(&self, gpu: &Gpu, scene: &Scene) -> Frame {
        let t = Targets::new(gpu, scene, self.bloom_levels);
        let (occlusion, ambient) = match &self.occlusion {
            Some(o) => o.passes(gpu, &t.depth),
            None => (Vec::new(), self.black.clone()),
        };
        let bloom = t.bloom.create_view(&Default::default());
        Frame {
            resolve: self.pass(
                gpu,
                &self.resolve,
                &self.layouts.resolve,
                [
                    BindingResource::TextureView(scene.depth()),
                    self.projection.as_entire_binding(),
                ],
                &t.depth,
            ),
            occlusion,
            reduce: vec![
                self.reduce(gpu, scene.output(), &t.half),
                self.reduce(gpu, &t.half, &t.quarter),
            ],
            bloom: self.bloom_passes(gpu, &t),
            tilt: [
                self.blur(gpu, &self.blur[0], &t.half, &t.scratch[0], &t.half),
                self.blur(gpu, &self.blur[0], &t.quarter, &t.scratch[1], &t.quarter),
            ]
            .into_iter()
            .flatten()
            .collect(),
            composite: self.composite_group(gpu, scene, &t, &bloom, &ambient),
            composite_without_bloom: self.composite_group(gpu, scene, &t, &self.black, &ambient),
        }
    }

    fn bloom_passes(&self, gpu: &Gpu, t: &Targets) -> Vec<Pass> {
        let level = |k| mip(&t.bloom, k);
        let mut passes =
            Vec::from(self.blur(gpu, &self.bright_blur, &t.quarter, &t.scratch[1], &level(0)));
        for k in 1..t.bloom.mip_level_count() {
            passes.push(self.reduce(gpu, &level(k - 1), &level(k)));
            let scratch = &t.scratch[k as usize + 1];
            passes.extend(self.blur(gpu, &self.blur[0], &level(k), scratch, &level(k)));
        }
        passes
    }

    fn reduce(&self, gpu: &Gpu, source: &wgpu::TextureView, target: &wgpu::TextureView) -> Pass {
        let resources = [
            BindingResource::TextureView(source),
            BindingResource::Sampler(&self.sampler),
        ];
        self.pass(
            gpu,
            &self.downsample,
            &self.layouts.downsample,
            resources,
            target,
        )
    }

    /// `source` blurred into `target` through `scratch`, `horizontal` drawing
    /// the first pass.
    fn blur(
        &self,
        gpu: &Gpu,
        horizontal: &wgpu::RenderPipeline,
        source: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) -> [Pass; 2] {
        let pass = |pipeline, source, target| {
            let resources = [
                BindingResource::TextureView(source),
                BindingResource::Sampler(&self.sampler),
                self.kernel.as_entire_binding(),
                self.bright.as_entire_binding(),
            ];
            self.pass(gpu, pipeline, &self.layouts.blur, resources, target)
        };
        [
            pass(horizontal, source, scratch),
            pass(&self.blur[1], scratch, target),
        ]
    }

    fn composite_group(
        &self,
        gpu: &Gpu,
        scene: &Scene,
        t: &Targets,
        bloom: &wgpu::TextureView,
        ambient: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        group(
            gpu,
            &self.layouts.composite,
            [
                BindingResource::TextureView(scene.output()),
                BindingResource::TextureView(&t.depth),
                BindingResource::TextureView(&t.half),
                BindingResource::TextureView(&t.quarter),
                BindingResource::TextureView(bloom),
                BindingResource::Sampler(&self.sampler),
                self.params.as_entire_binding(),
                BindingResource::TextureView(ambient),
            ],
        )
    }

    fn pass<'a>(
        &self,
        gpu: &Gpu,
        pipeline: &wgpu::RenderPipeline,
        layout: &wgpu::BindGroupLayout,
        resources: impl IntoIterator<Item = BindingResource<'a>>,
        target: &wgpu::TextureView,
    ) -> Pass {
        Pass {
            pipeline: pipeline.clone(),
            group: group(gpu, layout, resources),
            target: target.clone(),
        }
    }
}

impl Targets {
    fn new(gpu: &Gpu, scene: &Scene, bloom_levels: u32) -> Targets {
        let size = scene.output().texture().size();
        let full = (size.width, size.height);
        let half = |(w, h): (u32, u32)| ((w / 2).max(1), (h / 2).max(1));
        // Level k is 1/2^(k+1) of the full size.
        let level = |k: u32| (0..=k).fold(full, |size, _| half(size));
        let view = |label, format, size| mip(&texture(gpu, label, format, size, 1), 0);
        let bloom = texture(gpu, "bloom", HDR_FORMAT, level(1), bloom_levels);
        Targets {
            depth: view("view depth", VIEW_DEPTH_FORMAT, full),
            half: view("post 1/2", HDR_FORMAT, level(0)),
            quarter: view("post 1/4", HDR_FORMAT, level(1)),
            scratch: (0..=bloom.mip_level_count())
                .map(|k| view("blur scratch", HDR_FORMAT, level(k)))
                .collect(),
            bloom,
        }
    }
}

/// Binds `resources` to bindings 0, 1, 2…
fn group<'a>(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    resources: impl IntoIterator<Item = BindingResource<'a>>,
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
/// smallest at each end, r = taps − 1, as in Rákos's linear-sampling
/// Gaussian; 5 taps give the Origem game's kernel.
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
