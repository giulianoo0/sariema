// SPDX-License-Identifier: Apache-2.0

//! The scene pass: background gradient, then opaque, blended and additive
//! draws into the multisampled HDR target, whose alpha is the "drawn" mask.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use sariema_core::Camera;
use sariema_format::{AlphaMode, CharacterLight, Fog, Light, Shading};

use crate::cluster::{self, Bins, Grid, Visible};
use crate::lights::{GPU_LIGHT_SIZE, GpuLight, gpu_light};
use crate::resources::{CutUniform, DrawUniform};
use crate::shadow::{self, Assigned, Shadows};
use crate::table::{self, Table, Texel};
use crate::{FrameStats, Gpu, Lighting, Quality, RenderPath, Stage};

mod draw;

pub(crate) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Lights, cluster cells, light indices and shadow slots.
pub(crate) const STORAGE_BUFFERS: u32 = 4;

/// common.wgsl and scene.wgsl with the per-frame arrays of `quality`'s path.
pub(crate) fn source(quality: &Quality) -> String {
    let tables = match quality.path {
        RenderPath::Downlevel => format!(
            "const TABLE_WIDTH: u32 = {}u;\n{}",
            quality.data_texture_width,
            include_str!("../shaders/tables_textures.wgsl")
        ),
        _ => include_str!("../shaders/tables_buffers.wgsl").to_string(),
    };
    [
        include_str!("../shaders/common.wgsl"),
        include_str!("../shaders/scene.wgsl"),
        include_str!("../shaders/fog.wgsl"),
        include_str!("../shaders/cut.wgsl"),
        &tables,
    ]
    .concat()
}

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
    character_dir: [f32; 4],
    character_color: [f32; 4],
    character: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
}

/// What picks a draw's pipeline. There is one pipeline per kind; nothing
/// depends on how many lights a room has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kind {
    pub lit: bool,
    pub alpha: AlphaMode,
    pub double_sided: bool,
}

impl Kind {
    pub const COUNT: usize = 12;
    const ALPHAS: [AlphaMode; 3] = [AlphaMode::Opaque, AlphaMode::Blend, AlphaMode::Additive];

    pub fn of(material: &sariema_format::Material) -> Kind {
        Kind {
            lit: material.shading == Shading::Lit,
            alpha: material.alpha,
            double_sided: material.double_sided,
        }
    }

    /// Where its pipeline is in `Scene::pipelines`; [`Kind::from_index`] inverts it.
    pub fn index(self) -> usize {
        let alpha = Kind::ALPHAS.iter().position(|&a| a == self.alpha).unwrap();
        (self.lit as usize * Kind::ALPHAS.len() + alpha) * 2 + self.double_sided as usize
    }

    fn from_index(i: usize) -> Kind {
        Kind {
            lit: i / (2 * Kind::ALPHAS.len()) == 1,
            alpha: Kind::ALPHAS[i / 2 % Kind::ALPHAS.len()],
            double_sided: i % 2 == 1,
        }
    }

    pub fn is_opaque(self) -> bool {
        self.alpha == AlphaMode::Opaque
    }

    /// Where an opaque kind's dithering pipeline is in `Scene::dithered`.
    pub fn dither_index(self) -> usize {
        self.lit as usize * 2 + self.double_sided as usize
    }

    fn target(self) -> wgpu::ColorTargetState {
        let premultiplied = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let (blend, write_mask) = match self.alpha {
            AlphaMode::Opaque => (None, wgpu::ColorWrites::ALL),
            // The blend constant carries the material's mask value, so the
            // mask blends towards it by the surface's opacity.
            AlphaMode::Blend => (
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
            AlphaMode::Additive => (
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
    /// The downlevel path's single-sample depth of the opaque draws, drawn
    /// in a pass of its own where the scene is multisampled: OpenGL ES 3.0
    /// cannot sample a multisampled texture.
    single_depth: Option<wgpu::TextureView>,
}

/// The pipelines of draws in cutaways: by [`Kind::index`], and their
/// depth alone by `double_sided as usize`; then those drawing again the
/// back faces of a draw under the cut, pulled towards the camera so the cap
/// they show wins over a floor they lie on.
pub(crate) struct CutPipelines {
    pub color: [wgpu::RenderPipeline; Kind::COUNT],
    pub depth: [wgpu::RenderPipeline; 2],
    pub backs: [wgpu::RenderPipeline; Kind::COUNT],
    pub depth_backs: wgpu::RenderPipeline,
}

pub(crate) struct Scene {
    layouts: Layouts,
    /// Indexed by [`Kind::index`].
    pipelines: [wgpu::RenderPipeline; Kind::COUNT],
    /// Opaque kinds that dither, indexed by [`Kind::dither_index`].
    dithered: [wgpu::RenderPipeline; 4],
    background: wgpu::RenderPipeline,
    /// Indexed by `double_sided as usize`.
    depth_only: [wgpu::RenderPipeline; 2],
    /// Made when the first room with cutaways is uploaded.
    cut: Option<CutPipelines>,
    module: wgpu::ShaderModule,
    mesh_layout: wgpu::PipelineLayout,
    depth_layout: wgpu::PipelineLayout,
    frame: wgpu::Buffer,
    lights: Table,
    cells: Table,
    indices: Table,
    frame_group: wgpu::BindGroup,
    targets: Targets,
    grid: Grid,
    max_lights_per_cluster: u32,
    frustum_culling: bool,
    packed: Vec<GpuLight>,
    bins: Bins,
    sent: Uploads,
    /// The drawn rooms' draws in view, as [`Scene::order`] sorted them:
    /// layer, distance, room slot and draw.
    order: Vec<(u8, f32, u32, u32)>,
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
        let module = gpu.shader("scene", source(quality));
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
        let pipelines = std::array::from_fn(|i| {
            let vertex = ("vs_main", vertex_layout());
            let kind = Kind::from_index(i);
            mesh_pipeline(
                gpu,
                &mesh_layout,
                &module,
                kind,
                (false, false),
                quality,
                vertex,
            )
        });
        let dithered = std::array::from_fn(|i| {
            let vertex = ("vs_main", vertex_layout());
            let kind = Kind {
                lit: i / 2 == 1,
                alpha: AlphaMode::Opaque,
                double_sided: i % 2 == 1,
            };
            mesh_pipeline(
                gpu,
                &mesh_layout,
                &module,
                kind,
                (true, false),
                quality,
                vertex,
            )
        });
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
        let depth_layout = pipeline_layout(&[&layouts.frame, &layouts.draw]);
        let depth_only = [false, true].map(|double_sided| {
            gpu.pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("depth only"),
                layout: Some(&depth_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_depth"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout())],
                },
                primitive: primitive(double_sided),
                depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Less)),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        });

        let frame = gpu.buffer(
            "frame",
            size_of::<FrameUniform>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let table = |label, capacity, texel| {
            let width = quality.data_texture_width;
            Table::new(gpu, label, quality.path, capacity, texel, width)
        };
        let lights = table(
            "lights",
            quality.max_visible_lights.max(1) as u64 * GPU_LIGHT_SIZE,
            Texel::Four,
        );
        let cells = table(
            "cluster cells",
            quality.cluster_budget as u64 * 4,
            Texel::One,
        );
        let indices = table(
            "cluster light indices",
            quality.cluster_budget as u64 * quality.max_lights_per_cluster as u64 * 4,
            Texel::One,
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
            entry(1, lights.binding()),
            entry(2, cells.binding()),
            entry(3, indices.binding()),
            entry(4, wgpu::BindingResource::Sampler(&sampler)),
        ];
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &layouts.frame,
            entries: &[entries.as_slice(), &shadows.group_entries()].concat(),
        });
        Scene {
            targets: targets(gpu, width, height, quality),
            grid: Grid::new(width, height, quality),
            max_lights_per_cluster: quality.max_lights_per_cluster,
            frustum_culling: quality.frustum_culling,
            packed: Vec::new(),
            bins: Bins::default(),
            sent: Uploads::default(),
            order: Vec::new(),
            layouts,
            pipelines,
            dithered,
            background,
            depth_only,
            cut: None,
            module,
            mesh_layout,
            depth_layout,
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

    /// The depth the post-processing reads: the scene's, with
    /// `Quality::msaa` samples per pixel, or the downlevel path's single-sample one.
    pub fn depth(&self) -> &wgpu::TextureView {
        let t = &self.targets;
        t.single_depth.as_ref().unwrap_or(&t.depth)
    }

    /// Passes of their own that draw [`Scene::depth`] when the post-processing reads it.
    pub fn depth_passes(&self) -> u32 {
        self.targets.single_depth.is_some() as u32
    }

    /// Makes the pipelines of draws in cutaways, once.
    pub fn prepare_cutaways(&mut self, gpu: &Gpu, quality: &Quality) {
        if self.cut.is_some() {
            return;
        }
        let shaded = |backs: bool| {
            std::array::from_fn(|i| {
                let kind = Kind::from_index(i);
                let shading = (kind.is_opaque(), false, "fs_cut", backs);
                let vertex = ("vs_main", vertex_layout());
                shaded_pipeline(
                    gpu,
                    &self.mesh_layout,
                    &self.module,
                    kind,
                    shading,
                    quality,
                    vertex,
                )
            })
        };
        let buffers = [Some(vertex_layout())];
        let depth = |double_sided: bool| wgpu::RenderPipelineDescriptor {
            label: Some("depth only, cut"),
            layout: Some(&self.depth_layout),
            vertex: wgpu::VertexState {
                module: &self.module,
                entry_point: Some("vs_depth_cut"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: primitive(double_sided),
            depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Less)),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &self.module,
                entry_point: Some("fs_depth_cut"),
                compilation_options: Default::default(),
                targets: &[],
            }),
            multiview_mask: None,
            cache: None,
        };
        self.cut = Some(CutPipelines {
            color: shaded(false),
            depth: [false, true].map(|double_sided| gpu.pipeline(&depth(double_sided))),
            backs: shaded(true),
            depth_backs: {
                let mut backs = depth(true);
                only_backs(&mut backs.primitive, backs.depth_stencil.as_mut().unwrap());
                gpu.pipeline(&backs)
            },
        });
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32, quality: &Quality) {
        self.targets = targets(gpu, width, height, quality);
        self.grid = Grid::new(width, height, quality);
    }

    /// Orders the drawn rooms' draws in view of `camera`; returns how many
    /// visible draws were left out as outside it.
    pub fn cull(&mut self, stage: &Stage, camera: &Camera) -> u32 {
        self.order(stage, camera)
    }

    /// Bins the frame's `visible` lights, with their `shadows`, for `camera`
    /// and uploads what changed since the last frame.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        lights: &[Light],
        lighting: Option<&Lighting>,
        camera: &Camera,
        time_s: f32,
        visible: &Visible,
        shadows: &Assigned,
    ) -> FrameStats {
        self.packed.clear();
        self.packed.extend(
            (visible.lights.iter().zip(&shadows.lights))
                .map(|(&i, &slot)| gpu_light(&lights[i], i, time_s, slot)),
        );
        let (near, far) = cluster::light_depths(camera.proj, &visible.spheres);
        let grid = self.grid.sliced(near, far);
        self.bins.fill(
            &grid,
            camera.proj,
            &visible.spheres,
            self.max_lights_per_cluster,
        );
        let frame = frame_uniform(lighting, camera, &grid, shadows.sun);
        let sent = &mut self.sent;
        let tables = [
            (
                &mut sent.lights,
                &self.lights,
                bytemuck::cast_slice(&self.packed),
            ),
            (
                &mut sent.cells,
                &self.cells,
                bytemuck::cast_slice(&self.bins.cells),
            ),
            (
                &mut sent.indices,
                &self.indices,
                bytemuck::cast_slice(&self.bins.indices),
            ),
        ];
        for (sent, table, bytes) in tables {
            if sent.changed(bytes) {
                table.write(gpu, bytes);
            }
        }
        if sent.frame.changed(bytemuck::bytes_of(&frame)) {
            gpu.write(&self.frame, bytemuck::bytes_of(&frame));
        }
        FrameStats {
            lights_in_frustum: visible.in_frustum,
            lights_dropped: visible.dropped,
            clusters: grid.len(),
            max_lights_overlapping_cluster: self.bins.max_overlapping,
            cluster_overflows: self.bins.overflows,
            shadows_skipped: shadows.skipped,
            ..FrameStats::default()
        }
    }
}

/// The bytes last written to a buffer, so that a frame writes it only when
/// they change: each write stages a copy that costs more than the comparison.
#[derive(Default)]
struct Sent(Vec<u8>);

impl Sent {
    /// Whether `bytes`, if any, differ from those last given; it keeps them.
    fn changed(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() || self.0 == bytes {
            return false;
        }
        self.0.clear();
        self.0.extend_from_slice(bytes);
        true
    }
}

#[derive(Default)]
struct Uploads {
    lights: Sent,
    cells: Sent,
    indices: Sent,
    frame: Sent,
}

fn frame_uniform(
    lighting: Option<&Lighting>,
    camera: &Camera,
    grid: &Grid,
    sun_shadow: Option<u32>,
) -> FrameUniform {
    let mut sh = [[0.0; 4]; 9];
    let (mut sun_dir, mut sun_color) = ([0.0; 4], [0.0; 4]);
    let (mut bottom, mut top) = ([0.0; 4], [0.0; 4]);
    let off = CharacterLight::default();
    let character = lighting.map_or(&off, |l| &l.character);
    let towards = (camera.view.inverse())
        .transform_vector3(Vec3::from(character.direction))
        .normalize_or(Vec3::Z);
    if let Some(lighting) = lighting {
        for (dst, src) in sh.iter_mut().zip(&lighting.ambient) {
            *dst = rgb_w(*src, 0.0);
        }
        if let Some(sun) = &lighting.sun {
            let towards = Vec3::from(sun.direction).normalize_or_zero();
            sun_dir = towards
                .extend(sun_shadow.map_or(-1.0, |slot| slot as f32))
                .into();
            sun_color = rgb_w(sun.color.map(|c| c * sun.intensity), 0.0);
        }
        bottom = rgb_w(lighting.background.bottom, 0.0);
        top = rgb_w(lighting.background.top, 0.0);
    }
    let fog = lighting.map_or(Fog::default(), |l| l.fog);
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
        character_dir: towards.extend(character.wrap).into(),
        character_color: rgb_w(character.color, character.intensity),
        character: [
            character.rim,
            character.rim_power,
            character.min_brightness,
            character.room_tint,
        ],
        fog_color: rgb_w(fog.color, 0.0),
        fog: fog_params(&fog),
    }
}

/// `fog` as fog.wgsl's `fog_share` takes it, with 1 in w while there is fog.
pub(crate) fn fog_params(fog: &Fog) -> [f32; 4] {
    let ramp = if fog.end > fog.start {
        (1.0 / (fog.end - fog.start)).min(f32::MAX)
    } else {
        0.0
    };
    let on = fog.density > 0.0 || ramp > 0.0;
    [fog.start, ramp, fog.density, on as u8 as f32]
}

fn rgb_w(rgb: [f32; 3], w: f32) -> [f32; 4] {
    [rgb[0], rgb[1], rgb[2], w]
}

pub(crate) fn layouts(gpu: &Gpu, path: RenderPath) -> Layouts {
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
                    table::layout_entry(1, path),
                    table::layout_entry(2, path),
                    table::layout_entry(3, path),
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ]
                .as_slice(),
                &shadow::layout_entries(path),
            ]
            .concat(),
        ),
        draw: layout(
            "draw",
            &[
                uniform(0, both, Some(size_of::<DrawUniform>() as u64)),
                uniform(
                    2,
                    wgpu::ShaderStages::FRAGMENT,
                    Some(size_of::<CutUniform>() as u64),
                ),
            ],
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

/// Counter-clockwise front faces with back faces culled, three.js's `FrontSide`,
/// or kept when `double_sided`, its `DoubleSide`.
fn primitive(double_sided: bool) -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        cull_mode: (!double_sided).then_some(wgpu::Face::Back),
        ..Default::default()
    }
}

/// Back faces alone, pulled towards the camera past what is coplanar with them.
fn only_backs(primitive: &mut wgpu::PrimitiveState, depth: &mut wgpu::DepthStencilState) {
    primitive.cull_mode = Some(wgpu::Face::Front);
    depth.bias = wgpu::DepthBiasState {
        constant: -4,
        slope_scale: -1.0,
        clamp: 0.0,
    };
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

/// The pipeline drawing `kind` with fs_main, its vertices read by the entry
/// point and buffer layout of `vertex`; `dither` for a dissolving room's
/// opaque draws, `actor` for actors, which take the character light.
pub(crate) fn mesh_pipeline(
    gpu: &Gpu,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    kind: Kind,
    (dither, actor): (bool, bool),
    quality: &Quality,
    vertex: (&str, wgpu::VertexBufferLayout<'_>),
) -> wgpu::RenderPipeline {
    let shading = (dither, actor, "fs_main", false);
    shaded_pipeline(gpu, layout, module, kind, shading, quality, vertex)
}

fn shaded_pipeline(
    gpu: &Gpu,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    kind: Kind,
    (dither, actor, fragment, backs): (bool, bool, &str, bool),
    quality: &Quality,
    (entry_point, buffer): (&str, wgpu::VertexBufferLayout<'_>),
) -> wgpu::RenderPipeline {
    let mut primitive = primitive(kind.double_sided);
    let mut depth_stencil = depth_state(kind.is_opaque(), wgpu::CompareFunction::Less);
    if backs {
        only_backs(&mut primitive, &mut depth_stencil);
    }
    // The largest square grid within the tap count.
    let pcf_side = quality.shadow_pcf_taps.isqrt().max(1);
    let constants = [
        ("LIT", kind.lit as u8 as f64),
        ("BLEND", !kind.is_opaque() as u8 as f64),
        ("PCF_SIDE", pcf_side as f64),
        ("DITHER", dither as u8 as f64),
        ("ACTOR", actor as u8 as f64),
        ("GLOW", (kind.alpha == AlphaMode::Additive) as u8 as f64),
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
            entry_point: Some(entry_point),
            compilation_options: options.clone(),
            buffers: &[Some(buffer)],
        },
        primitive,
        depth_stencil: Some(depth_stencil),
        multisample: wgpu::MultisampleState {
            count: quality.msaa,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: options,
            targets: &[Some(kind.target())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn targets(gpu: &Gpu, width: u32, height: u32, quality: &Quality) -> Targets {
    let msaa = quality.msaa;
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
    let sampled = attachment | wgpu::TextureUsages::TEXTURE_BINDING;
    let single_depth = (quality.single_depth_pass())
        .then(|| texture("scene depth, one sample", DEPTH_FORMAT, 1, sampled));
    let depth_usage = match single_depth {
        Some(_) => attachment,
        None => sampled,
    };
    Targets {
        multisampled: (msaa > 1).then(|| texture("scene msaa", HDR_FORMAT, msaa, attachment)),
        resolved: texture("scene", HDR_FORMAT, 1, sampled),
        depth: texture("scene depth", DEPTH_FORMAT, msaa, depth_usage),
        single_depth,
    }
}
