// SPDX-License-Identifier: Apache-2.0

//! The actors' pipelines, created when the first pack is loaded: one per
//! material kind its parts use, the shadow caster and the disc shadow.

use sariema_format::actor::SkinnedVertex;

use crate::scene::{HDR_FORMAT, Kind, Layouts, depth_state, mesh_pipeline};
use crate::{Gpu, Quality, RenderError};

const SCENE_SOURCE: &str = concat!(
    include_str!("../../shaders/common.wgsl"),
    include_str!("../../shaders/scene.wgsl"),
    include_str!("../../shaders/skin.wgsl"),
    include_str!("../../shaders/actor.wgsl")
);
const SHADOW_SOURCE: &str = concat!(
    include_str!("../../shaders/common.wgsl"),
    include_str!("../../shaders/skin.wgsl"),
    include_str!("../../shaders/shadow.wgsl"),
    include_str!("../../shaders/actor_shadow.wgsl")
);

pub(super) struct Pipelines {
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    /// Indexed by [`Kind::index`]; made for the kinds loaded packs use.
    kinds: [Option<wgpu::RenderPipeline>; Kind::COUNT],
    pub shadow: wgpu::RenderPipeline,
    pub disc: wgpu::RenderPipeline,
}

impl Pipelines {
    /// Fails on a device that cannot read storage buffers in vertex shaders,
    /// such as some OpenGL ES 3.1 ones.
    pub fn new(
        gpu: &Gpu,
        layouts: &Layouts,
        actor: &wgpu::BindGroupLayout,
        shadow_view: &wgpu::BindGroupLayout,
        quality: &Quality,
    ) -> Result<Pipelines, RenderError> {
        let device = &gpu.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = |label, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let scene = module("actor", SCENE_SOURCE);
        let shadow_module = module("actor shadow", SHADOW_SOURCE);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("actor"),
            bind_group_layouts: &[Some(&layouts.frame), Some(actor), Some(&layouts.material)],
            immediate_size: 0,
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("actor shadow"),
            bind_group_layouts: &[None, Some(actor), Some(shadow_view)],
            immediate_size: 0,
        });
        let disc_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("disc shadow"),
            bind_group_layouts: &[Some(&layouts.frame), Some(actor)],
            immediate_size: 0,
        });
        let shadow = gpu.pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("actor shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shadow_module,
                entry_point: Some("vs_actor_shadow"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout())],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                bias: crate::shadow::CASTER_BIAS,
                ..depth_state(true, wgpu::CompareFunction::Less)
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let darken = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let disc = gpu.pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("disc shadow"),
            layout: Some(&disc_layout),
            vertex: wgpu::VertexState {
                module: &scene,
                entry_point: Some("vs_disc"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(depth_state(false, wgpu::CompareFunction::LessEqual)),
            multisample: wgpu::MultisampleState {
                count: quality.msaa,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &scene,
                entry_point: Some("fs_disc"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: darken,
                        alpha: darken,
                    }),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(RenderError::Device(format!(
                "actors need storage buffers in vertex shaders: {error}"
            )));
        }
        Ok(Pipelines {
            module: scene,
            layout,
            kinds: Default::default(),
            shadow,
            disc,
        })
    }

    /// Creates `kind`'s pipeline unless it exists.
    pub fn ensure(&mut self, gpu: &Gpu, kind: Kind, quality: &Quality) {
        let slot = &mut self.kinds[kind.index()];
        if slot.is_none() {
            *slot = Some(mesh_pipeline(
                gpu,
                &self.layout,
                &self.module,
                kind,
                quality,
                ("vs_actor", vertex_layout()),
            ));
        }
    }

    pub fn kind(&self, kind: Kind) -> &wgpu::RenderPipeline {
        self.kinds[kind.index()]
            .as_ref()
            .expect("made when the pack using it was loaded")
    }
}

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Unorm8x4, 4 => Uint16x4, 5 => Unorm16x4
    ];
    wgpu::VertexBufferLayout {
        array_stride: size_of::<SkinnedVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRIBUTES,
    }
}
