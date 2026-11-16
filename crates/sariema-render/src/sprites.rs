// SPDX-License-Identifier: Apache-2.0

//! Particles and ground dots in one instanced draw of the scene pass, after
//! the translucent actors, every sprite reading the same small array of
//! built-in shapes. No compute and nothing WebGL2 lacks: the host gives the
//! sprites each frame, already simulated.

use bytemuck::{Pod, Zeroable};
use sariema_core::Camera;

use crate::scene::{HDR_FORMAT, Layouts, depth_state};
use crate::{Gpu, Quality};

/// A sprite as the host gives it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Sprite {
    pub position: [f32; 3],
    /// Metres across.
    pub size: f32,
    /// Linear RGB, above 1 to bloom, and opacity.
    pub color: [f32; 4],
    /// Radians about the axis it faces along.
    pub rotation: f32,
    /// 1 adds its light to what is behind, 0 covers it.
    pub additive: f32,
    pub shape: u32,
    /// [`Sprite::FLAT`] lies on the ground facing up; otherwise it faces the
    /// camera.
    pub flags: u32,
}

impl Sprite {
    pub const FLAT: u32 = 1;
}

/// The built-in shapes, by [`Sprite::shape`]: a soft round spot, a
/// four-pointed star, a disc with a soft rim and a spot stretched along x.
pub const SHAPES: u32 = 4;
const SHAPE_SIDE: u32 = 64;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SpriteFrame {
    scale: [f32; 4],
}

pub(crate) struct Sprites {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    instances: wgpu::Buffer,
    capacity: usize,
    min_px: f32,
    /// What the host gave for the next frame.
    staged: Vec<Sprite>,
    /// Sprites the next draw shows, and those past the capacity left out.
    count: u32,
    pub dropped: u32,
}

impl Sprites {
    pub fn new(gpu: &Gpu, layouts: &Layouts, quality: &Quality) -> Sprites {
        let device = &gpu.device;
        let module = gpu.shader(
            "sprite",
            [
                include_str!("../shaders/common.wgsl"),
                include_str!("../shaders/sprite.wgsl"),
            ]
            .concat(),
        );
        let entry = |binding, ty, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprites"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    wgpu::ShaderStages::VERTEX,
                ),
                entry(
                    1,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    wgpu::ShaderStages::FRAGMENT,
                ),
                entry(
                    2,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    wgpu::ShaderStages::FRAGMENT,
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprites"),
            bind_group_layouts: &[Some(&layouts.frame), Some(&layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32, 2 => Float32x4, 3 => Float32,
            4 => Float32, 5 => Uint32, 6 => Uint32,
        ];
        let blend = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = gpu.pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprites"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_sprite"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Sprite>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Less)),
            multisample: wgpu::MultisampleState {
                count: quality.msaa,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_sprite"),
                compilation_options: Default::default(),
                // The mask in alpha is left as the room drew it.
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: blend,
                        alpha: blend,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let capacity = quality.max_sprites.max(1) as usize;
        let instances = gpu.buffer(
            "sprites",
            (capacity * size_of::<Sprite>()) as u64,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        );
        let uniform = gpu.buffer(
            "sprite frame",
            size_of::<SpriteFrame>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let shapes = shape_texture(gpu);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprite shapes"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprites"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shapes.create_view(
                        &wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            ..Default::default()
                        },
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Sprites {
            pipeline,
            group,
            uniform,
            instances,
            capacity,
            min_px: quality.sprite_min_px as f32,
            staged: Vec::with_capacity(capacity),
            count: 0,
            dropped: 0,
        }
    }

    /// Keeps `sprites` for the next frame; past the capacity they are left
    /// out and counted.
    pub fn stage(&mut self, sprites: &[Sprite]) {
        let kept = sprites.len().min(self.capacity);
        self.staged.clear();
        self.staged.extend_from_slice(&sprites[..kept]);
        self.dropped = (sprites.len() - kept) as u32;
    }

    /// Uploads the staged sprites and the camera's scale for a target
    /// `height` pixels high.
    pub fn prepare(&mut self, gpu: &Gpu, camera: &Camera, height: u32) {
        self.count = self.staged.len() as u32;
        if self.count == 0 {
            return;
        }
        gpu.write(&self.instances, bytemuck::cast_slice(&self.staged));
        let frame = SpriteFrame {
            scale: [
                camera.proj.y_axis.y * height as f32 * 0.5,
                self.min_px,
                0.0,
                0.0,
            ],
        };
        gpu.write(&self.uniform, bytemuck::bytes_of(&frame));
    }

    /// Draws the sprites into a pass whose frame group is bound; the draws
    /// recorded.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) -> u32 {
        if self.count == 0 {
            return 0;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(1, &self.group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..4, 0..self.count);
        1
    }

    pub fn count(&self) -> u32 {
        self.count
    }
}

/// Coverage of shape `shape` at `p`, each coordinate in −1 to 1; 0 along
/// the edge, so neither the clamp nor the smaller mips bleed.
fn coverage(shape: u32, [x, y]: [f32; 2]) -> f32 {
    let r2 = x * x + y * y;
    let fade = (1.0 - r2).max(0.0);
    let value = match shape {
        0 => (-4.0 * r2).exp(),
        1 => {
            let core = (-30.0 * r2).exp();
            let ray = |along: f32, across: f32| {
                (-(across / 0.05).powi(2)).exp() * (1.0 - along.abs()).powi(3)
            };
            core + 0.85 * (ray(x, y) + ray(y, x))
        }
        2 => {
            let t = ((r2.sqrt() - 0.6) / 0.35).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        }
        _ => (-(4.0 * x * x + 60.0 * y * y)).exp(),
    };
    (value * fade).clamp(0.0, 1.0)
}

fn shape_texture(gpu: &Gpu) -> wgpu::Texture {
    let levels = SHAPE_SIDE.ilog2() + 1;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sprite shapes"),
        size: wgpu::Extent3d {
            width: SHAPE_SIDE,
            height: SHAPE_SIDE,
            depth_or_array_layers: SHAPES,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for shape in 0..SHAPES {
        let side = SHAPE_SIDE as usize;
        let mut level: Vec<f32> = (0..side * side)
            .map(|i| {
                let at = |k: usize| (k as f32 + 0.5) / side as f32 * 2.0 - 1.0;
                coverage(shape, [at(i % side), at(i / side)])
            })
            .collect();
        for mip in 0..levels {
            let s = (SHAPE_SIDE >> mip).max(1);
            let texels: Vec<u8> = level.iter().map(|v| (v * 255.0).round() as u8).collect();
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: shape,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(s),
                    rows_per_image: Some(s),
                },
                wgpu::Extent3d {
                    width: s,
                    height: s,
                    depth_or_array_layers: 1,
                },
            );
            let half = (s / 2).max(1) as usize;
            let s = s as usize;
            level = (0..half * half)
                .map(|i| {
                    let (x, y) = (i % half * 2, i / half * 2);
                    let at =
                        |dx: usize, dy: usize| level[(y + dy).min(s - 1) * s + (x + dx).min(s - 1)];
                    (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) * 0.25
                })
                .collect();
        }
    }
    texture
}
