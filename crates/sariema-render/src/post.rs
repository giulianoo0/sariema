// SPDX-License-Identifier: Apache-2.0

//! Post-processing. The scene is reduced to 1/2 and 1/4 of its size; the
//! bloom blurs the light past its threshold from 1/4 down, and the miniature
//! blur blurs the 1/2 and 1/4 levels, small levels in one pass and the others
//! in two. The miniature blur and the ambient occlusion share one resolve of
//! the depth. One pass then composites, tone maps, grades and dithers into
//! the target. An effect the look turns off draws none of its passes.

use std::cell::Cell;

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use sariema_core::Camera;
use sariema_format::Look;
use wgpu::BindingResource;

use crate::scene::{HDR_FORMAT, Scene};
use crate::{Gpu, Quality, RenderPath};

pub(crate) mod ao;
mod held;

pub(crate) const VIEW_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
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
    bloom_levels: u32,
    _pad: [u32; 3],
}

struct Layouts {
    resolve: wgpu::BindGroupLayout,
    downsample: wgpu::BindGroupLayout,
    blur: wgpu::BindGroupLayout,
    composite: wgpu::BindGroupLayout,
}

/// A full-screen draw into `target`.
struct Pass {
    label: &'static str,
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
    params: CompositeUniform,
    /// The composite's uniform and bloom of the frame drawn last, to
    /// composite it again into the frame held.
    drawn: Option<(CompositeUniform, bool)>,
    held: held::Held,
}

/// What does not depend on the target's size.
struct Stages {
    layouts: Layouts,
    resolve: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    /// Both axes in one pass.
    blur: wgpu::RenderPipeline,
    /// Both axes in one pass, of the level above reduced.
    reduce_blur: wgpu::RenderPipeline,
    /// Horizontal, then vertical.
    axes: [wgpu::RenderPipeline; 2],
    /// The horizontal blur keeping only the light past the bloom threshold.
    bright_blur: wgpu::RenderPipeline,
    blur_one_pass_max_texels: u32,
    composite: wgpu::RenderPipeline,
    /// `None` where the quality turns the occlusion off.
    occlusion: Option<ao::Occlusion>,
    sampler: wgpu::Sampler,
    /// Reads the scene's depth on the downlevel path.
    nearest: Option<wgpu::Sampler>,
    /// Bound as the bloom while it is off, and as the occlusion where the
    /// quality turns it off.
    black: wgpu::TextureView,
    projection: wgpu::Buffer,
    bright: wgpu::Buffer,
    params: wgpu::Buffer,
    /// The uniform of the frame drawn last, while it is composited again.
    held_params: wgpu::Buffer,
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
    /// `scratch[k]` is at `sizes[k]`, 1/2^(k+1) of the size: a two-pass
    /// blur's first pass, or a one-pass blur's source or target.
    scratch: Vec<wgpu::TextureView>,
    sizes: Vec<(u32, u32)>,
}

/// The passes over the current size's textures, by the effect they serve.
struct Frame {
    /// The scene's depth to view depth, for the miniature blur and the occlusion.
    resolve: Pass,
    occlusion: Vec<Pass>,
    /// The scene to 1/2, then 1/2 to 1/4; the bloom and the miniature blur start there.
    reduce: Vec<Pass>,
    bloom: Vec<Pass>,
    /// The 1/2 and 1/4 levels blurred, in place or into `scratch[0]` and
    /// `scratch[1]`.
    tilt: Vec<Pass>,
    composite: wgpu::BindGroup,
    composite_without_bloom: wgpu::BindGroup,
    /// The composite's groups with the uniform of the frame drawn last.
    held: wgpu::BindGroup,
    held_without_bloom: wgpu::BindGroup,
    /// The bloom's mip count, which GLSL ES 3.0 cannot query.
    bloom_levels: u32,
}

impl Post {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, quality: &Quality, scene: &Scene) -> Post {
        let depth_samples = scene.depth().texture().sample_count();
        let stages = Stages::new(gpu, format, quality, depth_samples);
        Post {
            frame: stages.frame(gpu, scene),
            stages,
            tilt: false,
            bloom: false,
            ao: false,
            ao_radius_m: 0.0,
            uploaded_for: Cell::new(None),
            params: CompositeUniform::zeroed(),
            drawn: None,
            held: held::Held::new(gpu, format),
        }
    }

    /// Rebuilds the chain's textures at the size of the scene's.
    pub fn resize(&mut self, gpu: &Gpu, scene: &Scene) {
        self.frame = self.stages.frame(gpu, scene);
        self.drawn = None;
        self.held.resize();
        if self.params.bloom_levels != self.frame.bloom_levels {
            self.params.bloom_levels = self.frame.bloom_levels;
            gpu.write(&self.stages.params, bytemuck::bytes_of(&self.params));
        }
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
        self.params = CompositeUniform {
            exposure: look.exposure,
            bloom_strength: look.bloom_strength,
            real: look.real,
            night: look.night,
            tilt: look.tilt,
            focus: look.focus,
            srgb_target: self.stages.srgb_target as u32,
            ao_strength: if self.ao { look.ao_strength } else { 0.0 },
            bloom_levels: self.frame.bloom_levels,
            _pad: [0; 3],
        };
        gpu.write(&self.stages.bright, bytemuck::bytes_of(&bright));
        gpu.write(&self.stages.params, bytemuck::bytes_of(&self.params));
    }

    /// Whether the next frame reads the scene's depth.
    pub fn reads_depth(&self) -> bool {
        self.tilt || self.ao
    }

    /// Passes of a frame with every effect on.
    pub fn most_passes(&self) -> u32 {
        let f = &self.frame;
        let chains = [&f.occlusion, &f.reduce, &f.bloom, &f.tilt];
        // The depth resolve and the composite.
        2 + chains.iter().map(|c| c.len() as u32).sum::<u32>()
    }

    /// Records the passes the look needs, then the composite into `target`,
    /// for a frame seen by `camera`; returns how many passes that is.
    pub fn draw(
        &mut self,
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
            gpu.write(&self.stages.projection, bytemuck::bytes_of(&projection));
            if let Some(occlusion) = self.stages.occlusion.as_ref().filter(|_| self.ao) {
                occlusion.prepare(gpu, camera, self.ao_radius_m);
            }
        }
        let frame = &self.frame;
        // The bloom uses `scratch[1]` and reads the 1/4 level before the
        // miniature blur writes either.
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
                fullscreen(
                    gpu,
                    encoder,
                    pass.label,
                    &pass.pipeline,
                    &pass.group,
                    &pass.target,
                );
            }
            drawn += passes.len() as u32;
        }
        let group = match self.bloom {
            true => &frame.composite,
            false => &frame.composite_without_bloom,
        };
        let composite = &self.stages.composite;
        fullscreen(gpu, encoder, "composite", composite, group, target);
        self.drawn = Some((self.params, self.bloom));
        drawn + self.held.draw(gpu, encoder, target) as u32
    }

    /// Composites the frame drawn last again, with its own look, into the
    /// frame held; to record before anything of the next frame is drawn.
    /// Nothing when no frame was drawn at the current size.
    pub fn hold(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        size: (u32, u32),
    ) -> bool {
        let Some((params, bloom)) = self.drawn else {
            return false;
        };
        gpu.write(&self.stages.held_params, bytemuck::bytes_of(&params));
        let group = match bloom {
            true => &self.frame.held,
            false => &self.frame.held_without_bloom,
        };
        let target = self.held.target(gpu, size);
        fullscreen(gpu, encoder, "hold", &self.stages.composite, group, target);
        true
    }

    /// How much of the frame held each frame shows over itself, 0 to 1.
    pub fn set_held_opacity(&mut self, gpu: &Gpu, opacity: f32) {
        self.held.set_opacity(gpu, opacity.clamp(0.0, 1.0));
    }
}

impl Stages {
    /// `depth_samples` is the sample count of the scene depth the chain reads.
    fn new(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        quality: &Quality,
        depth_samples: u32,
    ) -> Stages {
        use wgpu::util::DeviceExt;
        let device = &gpu.device;
        let module = |label: &str, source: &str| {
            gpu.shader(
                label,
                [include_str!("../shaders/fullscreen.wgsl"), source].concat(),
            )
        };
        let multisampled = depth_samples > 1;
        // GLSL ES 3.0 cannot load from a depth texture, so the downlevel path
        // binds the scene's depth as a plain float texture. It reads it through
        // a nearest sampler: wgpu's GL backend leaves a depth texture's own
        // filter at the mipmapped linear default, which makes it incomplete,
        // reading 0, for a load without a sampler.
        let as_float = quality.path == RenderPath::Downlevel;
        let scene_depth = match (multisampled, as_float) {
            (true, _) => {
                "alias SceneDepth = texture_depth_multisampled_2d;
                fn load_depth(p: vec2<i32>, s: u32) -> f32 { return textureLoad(scene_depth, p, s); }\n"
            }
            (false, false) => {
                "alias SceneDepth = texture_depth_2d;
                fn load_depth(p: vec2<i32>, s: u32) -> f32 { return textureLoad(scene_depth, p, s); }\n"
            }
            (false, true) => {
                "alias SceneDepth = texture_2d<f32>;
                @group(0) @binding(2) var nearest: sampler;
                fn load_depth(p: vec2<i32>, s: u32) -> f32 {
                    let size = vec2<f32>(textureDimensions(scene_depth));
                    return textureSampleLevel(scene_depth, nearest, (vec2<f32>(p) + 0.5) / size, 0.0).r;
                }\n"
            }
        };
        let depth_type = if as_float {
            wgpu::TextureSampleType::Float { filterable: false }
        } else {
            wgpu::TextureSampleType::Depth
        };
        let resolve = module(
            "resolve depth",
            &[scene_depth, include_str!("../shaders/resolve_depth.wgsl")].concat(),
        );
        let downsample = module("downsample", include_str!("../shaders/downsample.wgsl"));
        let blur = module(
            "blur",
            &[
                &kernel_wgsl(quality.blur_taps),
                include_str!("../shaders/blur.wgsl"),
            ]
            .concat(),
        );
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
                    texture_entry(0, depth_type, multisampled),
                    buffer_entry(1, uniform),
                    fragment_entry(
                        2,
                        wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    ),
                ][..if as_float { 3 } else { 2 }],
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
                    buffer_entry(2, uniform),
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
        let blur_pipeline = |axes: u32, bright: bool, reduce: bool| {
            let constants = [
                ("AXES", axes as f64),
                ("BRIGHT", bright as u8 as f64),
                ("REDUCE", reduce as u8 as f64),
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
        let samples = [("SAMPLES", depth_samples as f64)];
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
            blur: blur_pipeline(0, false, false),
            reduce_blur: blur_pipeline(0, false, true),
            axes: [
                blur_pipeline(1, false, false),
                blur_pipeline(2, false, false),
            ],
            bright_blur: blur_pipeline(1, true, false),
            blur_one_pass_max_texels: quality.blur_one_pass_max_texels,
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
            nearest: as_float.then(|| {
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("scene depth"),
                    ..Default::default()
                })
            }),
            black: black.create_view(&Default::default()),
            projection: uniform_buffer("depth projection", size_of::<ProjectionUniform>()),
            bright: uniform_buffer("bright", size_of::<BrightUniform>()),
            params: uniform_buffer("composite", size_of::<CompositeUniform>()),
            held_params: uniform_buffer("held composite", size_of::<CompositeUniform>()),
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
        let mut tilt = Vec::new();
        let blurred = [&t.half, &t.quarter]
            .into_iter()
            .enumerate()
            .map(|(k, level)| {
                let scratch = &t.scratch[k];
                if self.one_pass(texels(t.sizes[k])) {
                    tilt.push(self.blur(gpu, "tilt", &self.blur, level, scratch));
                    scratch.clone()
                } else {
                    tilt.extend(self.blur_axes(gpu, "tilt", &self.axes[0], level, scratch, level));
                    level.clone()
                }
            });
        let blurred: Vec<wgpu::TextureView> = blurred.collect();
        Frame {
            resolve: self.pass(
                gpu,
                "resolve depth",
                &self.resolve,
                &self.layouts.resolve,
                [
                    BindingResource::TextureView(scene.depth()),
                    self.projection.as_entire_binding(),
                ]
                .into_iter()
                .chain(self.nearest.as_ref().map(BindingResource::Sampler)),
                &t.depth,
            ),
            occlusion,
            reduce: vec![
                self.reduce(gpu, scene.output(), &t.half),
                self.reduce(gpu, &t.half, &t.quarter),
            ],
            bloom: self.bloom_passes(gpu, &t),
            tilt,
            composite: self.composite_group(
                gpu,
                scene,
                &t.depth,
                &blurred,
                &bloom,
                &ambient,
                &self.params,
            ),
            composite_without_bloom: self.composite_group(
                gpu,
                scene,
                &t.depth,
                &blurred,
                &self.black,
                &ambient,
                &self.params,
            ),
            held: self.composite_group(
                gpu,
                scene,
                &t.depth,
                &blurred,
                &bloom,
                &ambient,
                &self.held_params,
            ),
            held_without_bloom: self.composite_group(
                gpu,
                scene,
                &t.depth,
                &blurred,
                &self.black,
                &ambient,
                &self.held_params,
            ),
            bloom_levels: t.bloom.mip_level_count(),
        }
    }

    fn one_pass(&self, texels: u64) -> bool {
        texels <= self.blur_one_pass_max_texels as u64
    }

    fn bloom_passes(&self, gpu: &Gpu, t: &Targets) -> Vec<Pass> {
        let level = |k| mip(&t.bloom, k);
        let mut passes = Vec::from(self.blur_axes(
            gpu,
            "bloom",
            &self.bright_blur,
            &t.quarter,
            &t.scratch[1],
            &level(0),
        ));
        for k in 1..t.bloom.mip_level_count() {
            let (above, target) = (level(k - 1), level(k));
            let k = k as usize + 1;
            let scratch = &t.scratch[k];
            if self.one_pass(4 * texels(t.sizes[k])) {
                passes.push(self.blur(gpu, "bloom", &self.reduce_blur, &above, &target));
            } else {
                passes.push(self.reduce(gpu, &above, &target));
                let first = &self.axes[0];
                passes.extend(self.blur_axes(gpu, "bloom", first, &target, scratch, &target));
            }
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
            "reduce",
            &self.downsample,
            &self.layouts.downsample,
            resources,
            target,
        )
    }

    /// `source` blurred into `target` through `scratch`, `horizontal` drawing
    /// the first pass.
    fn blur_axes(
        &self,
        gpu: &Gpu,
        label: &'static str,
        horizontal: &wgpu::RenderPipeline,
        source: &wgpu::TextureView,
        scratch: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) -> [Pass; 2] {
        [
            self.blur(gpu, label, horizontal, source, scratch),
            self.blur(gpu, label, &self.axes[1], scratch, target),
        ]
    }

    fn blur(
        &self,
        gpu: &Gpu,
        label: &'static str,
        pipeline: &wgpu::RenderPipeline,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
    ) -> Pass {
        let resources = [
            BindingResource::TextureView(source),
            BindingResource::Sampler(&self.sampler),
            self.bright.as_entire_binding(),
        ];
        self.pass(gpu, label, pipeline, &self.layouts.blur, resources, target)
    }

    /// `blurred` holds the blurred 1/2 and 1/4 levels.
    #[allow(clippy::too_many_arguments)]
    fn composite_group(
        &self,
        gpu: &Gpu,
        scene: &Scene,
        depth: &wgpu::TextureView,
        blurred: &[wgpu::TextureView],
        bloom: &wgpu::TextureView,
        ambient: &wgpu::TextureView,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        group(
            gpu,
            &self.layouts.composite,
            [
                BindingResource::TextureView(scene.output()),
                BindingResource::TextureView(depth),
                BindingResource::TextureView(&blurred[0]),
                BindingResource::TextureView(&blurred[1]),
                BindingResource::TextureView(bloom),
                BindingResource::Sampler(&self.sampler),
                params.as_entire_binding(),
                BindingResource::TextureView(ambient),
            ],
        )
    }

    fn pass<'a>(
        &self,
        gpu: &Gpu,
        label: &'static str,
        pipeline: &wgpu::RenderPipeline,
        layout: &wgpu::BindGroupLayout,
        resources: impl IntoIterator<Item = BindingResource<'a>>,
        target: &wgpu::TextureView,
    ) -> Pass {
        Pass {
            label,
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
        let sizes: Vec<(u32, u32)> = (0..=bloom.mip_level_count()).map(level).collect();
        Targets {
            depth: view("view depth", VIEW_DEPTH_FORMAT, full),
            half: view("post 1/2", HDR_FORMAT, level(0)),
            quarter: view("post 1/4", HDR_FORMAT, level(1)),
            scratch: (sizes.iter())
                .map(|&size| view("blur scratch", HDR_FORMAT, size))
                .collect(),
            sizes,
            bloom,
        }
    }
}

fn texels((width, height): (u32, u32)) -> u64 {
    width as u64 * height as u64
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
/// Gaussian; 5 taps give the reference kernel.
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

/// [`kernel`] as the WGSL constants `TAPS` and `TAP_COUNT`, so the blur's
/// loops have a fixed length and read no buffer.
fn kernel_wgsl(taps: u32) -> String {
    let k = kernel(taps);
    let entries: Vec<String> = (k.iter())
        .map(|[offset, weight]| format!("vec2<f32>({offset:?}, {weight:?})"))
        .collect();
    format!(
        "const TAP_COUNT = {}u;\nconst TAPS = array<vec2<f32>, TAP_COUNT>({});\n",
        k.len(),
        entries.join(", ")
    )
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
    gpu: &Gpu,
    encoder: &mut wgpu::CommandEncoder,
    label: &'static str,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
    target: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        timestamp_writes: gpu.pass(label),
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
    gpu.count_draws(1);
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
