// SPDX-License-Identifier: Apache-2.0

//! The UI pass: instanced quads over the frame, one draw per batch, the
//! batch's texture bound and its scissor set. Shapes are distance fields
//! evaluated per pixel (rounded boxes with elliptical corners, borders,
//! blurred shadows, rings, stroke segments); glyphs sample a one-channel
//! distance-field atlas; images are premultiplied RGBA.
//!
//! The pipeline, sampler and first buffers are created in [`UiPass::new`];
//! the instance buffer grows only when a frame has more quads than ever.
//! Colours arrive as sRGB with straight alpha and are blended
//! premultiplied; on an sRGB target they are converted to linear first, so
//! blending there happens in linear light.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use image::RgbaImage;

use crate::{RenderError, request_device};

/// One quad, as `sariema_ui::Quad`: same fields, same layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct UiInstance {
    pub rect: [f32; 4],
    pub xform: [f32; 6],
    pub uv: [f32; 4],
    pub fill: [f32; 4],
    pub stroke: [f32; 4],
    pub radii: [f32; 4],
    pub radii_y: [f32; 4],
    pub params: [f32; 4],
    pub kind: u32,
    pub gradient: [f32; 3],
}

/// Quads `start..start + count` with one texture and one scissor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiBatch {
    /// An id given to [`UiPass::set_texture`]; any other draws with a
    /// white texel, which shapes ignore.
    pub texture: u32,
    /// Pixels: x, y, width, height.
    pub scissor: [u32; 4],
    pub start: u32,
    pub count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiTextureFormat {
    /// One channel: the glyph atlas.
    R8,
    /// Premultiplied RGBA8: images.
    Rgba8,
}

const SHADER: &str = r#"
struct Globals { screen: vec2f, linear_out: u32, pad: u32 }
@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct Instance {
    @location(0) rect: vec4f,
    @location(1) xf: vec4f,
    @location(2) xt: vec2f,
    @location(3) uv: vec4f,
    @location(4) fill: vec4f,
    @location(5) stroke: vec4f,
    @location(6) radii: vec4f,
    @location(7) radii_y: vec4f,
    @location(8) params: vec4f,
    @location(9) kind: u32,
    @location(10) grad: vec3f,
}

struct Varyings {
    @builtin(position) pos: vec4f,
    @location(0) local: vec2f,
    @location(1) uv: vec2f,
    @location(2) @interpolate(flat) kind: u32,
    @location(3) @interpolate(flat) rect: vec4f,
    @location(4) @interpolate(flat) fill: vec4f,
    @location(5) @interpolate(flat) stroke: vec4f,
    @location(6) @interpolate(flat) radii: vec4f,
    @location(7) @interpolate(flat) radii_y: vec4f,
    @location(8) @interpolate(flat) params: vec4f,
    @location(9) @interpolate(flat) seg: vec4f,
    @location(10) @interpolate(flat) grad: vec3f,
}

const RECT: u32 = 0u;
const GLYPH: u32 = 1u;
const IMAGE: u32 = 2u;
const RING: u32 = 3u;
const SEGMENT: u32 = 4u;
const LINEAR: u32 = 5u;
const RADIAL: u32 = 6u;
const TAU: f32 = 6.28318530718;

@vertex
fn vs(@builtin(vertex_index) vi: u32, i: Instance) -> Varyings {
    let corner = vec2f(f32(vi & 1u), f32(vi >> 1u));
    var lo = i.rect.xy;
    var size = i.rect.zw;
    let boxed = i.kind == RECT || i.kind == LINEAR || i.kind == RADIAL;
    if (boxed || i.kind == RING || i.kind == SEGMENT) {
        // Room for the antialiased edge, and for a shadow's blur.
        let sx = max(length(i.xf.xy), 1e-6);
        let sy = max(length(i.xf.zw), 1e-6);
        var pad = vec2f(1.5 / sx, 1.5 / sy);
        if (boxed) {
            pad += vec2f(i.params.y * 1.5);
        }
        lo -= pad;
        size += 2.0 * pad;
    }
    let local = lo + corner * size;
    let px = vec2f(
        i.xf.x * local.x + i.xf.z * local.y + i.xt.x,
        i.xf.y * local.x + i.xf.w * local.y + i.xt.y,
    );
    var out: Varyings;
    out.pos = vec4f(px.x / g.screen.x * 2.0 - 1.0, 1.0 - px.y / g.screen.y * 2.0, 0.0, 1.0);
    out.local = local;
    out.uv = mix(i.uv.xy, i.uv.zw, corner);
    out.kind = i.kind;
    out.rect = i.rect;
    out.fill = i.fill;
    out.stroke = i.stroke;
    out.radii = i.radii;
    out.radii_y = i.radii_y;
    out.params = i.params;
    out.seg = i.uv;
    out.grad = i.grad;
    return out;
}

fn premul(c: vec4f) -> vec4f {
    return vec4f(c.rgb * c.a, c.a);
}

// Abramowitz and Stegun 7.1.26.
fn erf(x: f32) -> f32 {
    let a = abs(x);
    let t = 1.0 / (1.0 + 0.3275911 * a);
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t
        + 0.254829592) * t * exp(-a * a);
    return sign(x) * y;
}

// Distance to a box of half size `half` whose corner in p's quadrant has
// radii `r`; elliptical corners use Quilez's gradient-scaled estimate.
fn box_distance(p: vec2f, half: vec2f, r: vec2f) -> f32 {
    let q = abs(p) - half + r;
    if (q.x > 0.0 && q.y > 0.0) {
        let rr = max(r, vec2f(1e-4));
        let k0 = length(q / rr);
        let k1 = length(q / (rr * rr));
        return k0 * (k0 - 1.0) / k1;
    }
    return max(q.x - r.x, q.y - r.y);
}

fn corner(p: vec2f, rx: vec4f, ry: vec4f) -> vec2f {
    // Corners: top-left, top-right, bottom-right, bottom-left; y is down.
    if (p.y < 0.0) {
        if (p.x < 0.0) { return vec2f(rx.x, ry.x); }
        return vec2f(rx.y, ry.y);
    }
    if (p.x < 0.0) { return vec2f(rx.w, ry.w); }
    return vec2f(rx.z, ry.z);
}

// Premultiplied, mixed in the colours' own encoding as browsers do.
fn box_fill(v: Varyings) -> vec4f {
    var t = 0.0;
    if (v.kind == LINEAR) {
        let ab = v.grad.xy - v.params.zw;
        t = dot(v.local - v.params.zw, ab) / max(dot(ab, ab), 1e-12);
    } else if (v.kind == RADIAL) {
        let q = (v.local - v.params.zw) / max(v.grad.xy, vec2f(1e-6));
        t = (length(q) - v.grad.z) / max(1.0 - v.grad.z, 1e-6);
    } else {
        return premul(v.fill);
    }
    return mix(premul(v.fill), premul(v.seg), clamp(t, 0.0, 1.0));
}

fn coverage(d: f32, w: f32) -> f32 {
    return clamp(0.5 - d / w, 0.0, 1.0);
}

fn to_linear(c: vec3f) -> vec3f {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3f(2.4));
    return select(high, low, c <= vec3f(0.04045));
}

@fragment
fn fs(v: Varyings) -> @location(0) vec4f {
    let texel = textureSampleLevel(tex, samp, v.uv, 0.0);
    let fl = fwidth(v.local);
    let fu = fwidth(v.uv);
    let w = max(length(fl) * 0.70710678, 1e-6);
    var color = vec4f(0.0);
    switch v.kind {
        case RECT, LINEAR, RADIAL: {
            let half = v.rect.zw * 0.5;
            let p = v.local - (v.rect.xy + half);
            let d = box_distance(p, half, corner(p, v.radii, v.radii_y));
            let blur = v.params.y;
            if (blur > 0.0) {
                // A Gaussian of sigma blur / 2, as CSS box-shadow.
                let cov = 0.5 - 0.5 * erf(d / (blur * 0.70710678));
                color = premul(v.fill) * cov;
            } else {
                let outer = coverage(d, w);
                let fill = box_fill(v);
                let bw = v.params.x;
                if (bw > 0.0) {
                    let inner = coverage(d + bw, w);
                    color = premul(v.stroke) * (outer - inner) + fill * inner;
                } else {
                    color = fill * outer;
                }
            }
        }
        case GLYPH: {
            let field = texel.r;
            let fw = max(length(fu) * v.params.z * 0.70710678, 1e-6);
            let soft = v.params.y;
            let fill_cov = clamp((field - 0.5) / (fw + soft) + 0.5, 0.0, 1.0);
            let outline = v.params.x;
            if (outline > 0.0) {
                let outer = clamp((field - (0.5 - outline)) / (fw + soft) + 0.5, 0.0, 1.0);
                color = premul(v.fill) * fill_cov + premul(v.stroke) * max(outer - fill_cov, 0.0);
            } else {
                color = premul(v.fill) * fill_cov;
            }
        }
        case IMAGE: {
            color = vec4f(texel.rgb * v.fill.rgb * v.fill.a, texel.a * v.fill.a);
        }
        case RING: {
            let sw = v.params.x;
            let c = v.rect.xy + v.rect.zw * 0.5;
            let p = v.local - c;
            let radius = min(v.rect.z, v.rect.w) * 0.5 - sw * 0.5;
            let d_ring = abs(length(p) - radius) - sw * 0.5;
            let start = v.params.y;
            let sweep = clamp(v.params.z, 0.0, 1.0) * TAU;
            let a = fract((atan2(p.x, -p.y) - start) / TAU) * TAU;
            var d_arc = 1e9;
            if (sweep > 0.0) {
                if (a <= sweep) {
                    d_arc = d_ring;
                } else {
                    let p0 = radius * vec2f(sin(start), -cos(start));
                    let p1 = radius * vec2f(sin(start + sweep), -cos(start + sweep));
                    d_arc = min(length(p - p0), length(p - p1)) - sw * 0.5;
                }
            }
            let arc = coverage(d_arc, w);
            color = premul(v.fill) * arc + premul(v.stroke) * coverage(d_ring, w) * (1.0 - arc);
        }
        case SEGMENT: {
            let a = v.seg.xy;
            let ba = v.seg.zw - a;
            let pa = v.local - a;
            let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
            let d = length(pa - ba * h) - v.params.x;
            color = premul(v.fill) * coverage(d, w);
        }
        default: {}
    }
    if (g.linear_out == 1u && color.a > 0.0) {
        color = vec4f(to_linear(color.rgb / color.a) * color.a, color.a);
    }
    return color;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen: [f32; 2],
    linear_out: u32,
    pad: u32,
}

struct Texture {
    texture: wgpu::Texture,
    format: UiTextureFormat,
    bind_group: wgpu::BindGroup,
}

impl Texture {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        [width, height]: [u32; 2],
        format: UiTextureFormat,
        texels: &[u8],
    ) -> Texture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui"),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: match format {
                UiTextureFormat::R8 => wgpu::TextureFormat::R8Unorm,
                UiTextureFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        let t = Texture {
            texture,
            format,
            bind_group,
        };
        t.upload(queue, texels);
        t
    }

    fn upload(&self, queue: &wgpu::Queue, texels: &[u8]) {
        let (width, height) = (self.texture.width(), self.texture.height());
        let bytes = match self.format {
            UiTextureFormat::R8 => 1,
            UiTextureFormat::Rgba8 => 4,
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * bytes),
                rows_per_image: Some(height),
            },
            extent(width, height),
        );
    }
}

/// Where [`UiPass::draw`] draws: a view of a `width × height` texture of
/// the pass's format.
#[derive(Clone, Copy)]
pub struct UiTarget<'a> {
    pub view: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

pub struct UiPass {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    instances: wgpu::Buffer,
    capacity: u64,
    textures: Vec<Option<Texture>>,
    white: wgpu::BindGroup,
    linear_out: bool,
    draw_calls: u32,
    buffers_created: u32,
    /// Texel bytes uploaded since the last draw.
    pending: u64,
    uploaded: u64,
}

impl UiPass {
    /// A pass drawing into targets of `format`, with room for `capacity`
    /// quads before its instance buffer grows.
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        format: wgpu::TextureFormat,
        capacity: u32,
    ) -> UiPass {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui"),
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x2, 3 => Float32x4, 4 => Float32x4,
            5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Uint32,
            10 => Float32x3
        ];
        let blend = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<UiInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: blend,
                        alpha: blend,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let capacity = u64::from(capacity.max(1));
        let white = Texture::new(
            &device,
            &queue,
            &texture_layout,
            &sampler,
            [1, 1],
            UiTextureFormat::Rgba8,
            &[255; 4],
        );
        UiPass {
            instances: instance_buffer(&device, capacity),
            white: white.bind_group,
            device,
            queue,
            pipeline,
            globals,
            globals_group,
            texture_layout,
            sampler,
            capacity,
            textures: Vec::new(),
            linear_out: format.is_srgb(),
            draw_calls: 0,
            buffers_created: 2,
            pending: 0,
            uploaded: 0,
        }
    }

    /// Creates texture `id` or replaces its texels; a new size or format
    /// creates it again.
    pub fn set_texture(
        &mut self,
        id: u32,
        width: u32,
        height: u32,
        format: UiTextureFormat,
        texels: &[u8],
    ) {
        self.pending += texels.len() as u64;
        let i = id as usize;
        if self.textures.len() <= i {
            self.textures.resize_with(i + 1, || None);
        }
        match &self.textures[i] {
            Some(t)
                if t.format == format
                    && t.texture.width() == width
                    && t.texture.height() == height =>
            {
                t.upload(&self.queue, texels);
            }
            _ => {
                self.textures[i] = Some(Texture::new(
                    &self.device,
                    &self.queue,
                    &self.texture_layout,
                    &self.sampler,
                    [width, height],
                    format,
                    texels,
                ));
            }
        }
    }

    /// Draws `batches` of `instances` into `target`, clearing it first to
    /// `clear` if given.
    pub fn draw(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: UiTarget,
        instances: &[UiInstance],
        batches: &[UiBatch],
        clear: Option<wgpu::Color>,
    ) {
        let UiTarget {
            view: target,
            width,
            height,
        } = target;
        self.draw_calls = 0;
        if instances.len() as u64 > self.capacity {
            self.capacity = (instances.len() as u64).next_power_of_two();
            self.instances = instance_buffer(&self.device, self.capacity);
            self.buffers_created += 1;
        }
        let globals = Globals {
            screen: [width as f32, height as f32],
            linear_out: u32::from(self.linear_out),
            pad: 0,
        };
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        let quads: &[u8] = bytemuck::cast_slice(instances);
        if !quads.is_empty() {
            self.queue.write_buffer(&self.instances, 0, quads);
        }
        self.uploaded =
            std::mem::take(&mut self.pending) + size_of::<Globals>() as u64 + quads.len() as u64;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.globals_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        for b in batches {
            let x0 = b.scissor[0].min(width);
            let y0 = b.scissor[1].min(height);
            let x1 = (b.scissor[0] + b.scissor[2]).min(width);
            let y1 = (b.scissor[1] + b.scissor[3]).min(height);
            let end = b.start.saturating_add(b.count).min(instances.len() as u32);
            if x1 <= x0 || y1 <= y0 || end <= b.start {
                continue;
            }
            pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
            let group = match self.textures.get(b.texture as usize) {
                Some(Some(t)) => &t.bind_group,
                _ => &self.white,
            };
            pass.set_bind_group(1, group, &[]);
            pass.draw(0..4, b.start..end);
            self.draw_calls += 1;
        }
    }

    /// Draw calls of the last [`UiPass::draw`].
    pub fn draw_calls(&self) -> u32 {
        self.draw_calls
    }

    /// Buffers created since [`UiPass::new`].
    pub fn buffers_created(&self) -> u32 {
        self.buffers_created
    }

    /// Bytes the last [`UiPass::draw`] wrote to buffers, with the texels
    /// [`UiPass::set_texture`] uploaded since the draw before it.
    pub fn uploaded(&self) -> u64 {
        self.uploaded
    }
}

fn instance_buffer(device: &wgpu::Device, capacity: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ui instances"),
        size: capacity * size_of::<UiInstance>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

const HEADLESS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A [`UiPass`] on its own device drawing into an offscreen image, cleared
/// to transparent: the result is premultiplied.
pub struct UiHeadless {
    pass: UiPass,
    target: wgpu::Texture,
    view: wgpu::TextureView,
}

impl UiHeadless {
    pub fn new(width: u32, height: u32, capacity: u32) -> Result<UiHeadless, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|_| RenderError::NoAdapter)?;
        let (device, queue) = request_device(&adapter, "sariema ui headless")?;
        let max = device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > max || height > max {
            return Err(RenderError::Device(format!(
                "{width} × {height} is not a texture size this device has"
            )));
        }
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui headless"),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HEADLESS_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(UiHeadless {
            pass: UiPass::new(device, queue, HEADLESS_FORMAT, capacity),
            view: target.create_view(&Default::default()),
            target,
        })
    }

    pub fn pass(&mut self) -> &mut UiPass {
        &mut self.pass
    }

    /// Draws one frame from transparent and reads it back.
    pub fn render(&mut self, instances: &[UiInstance], batches: &[UiBatch]) -> RgbaImage {
        let (width, height) = (self.target.width(), self.target.height());
        let mut encoder = self.pass.device.create_command_encoder(&Default::default());
        self.pass.draw(
            &mut encoder,
            UiTarget {
                view: &self.view,
                width,
                height,
            },
            instances,
            batches,
            Some(wgpu::Color::TRANSPARENT),
        );
        let row = width * 4;
        let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.pass.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui read back"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            extent(width, height),
        );
        self.pass.queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |result| {
            result.expect("mapping a read-back buffer");
        });
        self.pass
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("waiting for the GPU");
        let mapped = buffer.get_mapped_range(..).expect("mapped above");
        let texels = mapped
            .chunks_exact(padded as usize)
            .flat_map(|r| &r[..row as usize])
            .copied()
            .collect();
        RgbaImage::from_raw(width, height, texels).expect("4 bytes per texel")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32, fill: [f32; 4]) -> UiInstance {
        UiInstance {
            rect: [0.0, 0.0, w, h],
            xform: [1.0, 0.0, 0.0, 1.0, x, y],
            fill,
            ..UiInstance::default()
        }
    }

    fn batch(start: u32, count: u32, scissor: [u32; 4]) -> UiBatch {
        UiBatch {
            texture: u32::MAX,
            scissor,
            start,
            count,
        }
    }

    #[test]
    fn draws_batches_with_scissors_and_no_new_buffers() {
        let mut ui = UiHeadless::new(64, 32, 4).unwrap();
        let red = [1.0, 0.0, 0.0, 1.0];
        let half_green = [0.0, 1.0, 0.0, 0.5];
        let quads = [
            rect(4.5, 4.0, 20.0, 20.0, red),
            rect(30.0, 4.0, 20.0, 20.0, half_green),
        ];
        let batches = [batch(0, 1, [0, 0, 64, 32]), batch(1, 1, [0, 0, 40, 32])];
        for _ in 0..3 {
            let img = ui.render(&quads, &batches);
            assert_eq!(img.get_pixel(10, 10).0, [255, 0, 0, 255]);
            assert_eq!(img.get_pixel(1, 1).0, [0, 0, 0, 0]);
            // Premultiplied: half-transparent green is (0, 128, 0, 128).
            let g = img.get_pixel(35, 10).0;
            assert!(g[1].abs_diff(128) <= 1 && g[3].abs_diff(128) <= 1, "{g:?}");
            assert_eq!(img.get_pixel(45, 10).0, [0, 0, 0, 0], "outside the scissor");
            // The right edge at x = 24.5 halves the pixel it crosses.
            let edge = img.get_pixel(24, 10).0;
            assert!(
                edge[0].abs_diff(128) <= 2 && edge[3].abs_diff(128) <= 2,
                "{edge:?}"
            );
            assert_eq!(ui.pass().draw_calls(), 2);
        }
        assert_eq!(
            ui.pass().buffers_created(),
            2,
            "globals and instances, once"
        );
        let many = vec![rect(0.0, 0.0, 1.0, 1.0, red); 9];
        ui.render(&many, &[batch(0, 9, [0, 0, 64, 32])]);
        assert_eq!(
            ui.pass().buffers_created(),
            3,
            "grows past its capacity once"
        );
    }

    #[test]
    fn gradients_blend_premultiplied_along_their_line_and_radius() {
        let mut ui = UiHeadless::new(100, 60, 4).unwrap();
        let linear = UiInstance {
            uv: [0.0; 4],
            params: [0.0, 0.0, 0.0, 10.0],
            kind: 5,
            gradient: [100.0, 10.0, 0.0],
            ..rect(0.0, 0.0, 100.0, 20.0, [0.0, 0.0, 1.0, 1.0])
        };
        let radial = UiInstance {
            uv: [0.0, 1.0, 0.0, 1.0],
            params: [0.0, 0.0, 20.0, 20.0],
            kind: 6,
            gradient: [20.0, 20.0, 0.5],
            ..rect(30.0, 20.0, 40.0, 40.0, [1.0, 0.0, 0.0, 1.0])
        };
        let img = ui.render(&[linear, radial], &[batch(0, 2, [0, 0, 100, 60])]);
        let near = |p: [u8; 4], e: [u8; 4]| p.iter().zip(e).all(|(a, b)| a.abs_diff(b) <= 2);
        assert!(near(img.get_pixel(0, 10).0, [0, 0, 252, 252]));
        // Blue into transparent at t = 0.505: half the blue and half the
        // alpha, premultiplied; a straight mix would give a quarter blue.
        let mid = img.get_pixel(50, 10).0;
        assert!(near(mid, [0, 0, 126, 126]), "{mid:?}");
        assert!(near(img.get_pixel(99, 10).0, [0, 0, 1, 1]));
        assert_eq!(
            img.get_pixel(50, 40).0,
            [255, 0, 0, 255],
            "inside the inner stop"
        );
        // 14.5 units out of 20, the inner stop at 10: t = 0.451.
        let ring = img.get_pixel(64, 40).0;
        assert!(near(ring, [140, 115, 0, 255]), "{ring:?}");
        assert_eq!(img.get_pixel(31, 21).0, [0, 255, 0, 255], "past the radius");
    }

    #[test]
    fn the_pipeline_fits_webgl2() {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("an adapter");
        let limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: limits,
            ..Default::default()
        }))
        .expect("a device with WebGL2's limits");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pass = UiPass::new(
            Arc::new(device),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8Unorm,
            4,
        );
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        drop(pass);
    }
}
