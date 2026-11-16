// SPDX-License-Identifier: Apache-2.0

//! The actors' pipelines, created when the first pack is loaded: one per
//! material kind its parts use, the shadow caster, the disc shadow and, on
//! the downlevel path, the depth pass's.

use sariema_format::actor::SkinnedVertex;

use crate::scene::{self, DEPTH_FORMAT, HDR_FORMAT, Kind, Layouts, depth_state, mesh_pipeline};
use crate::{Gpu, Quality, RenderError, RenderPath};

/// How skin.wgsl reads `actor` on `quality`'s path.
fn record(quality: &Quality) -> String {
    match quality.path {
        RenderPath::Downlevel => format!(
            "const ACTOR_ROWS: u32 = {}u;\n{}",
            quality.actor_uniform_rows,
            include_str!("../../shaders/skin_uniform.wgsl")
        ),
        _ => include_str!("../../shaders/skin_storage.wgsl").to_string(),
    }
}

pub(super) struct Pipelines {
    module: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    /// Indexed by [`Kind::index`]; made for the kinds loaded packs use.
    kinds: [Option<wgpu::RenderPipeline>; Kind::COUNT],
    pub shadow: wgpu::RenderPipeline,
    pub disc: wgpu::RenderPipeline,
    /// Indexed by `double_sided as usize`; made where the scene draws a
    /// depth pass of its own.
    depth: Option<[wgpu::RenderPipeline; 2]>,
}

impl Pipelines {
    /// Fails where the device rejects a pipeline, as one that forced
    /// [`RenderPath::Full`] without storage buffers in vertex shaders would.
    pub fn new(
        gpu: &Gpu,
        layouts: &Layouts,
        actor: &wgpu::BindGroupLayout,
        shadow_view: &wgpu::BindGroupLayout,
        quality: &Quality,
        depth_pass: bool,
    ) -> Result<Pipelines, RenderError> {
        let device = &gpu.device;
        let scope = Scope::push(device);
        let module = |label, source: &str| gpu.shader(label, source.into());
        let record = record(quality);
        let scene_source = [
            scene::source(quality).as_str(),
            &record,
            include_str!("../../shaders/skin.wgsl"),
            include_str!("../../shaders/actor.wgsl"),
        ]
        .concat();
        let shadow_source = [
            include_str!("../../shaders/common.wgsl"),
            &record,
            include_str!("../../shaders/skin.wgsl"),
            include_str!("../../shaders/shadow.wgsl"),
            include_str!("../../shaders/actor_shadow.wgsl"),
        ]
        .concat();
        let scene = module("actor", &scene_source);
        let shadow_module = module("actor shadow", &shadow_source);
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
        let depth = depth_pass.then(|| {
            [false, true].map(|double_sided| {
                gpu.pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("actor depth"),
                    layout: Some(&disc_layout),
                    vertex: wgpu::VertexState {
                        module: &scene,
                        entry_point: Some("vs_actor_depth"),
                        compilation_options: Default::default(),
                        buffers: &[Some(vertex_layout())],
                    },
                    primitive: wgpu::PrimitiveState {
                        cull_mode: (!double_sided).then_some(wgpu::Face::Back),
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        ..depth_state(true, wgpu::CompareFunction::Less)
                    }),
                    multisample: Default::default(),
                    fragment: None,
                    multiview_mask: None,
                    cache: None,
                })
            })
        });
        if let Some(error) = scope.pop() {
            return Err(RenderError::Device(format!(
                "the actors' pipelines were rejected: {error}"
            )));
        }
        Ok(Pipelines {
            module: scene,
            layout,
            kinds: Default::default(),
            shadow,
            disc,
            depth,
        })
    }

    pub fn depth(&self, double_sided: bool) -> &wgpu::RenderPipeline {
        let depth = self
            .depth
            .as_ref()
            .expect("made where the scene has a depth pass");
        &depth[double_sided as usize]
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
                (false, true),
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

/// A validation error scope where its result can be waited for: not in a
/// browser's WebGPU, which settles it only once the page yields.
struct Scope(Option<wgpu::ErrorScopeGuard>);

impl Scope {
    fn push(device: &wgpu::Device) -> Scope {
        let browser = device.adapter_info().backend == wgpu::Backend::BrowserWebGpu;
        Scope((!browser).then(|| device.push_error_scope(wgpu::ErrorFilter::Validation)))
    }

    fn pop(self) -> Option<wgpu::Error> {
        self.0.and_then(|scope| pollster::block_on(scope.pop()))
    }
}
