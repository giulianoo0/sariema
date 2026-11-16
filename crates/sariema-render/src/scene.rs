// SPDX-License-Identifier: Apache-2.0

//! The scene pass: background gradient, then opaque, blended and additive
//! draws into the multisampled HDR target, whose alpha is the "drawn" mask.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use sariema_core::Camera;

use crate::cluster::{self, Grid, Visible};
use crate::lights::{GPU_LIGHT_SIZE, GpuLight, gpu_light};
use crate::resources::{DrawUniform, Room};
use crate::shadow::{self, Assigned, Shadows};
use crate::{FrameStats, Gpu, Quality};

mod draw;

pub(crate) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Lights, cluster cells, light indices and shadow slots.
pub(crate) const STORAGE_BUFFERS: u32 = 4;

const SOURCE: &str = concat!(
    include_str!("../shaders/common.wgsl"),
    include_str!("../shaders/scene.wgsl")
);

/// Mirrors `Frame` in common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sh: [[f32; 4]; 9],
    background_bottom: [f32; 4],
    background_top: [f32; 4],
    grid: [u32; 4],
    slicing: [f32; 4],
}

/// One pipeline each; nothing depends on how many lights a room has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    LitOpaque,
    LitBlend,
    UnlitOpaque,
    UnlitBlend,
    Additive,
}

impl Kind {
    const ALL: [Kind; 5] = [
        Kind::LitOpaque,
        Kind::LitBlend,
        Kind::UnlitOpaque,
        Kind::UnlitBlend,
        Kind::Additive,
    ];

    pub fn of(material: &sariema_format::Material) -> Kind {
        use sariema_format::{AlphaMode, Shading};
        match (material.shading, material.alpha) {
            (Shading::Lit, AlphaMode::Opaque) => Kind::LitOpaque,
            (Shading::Lit, AlphaMode::Blend) => Kind::LitBlend,
            (Shading::Unlit, AlphaMode::Opaque) => Kind::UnlitOpaque,
            (Shading::Unlit, AlphaMode::Blend) => Kind::UnlitBlend,
            (Shading::Additive, _) => Kind::Additive,
        }
    }

    pub fn is_opaque(self) -> bool {
        matches!(self, Kind::LitOpaque | Kind::UnlitOpaque)
    }

    fn target(self) -> wgpu::ColorTargetState {
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let (blend, write_mask) = match self {
            Kind::LitOpaque | Kind::UnlitOpaque => (None, wgpu::ColorWrites::ALL),
            // The blend constant carries the material's mask value, so the
            // mask blends towards it by the surface's opacity.
            Kind::LitBlend | Kind::UnlitBlend => (
                Some(wgpu::BlendState {
                    color: premultiplied,
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Constant,
                        ..premultiplied
                    },
                }),
                wgpu::ColorWrites::ALL,
            ),
            // Glows add light over whatever is behind and keep its mask.
            Kind::Additive => (
                Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        dst_factor: wgpu::BlendFactor::One,
                        ..premultiplied
                    },
                    alpha: premultiplied,
                }),
                wgpu::ColorWrites::COLOR,
            ),
        };
        wgpu::ColorTargetState {
            format: HDR_FORMAT,
            blend,
            write_mask,
        }
    }
}

pub(crate) struct Layouts {
    pub frame: wgpu::BindGroupLayout,
    pub draw: wgpu::BindGroupLayout,
    pub material: wgpu::BindGroupLayout,
    /// Bound where a material has no texture.
    pub white: wgpu::TextureView,
}

struct Targets {
    /// `None` without multisampling, when the pass draws into `resolved`.
    multisampled: Option<wgpu::TextureView>,
    resolved: wgpu::TextureView,
    depth: wgpu::TextureView,
}

pub(crate) struct Scene {
    layouts: Layouts,
    pipelines: [wgpu::RenderPipeline; Kind::ALL.len()],
    background: wgpu::RenderPipeline,
    depth_only: wgpu::RenderPipeline,
    frame: wgpu::Buffer,
    lights: wgpu::Buffer,
    cells: wgpu::Buffer,
    indices: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    targets: Targets,
    grid: Grid,
    max_lights_per_cluster: u32,
}

impl Scene {
    /// Creates every scene pipeline and the per-frame buffers, sized for the
    /// worst case `quality` allows so frames never reallocate them.
    pub fn new(
        gpu: &Gpu,
        (width, height): (u32, u32),
        quality: &Quality,
        layouts: Layouts,
        shadows: &Shadows,
    ) -> Scene {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let pipeline_layout = |groups: &[&wgpu::BindGroupLayout]| {
            let groups: Vec<_> = groups.iter().copied().map(Some).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &groups,
                immediate_size: 0,
            })
        };
        let mesh_layout = pipeline_layout(&[&layouts.frame, &layouts.draw, &layouts.material]);
        let msaa = quality.msaa;
        let pipelines =
            Kind::ALL.map(|kind| mesh_pipeline(gpu, &mesh_layout, &module, kind, quality));
        let background = gpu.pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("background"),
            layout: Some(&pipeline_layout(&[&layouts.frame])),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_background"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
            multisample: wgpu::MultisampleState {
                count: msaa,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_background"),
                compilation_options: Default::default(),
                targets: &[Some(HDR_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let depth_only = gpu.pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth only"),
            layout: Some(&pipeline_layout(&[&layouts.frame, &layouts.draw])),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_depth"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout())],
            },
            primitive: primitive(),
            depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Less)),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });

        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let frame = gpu.buffer(
            "frame",
            size_of::<FrameUniform>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let lights = gpu.buffer(
            "lights",
            quality.max_visible_lights.max(1) as u64 * GPU_LIGHT_SIZE,
            storage,
        );
        let cells = gpu.buffer("cluster cells", quality.cluster_budget as u64 * 4, storage);
        let indices = gpu.buffer(
            "cluster light indices",
            quality.cluster_budget as u64 * quality.max_lights_per_cluster as u64 * 4,
            storage,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let entries = [
            entry(0, frame.as_entire_binding()),
            entry(1, lights.as_entire_binding()),
            entry(2, cells.as_entire_binding()),
            entry(3, indices.as_entire_binding()),
            entry(4, wgpu::BindingResource::Sampler(&sampler)),
        ];
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &layouts.frame,
            entries: &[entries.as_slice(), &shadows.group_entries()].concat(),
        });
        Scene {
            targets: targets(gpu, width, height, msaa),
            grid: Grid::new(width, height, quality),
            max_lights_per_cluster: quality.max_lights_per_cluster,
            layouts,
            pipelines,
            background,
            depth_only,
            frame,
            lights,
            cells,
            indices,
            frame_group,
        }
    }

    pub fn layouts(&self) -> &Layouts {
        &self.layouts
    }

    /// The resolved HDR image; alpha is the "drawn" mask.
    pub fn output(&self) -> &wgpu::TextureView {
        &self.targets.resolved
    }

    /// The scene's depth, with `Quality::msaa` samples per pixel.
    pub fn depth(&self) -> &wgpu::TextureView {
        &self.targets.depth
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32, quality: &Quality) {
        self.targets = targets(gpu, width, height, quality.msaa);
        self.grid = Grid::new(width, height, quality);
    }

    /// Bins the frame's `visible` lights, with their `shadows`, for `camera`
    /// and uploads them with the frame constants.
    pub fn prepare(
        &self,
        gpu: &Gpu,
        room: Option<&Room>,
        camera: &Camera,
        time_s: f32,
        visible: &Visible,
        shadows: &Assigned,
    ) -> FrameStats {
        let lights = room.map_or(&[][..], |r| &r.lights[..]);
        let packed: Vec<GpuLight> = (visible.lights.iter().zip(&shadows.lights))
            .map(|(&i, &slot)| gpu_light(&lights[i], i, time_s, slot))
            .collect();
        let (near, far) = cluster::light_depths(camera.proj, &visible.spheres);
        let grid = self.grid.sliced(near, far);
        let bins = cluster::bin(
            &grid,
            camera.proj,
            &visible.spheres,
            self.max_lights_per_cluster,
        );
        let queue = &gpu.queue;
        for (buffer, bytes) in [
            (&self.lights, bytemuck::cast_slice(&packed)),
            (&self.cells, bytemuck::cast_slice(&bins.cells)),
            (&self.indices, bytemuck::cast_slice(&bins.indices)),
        ] {
            if !bytes.is_empty() {
                queue.write_buffer(buffer, 0, bytes);
            }
        }
        queue.write_buffer(
            &self.frame,
            0,
            bytemuck::bytes_of(&frame_uniform(room, camera, &grid, shadows.sun)),
        );
        FrameStats {
            lights_in_frustum: visible.in_frustum,
            lights_dropped: visible.dropped,
            clusters: grid.len(),
            max_lights_overlapping_cluster: bins.max_overlapping,
            cluster_overflows: bins.overflows,
            shadows_skipped: shadows.skipped,
            post_passes: 0,
        }
    }
}

fn frame_uniform(
    room: Option<&Room>,
    camera: &Camera,
    grid: &Grid,
    sun_shadow: Option<u32>,
) -> FrameUniform {
    let mut sh = [[0.0; 4]; 9];
    let (mut sun_dir, mut sun_color) = ([0.0; 4], [0.0; 4]);
    let (mut bottom, mut top) = ([0.0; 4], [0.0; 4]);
    if let Some(room) = room {
        for (dst, src) in sh.iter_mut().zip(&room.ambient) {
            *dst = rgb_w(*src, 0.0);
        }
        if let Some(sun) = &room.sun {
            let towards = Vec3::from(sun.direction).normalize_or_zero();
            sun_dir = towards
                .extend(sun_shadow.map_or(-1.0, |slot| slot as f32))
                .into();
            sun_color = rgb_w(sun.color.map(|c| c * sun.intensity), 0.0);
        }
        bottom = rgb_w(room.background.bottom, 0.0);
        top = rgb_w(room.background.top, 0.0);
    }
    let (scale, bias) = grid.slicing();
    FrameUniform {
        view_proj: (camera.proj * camera.view).to_cols_array_2d(),
        view: camera.view.to_cols_array_2d(),
        camera_pos: camera.position.extend(1.0).into(),
        sun_dir,
        sun_color,
        sh,
        background_bottom: bottom,
        background_top: top,
        grid: [grid.tiles_x, grid.tiles_y, grid.slices, grid.tile_px],
        slicing: [scale, bias, 0.0, 0.0],
    }
}

fn rgb_w(rgb: [f32; 3], w: f32) -> [f32; 4] {
    [rgb[0], rgb[1], rgb[2], w]
}

pub(crate) fn layouts(gpu: &Gpu) -> Layouts {
    let device = &gpu.device;
    // `dynamic` is the binding size of a uniform bound with a dynamic offset.
    let uniform = |binding, visibility, dynamic: Option<u64>| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic.is_some(),
            min_binding_size: dynamic.and_then(wgpu::BufferSize::new),
        },
        count: None,
    };
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
    let layout = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries,
        })
    };
    Layouts {
        frame: layout(
            "frame",
            &[
                [
                    uniform(0, both, None),
                    storage(1),
                    storage(2),
                    storage(3),
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ]
                .as_slice(),
                &shadow::layout_entries(),
            ]
            .concat(),
        ),
        draw: layout(
            "draw",
            &[uniform(
                0,
                wgpu::ShaderStages::VERTEX,
                Some(size_of::<DrawUniform>() as u64),
            )],
        ),
        material: layout(
            "material",
            &[uniform(0, both, None), texture(1), texture(2)],
        ),
        white: white_texture(gpu),
    }
}

fn white_texture(gpu: &Gpu) -> wgpu::TextureView {
    use wgpu::util::DeviceExt;
    let texture = gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some("white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &[255; 4],
    );
    texture.create_view(&Default::default())
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

pub(crate) fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Unorm8x4];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<sariema_format::Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}

/// Counter-clockwise front faces with back faces culled, three.js's `FrontSide`.
fn primitive() -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        cull_mode: Some(wgpu::Face::Back),
        ..Default::default()
    }
}

pub(crate) fn depth_state(write: bool, compare: wgpu::CompareFunction) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias: Default::default(),
    }
}

fn mesh_pipeline(
    gpu: &Gpu,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    kind: Kind,
    quality: &Quality,
) -> wgpu::RenderPipeline {
    let lit = matches!(kind, Kind::LitOpaque | Kind::LitBlend);
    // The largest square grid within the tap count.
    let pcf_side = quality.shadow_pcf_taps.isqrt().max(1);
    let constants = [
        ("LIT", lit as u8 as f64),
        ("BLEND", !kind.is_opaque() as u8 as f64),
        ("PCF_SIDE", pcf_side as f64),
    ];
    let options = wgpu::PipelineCompilationOptions {
        constants: &constants,
        ..Default::default()
    };
    gpu.pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("mesh"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: options.clone(),
            buffers: &[Some(vertex_layout())],
        },
        primitive: primitive(),
        depth_stencil: Some(depth_state(kind.is_opaque(), wgpu::CompareFunction::Less)),
        multisample: wgpu::MultisampleState {
            count: quality.msaa,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            compilation_options: options,
            targets: &[Some(kind.target())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn targets(gpu: &Gpu, width: u32, height: u32, msaa: u32) -> Targets {
    let texture = |label, format, sample_count, usage| {
        gpu.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
    Targets {
        multisampled: (msaa > 1).then(|| texture("scene msaa", HDR_FORMAT, msaa, attachment)),
        resolved: texture(
            "scene",
            HDR_FORMAT,
            1,
            attachment | wgpu::TextureUsages::TEXTURE_BINDING,
        ),
        depth: texture(
            "scene depth",
            DEPTH_FORMAT,
            msaa,
            attachment | wgpu::TextureUsages::TEXTURE_BINDING,
        ),
    }
}
