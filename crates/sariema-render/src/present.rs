// SPDX-License-Identifier: Apache-2.0

//! Placeholder output step: the HDR scene to the target through the look's
//! exposure and a filmic curve. The post-processing chain replaces it.

use bytemuck::{Pod, Zeroable};

use crate::Gpu;

/// Mirrors `Params` in present.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    exposure: f32,
    srgb_target: u32,
    _pad: [u32; 2],
}

pub(crate) struct Present {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    srgb_target: bool,
    group: wgpu::BindGroup,
}

impl Present {
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat, scene: &wgpu::TextureView) -> Present {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("present"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/present.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = gpu.pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present"),
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
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let params = gpu.buffer(
            "present",
            size_of::<Params>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let group = bind_group(gpu, &layout, scene, &params);
        let present = Present {
            pipeline,
            layout,
            params,
            srgb_target: format.is_srgb(),
            group,
        };
        present.set_exposure(gpu, 1.0);
        present
    }

    /// Points at a new scene image after a resize.
    pub fn bind(&mut self, gpu: &Gpu, scene: &wgpu::TextureView) {
        self.group = bind_group(gpu, &self.layout, scene, &self.params);
    }

    pub fn set_exposure(&self, gpu: &Gpu, exposure: f32) {
        let params = Params {
            exposure,
            srgb_target: self.srgb_target as u32,
            _pad: [0; 2],
        };
        gpu.queue
            .write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    }

    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn bind_group(
    gpu: &Gpu,
    layout: &wgpu::BindGroupLayout,
    scene: &wgpu::TextureView,
    params: &wgpu::Buffer,
) -> wgpu::BindGroup {
    gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("present"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(scene),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params.as_entire_binding(),
            },
        ],
    })
}
