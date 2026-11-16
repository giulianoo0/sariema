// SPDX-License-Identifier: Apache-2.0

//! The frame drawn last, kept to show over the frames after it: a dissolve
//! in screen space, where each frame is seen by its own camera and lit by
//! its own room. Keeping it composites the last frame's scene again, with
//! that frame's look, before anything of the new frame is drawn; showing it
//! is one blended full-screen pass over the composite. The blend mixes the
//! colour as the target stores it: linear on an sRGB target, encoded on the
//! others.

use bytemuck::{Pod, Zeroable};
use wgpu::BindingResource;

use super::{buffer_entry, group, layout, texture, texture_entry};
use crate::Gpu;

/// Mirrors `Params` in held.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParamsUniform {
    opacity: f32,
    _pad: [f32; 3],
}

pub(crate) struct Held {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    format: wgpu::TextureFormat,
    /// The frame held and the group that reads it, once one is.
    frame: Option<(wgpu::TextureView, wgpu::BindGroup)>,
    pub opacity: f32,
}

impl Held {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Held {
        let layout = layout(
            gpu,
            "held",
            &[
                texture_entry(
                    0,
                    wgpu::TextureSampleType::Float { filterable: false },
                    false,
                ),
                buffer_entry(1, wgpu::BufferBindingType::Uniform),
            ],
        );
        let module = gpu.shader(
            "held",
            [
                include_str!("../../shaders/fullscreen.wgsl"),
                include_str!("../../shaders/held.wgsl"),
            ]
            .concat(),
        );
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("held"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let keep_alpha = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = gpu.pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("held"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: keep_alpha,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let params = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("held"),
            size: size_of::<ParamsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Held {
            layout,
            pipeline,
            params,
            format,
            frame: None,
            opacity: 0.0,
        }
    }

    /// The texture to hold a frame of `size` in, made on first use.
    pub fn target(&mut self, gpu: &Gpu, size: (u32, u32)) -> &wgpu::TextureView {
        let (view, _) = self.frame.get_or_insert_with(|| {
            let view =
                texture(gpu, "held frame", self.format, size, 1).create_view(&Default::default());
            let read = group(
                gpu,
                &self.layout,
                [
                    BindingResource::TextureView(&view),
                    self.params.as_entire_binding(),
                ],
            );
            (view, read)
        });
        view
    }

    pub fn set_opacity(&mut self, gpu: &Gpu, opacity: f32) {
        if opacity != self.opacity {
            self.opacity = opacity;
            let params = ParamsUniform {
                opacity,
                _pad: [0.0; 3],
            };
            gpu.write(&self.params, bytemuck::bytes_of(&params));
        }
    }

    /// Forgets the frame held, which no longer fits the target.
    pub fn resize(&mut self) {
        self.frame = None;
    }

    /// Draws the frame held over `target`; false when none is.
    pub fn draw(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) -> bool {
        let Some((_, read)) = self.frame.as_ref().filter(|_| self.opacity > 0.0) else {
            return false;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("held"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes: gpu.pass("held"),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, read, &[]);
        pass.draw(0..3, 0..1);
        gpu.count_draws(1);
        true
    }
}
