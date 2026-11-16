// SPDX-License-Identifier: Apache-2.0

//! Shadows: one depth atlas shared by the sun and the highest-ranked local
//! lights. A slot is drawn when a light gets it, and again only when a shadow
//! caster within that light's reach is shown or hidden.

use std::collections::VecDeque;

use bytemuck::{Pod, Zeroable};
use glam::camera::rh::{proj::directx, view::look_to_mat4};
use glam::{BVec3, Mat4, Vec2, Vec3, vec2, vec3};
use sariema_core::Camera;
use sariema_format::{Light, LightKind, Sun};

use crate::actors::Actors;
use crate::cluster::{priority, reach, spot_axis};
use crate::cull;
use crate::resources::{DrawItem, Room, strided};
use crate::scene::{DEPTH_FORMAT, depth_state, vertex_layout};
use crate::{Gpu, Quality};

const SOURCE: &str = concat!(
    include_str!("../shaders/common.wgsl"),
    include_str!("../shaders/shadow.wgsl")
);
const VIEW_SIZE: u64 = size_of::<[[f32; 4]; 4]>() as u64;
/// Depth offset of every caster drawn into the atlas.
pub(crate) const CASTER_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 2,
    slope_scale: 2.0,
    clamp: 0.0,
};
pub(crate) const GPU_SHADOW_SIZE: u64 = size_of::<GpuShadow>() as u64;

/// Bytes per slot in the shadow pass's uniform of view-projections.
pub(crate) fn view_stride(limits: &wgpu::Limits) -> u64 {
    VIEW_SIZE.next_multiple_of(limits.min_uniform_buffer_offset_alignment as u64)
}

/// Mirrors `Shadow` in scene.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuShadow {
    to_atlas: [[f32; 4]; 4],
    bounds: [f32; 4],
    texel: [f32; 2],
    _pad: [f32; 2],
}

/// A square of the atlas, in texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: u32,
    y: u32,
    size: u32,
}

impl Rect {
    fn quadrants(self) -> [Rect; 4] {
        let size = self.size / 2;
        [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(i, j)| Rect {
            x: self.x + i * size,
            y: self.y + j * size,
            size,
        })
    }
}

/// Squares of at least `min` texels for the sun, if `sun`, and for `lights`
/// lights, largest first: the sun takes a quadrant, then the largest free
/// square is split in four until there are enough or halves would be too small.
fn allocate(atlas: u32, sun: bool, lights: usize, min: u32) -> (Option<Rect>, Vec<Rect>) {
    let root = Rect {
        x: 0,
        y: 0,
        size: atlas,
    };
    let (sun, mut free) = if sun {
        let [first, rest @ ..] = root.quadrants();
        (Some(first), VecDeque::from(rest))
    } else {
        (None, VecDeque::from([root]))
    };
    // Splitting only the largest keeps `free` sorted largest first.
    while free.len() < lights && free[0].size / 2 >= min {
        let largest = free.pop_front().expect("never empty");
        free.extend(largest.quadrants());
    }
    free.truncate(lights);
    free.retain(|r| r.size >= min);
    (sun.filter(|r| r.size >= min), free.into())
}

/// Room to place squares in without allocating.
#[derive(Debug, Default)]
struct Placing {
    taken: Vec<bool>,
    placed: Vec<Option<usize>>,
}

/// Fills `out` with the square for each rank, of `rects[rank]`'s size. A
/// light keeps the square it `held` while that size holds; the others take
/// free squares of their size.
fn place(rects: &[Rect], held: &[Option<Rect>], scratch: &mut Placing, out: &mut Vec<Rect>) {
    let Placing { taken, placed } = scratch;
    taken.clear();
    taken.resize(rects.len(), false);
    placed.clear();
    placed.extend(held.iter().enumerate().map(|(rank, held)| {
        let kept = rects.iter().position(|r| Some(*r) == *held)?;
        (rects[kept].size == rects[rank].size && !taken[kept]).then(|| {
            taken[kept] = true;
            kept
        })
    }));
    for (rank, square) in placed.iter_mut().enumerate() {
        if square.is_none() {
            let size = rects[rank].size;
            let free = (0..rects.len())
                .find(|&i| !taken[i] && rects[i].size == size)
                .expect("there are as many squares of a size as ranks that want it");
            taken[free] = true;
            *square = Some(free);
        }
    }
    out.clear();
    out.extend(placed.iter().flatten().map(|&i| rects[i]));
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    Sun,
    Light(usize),
}

/// A light's map in the atlas.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slot {
    source: Source,
    rect: Rect,
    view_proj: Mat4,
    /// World size of a texel times the map's side: x plus y × distance to the light.
    texel: Vec2,
    /// Whether actors are drawn into it.
    actors: bool,
}

impl Slot {
    /// Whether a caster within `min..max` can be in this slot's map.
    fn reaches(&self, lights: &[Light], (min, max): (Vec3, Vec3)) -> bool {
        match self.source {
            Source::Sun => true,
            Source::Light(i) => {
                let s = reach(&lights[i]);
                s.center.clamp(min, max).distance_squared(s.center) <= s.radius * s.radius
            }
        }
    }

    fn gpu(&self, atlas: u32) -> GpuShadow {
        let atlas = atlas as f32;
        let [x, y, size] = [self.rect.x, self.rect.y, self.rect.size].map(|t| t as f32 / atlas);
        // NDC to the slot's texture coordinates, whose v points down.
        let to_slot = Mat4::from_translation(vec3(x + size * 0.5, y + size * 0.5, 0.0))
            * Mat4::from_scale(vec3(size * 0.5, -size * 0.5, 1.0));
        // Half a texel in, so filtering never reads a neighbouring slot.
        let inset = 0.5 / atlas;
        GpuShadow {
            to_atlas: (to_slot * self.view_proj).to_cols_array_2d(),
            bounds: [x + inset, y + inset, x + size - inset, y + size - inset],
            texel: (self.texel / self.rect.size as f32).into(),
            _pad: [0.0; 2],
        }
    }
}

/// The shadow slots of one frame.
#[derive(Debug, Default)]
pub(crate) struct Assigned {
    /// The slot of each of the frame's visible lights, in their order.
    pub lights: Vec<Option<u32>>,
    pub sun: Option<u32>,
    /// Visible lights with `casts_shadow` left without a slot.
    pub skipped: u32,
}

pub(crate) struct Shadows {
    shadow: wgpu::RenderPipeline,
    clear: wgpu::RenderPipeline,
    atlas: wgpu::TextureView,
    sampler: wgpu::Sampler,
    gpu_slots: wgpu::Buffer,
    views: wgpu::Buffer,
    views_group: wgpu::BindGroup,
    view_stride: u32,
    atlas_size: u32,
    max_lights: u32,
    min_slot: u32,
    actor_slots: u32,
    /// Group 2 of every caster pipeline: a slot's view-projection.
    pub view_layout: wgpu::BindGroupLayout,
    /// The room's squares: the sun's, then one per shadowed rank, largest first.
    layout: (Option<Rect>, Vec<Rect>),
    /// Bounds of all the room's draws.
    room_bounds: Option<(Vec3, Vec3)>,
    /// What the atlas holds, by slot index.
    slots: Vec<Slot>,
    /// Slots to draw this frame.
    stale: Vec<usize>,
    /// Bounds of the casters shown or hidden since the last frame.
    toggled: Vec<(Vec3, Vec3)>,
    /// Bounds of the visible casters, if there are any; `None` until measured.
    casters: Option<Option<(Vec3, Vec3)>>,
    drawn: u32,
    assigned: Assigned,
    ranked: Vec<(f32, usize)>,
    held: Vec<Option<Rect>>,
    placing: Placing,
    placed: Vec<Rect>,
    /// The frame's slots while they are compared with `slots`.
    next: Vec<Slot>,
}

impl Shadows {
    pub fn new(gpu: &Gpu, quality: &Quality, draw_layout: &wgpu::BindGroupLayout) -> Shadows {
        let device = &gpu.device;
        let atlas_size = quality.shadow_atlas_size;
        let atlas = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("shadow atlas"),
                size: wgpu::Extent3d {
                    width: atlas_size,
                    height: atlas_size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let slots = quality.max_shadowed_lights as u64 + 1;
        let view_stride = view_stride(&device.limits());
        let gpu_slots = gpu.buffer(
            "shadow slots",
            slots * GPU_SHADOW_SIZE,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let views = gpu.buffer(
            "shadow views",
            slots * view_stride,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow view"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(VIEW_SIZE),
                },
                count: None,
            }],
        });
        let views_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow views"),
            layout: &view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &views,
                    offset: 0,
                    size: wgpu::BufferSize::new(VIEW_SIZE),
                }),
            }],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let layout = |groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("shadow"),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        let pipeline = |layout: &wgpu::PipelineLayout,
                        entry_point,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        depth_stencil| {
            gpu.pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry_point),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    buffers,
                },
                // Both faces: single-sided casters still cast, and a mirroring
                // transform needs no other winding.
                primitive: Default::default(),
                depth_stencil: Some(depth_stencil),
                multisample: Default::default(),
                fragment: None,
                multiview_mask: None,
                cache: None,
            })
        };
        // Group 0 is the scene's frame, which samples the atlas this pass writes.
        let shadow = pipeline(
            &layout(&[None, Some(draw_layout), Some(&view_layout)]),
            "vs_shadow",
            &[Some(vertex_layout())],
            wgpu::DepthStencilState {
                bias: CASTER_BIAS,
                ..depth_state(true, wgpu::CompareFunction::Less)
            },
        );
        let clear = pipeline(
            &layout(&[]),
            "vs_clear",
            &[],
            depth_state(true, wgpu::CompareFunction::Always),
        );
        Shadows {
            shadow,
            clear,
            atlas,
            sampler,
            gpu_slots,
            views,
            views_group,
            view_stride: view_stride as u32,
            atlas_size,
            max_lights: quality.max_shadowed_lights,
            min_slot: quality.shadow_min_slot_px,
            actor_slots: quality.actor_shadow_slots,
            view_layout,
            layout: (None, Vec::new()),
            room_bounds: None,
            slots: Vec::new(),
            stale: Vec::new(),
            toggled: Vec::new(),
            casters: None,
            drawn: 0,
            assigned: Assigned::default(),
            ranked: Vec::new(),
            held: Vec::new(),
            placing: Placing::default(),
            placed: Vec::new(),
            next: Vec::new(),
        }
    }

    /// Bindings 5 to 7 of the scene's frame group; see [`layout_entries`].
    pub fn group_entries(&self) -> [wgpu::BindGroupEntry<'_>; 3] {
        [
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&self.atlas),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: self.gpu_slots.as_entire_binding(),
            },
        ]
    }

    /// Slots drawn since [`Shadows::new`].
    pub fn drawn(&self) -> u32 {
        self.drawn
    }

    /// Forgets what the atlas holds and lays it out for `room`: a quadrant
    /// for its sun if that casts shadows, and a square per shadow-casting
    /// light up to the budget. Lights that join or leave the view never
    /// change the layout, so they never move another light's square.
    pub fn reset(&mut self, room: &Room) {
        self.slots.clear();
        self.stale.clear();
        self.toggled.clear();
        self.casters = None;
        self.room_bounds = bounds(room.draws.iter());
        let sun = room.sun.as_ref().is_some_and(|s| s.casts_shadow);
        let lights = room.lights.iter().filter(|l| l.casts_shadow).count();
        let lights = lights.min(self.max_lights as usize);
        self.layout = allocate(self.atlas_size, sun, lights, self.min_slot);
    }

    /// Redraws, next frame, the slots that can see a caster within `min..max`.
    pub fn caster_toggled(&mut self, min: Vec3, max: Vec3) {
        self.toggled.push((min, max));
    }

    /// Gives slots to the sun and to the `visible` lights (indices into the
    /// room's lights) that cast shadows, ranked by [`priority`] at the
    /// camera's focus, and uploads them if they changed. The first
    /// `Quality::actor_shadow_slots` hold actors too, and are redrawn when
    /// one of `actors_moved` is within their reach.
    pub fn assign(
        &mut self,
        gpu: &Gpu,
        room: Option<&Room>,
        camera: &Camera,
        visible: &[usize],
        actors_moved: &[(Vec3, Vec3)],
    ) -> &Assigned {
        let assigned = &mut self.assigned;
        assigned.lights.clear();
        assigned.lights.resize(visible.len(), None);
        assigned.sun = None;
        assigned.skipped = 0;
        self.stale.clear();
        let Some(room) = room else {
            return &self.assigned;
        };
        if !self.toggled.is_empty() || self.casters.is_none() {
            self.casters = Some(bounds(
                room.draws.iter().filter(|d| d.visible && d.casts_shadow),
            ));
        }
        let ranked = &mut self.ranked;
        ranked.clear();
        ranked.extend(
            (visible.iter().enumerate())
                .filter(|&(_, &i)| room.lights[i].casts_shadow)
                .map(|(k, &i)| (priority(&room.lights[i], camera.focus), k)),
        );
        // Equal priorities keep the frame's order.
        ranked.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let (sun_rect, rects) = &self.layout;
        let casting = ranked.len();
        ranked.truncate(rects.len());
        assigned.skipped = (casting - ranked.len()) as u32;

        let slots = &mut self.next;
        slots.clear();
        let casters = self.casters.flatten();
        let sun = (sun_rect.zip(room.sun.as_ref()).zip(casters))
            .and_then(|((rect, sun), casters)| Some((rect, sun_view(sun, casters)?)));
        if let Some((rect, (view_proj, texel))) = sun {
            assigned.sun = Some(0);
            slots.push(Slot {
                source: Source::Sun,
                rect,
                view_proj,
                texel,
                actors: self.actor_slots > 0,
            });
        }
        self.held.clear();
        self.held.extend(ranked.iter().map(|&(_, k)| {
            let source = Source::Light(visible[k]);
            (self.slots.iter())
                .find(|s| s.source == source)
                .map(|s| s.rect)
        }));
        place(rects, &self.held, &mut self.placing, &mut self.placed);
        for (&(_, k), &rect) in ranked.iter().zip(&self.placed) {
            let (view_proj, texel) = light_view(&room.lights[visible[k]], self.room_bounds);
            assigned.lights[k] = Some(slots.len() as u32);
            slots.push(Slot {
                source: Source::Light(visible[k]),
                rect,
                view_proj,
                texel,
                actors: (slots.len() as u32) < self.actor_slots,
            });
        }

        let (held, toggled) = (&self.slots, &self.toggled);
        self.stale.extend((0..slots.len()).filter(|&i| {
            let slot = &slots[i];
            let sees = |b: &(Vec3, Vec3)| slot.reaches(&room.lights, *b);
            !held.contains(slot)
                || toggled.iter().any(sees)
                || (slot.actors && actors_moved.iter().any(sees))
        }));
        self.toggled.clear();
        if *slots != self.slots {
            let views = strided(
                slots.iter().map(|s| s.view_proj.to_cols_array_2d()),
                self.view_stride as usize,
            );
            let gpu_slots: Vec<GpuShadow> = slots.iter().map(|s| s.gpu(self.atlas_size)).collect();
            if !views.is_empty() {
                gpu.queue.write_buffer(&self.views, 0, &views);
                gpu.queue
                    .write_buffer(&self.gpu_slots, 0, bytemuck::cast_slice(&gpu_slots));
            }
            std::mem::swap(&mut self.slots, &mut self.next);
        }
        &self.assigned
    }

    /// Whether a slot of this frame that holds actors has `min..max` within
    /// its map's view.
    pub fn maps(&self, (min, max): (Vec3, Vec3)) -> bool {
        (self.slots.iter())
            .any(|s| s.actors && cull::box_visible(&cull::planes(s.view_proj), min, max))
    }

    /// Draws the slots [`Shadows::assign`] found stale.
    pub fn draw(&mut self, encoder: &mut wgpu::CommandEncoder, room: &Room, actors: &Actors) {
        if self.stale.is_empty() {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadows"),
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.atlas,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        let mut meshes_bound = false;
        for &i in &self.stale {
            let slot = &self.slots[i];
            let Rect { x, y, size } = slot.rect;
            pass.set_viewport(x as f32, y as f32, size as f32, size as f32, 0.0, 1.0);
            pass.set_scissor_rect(x, y, size, size);
            pass.set_pipeline(&self.clear);
            pass.draw(0..3, 0..1);
            let casters = room.draws.iter().enumerate().filter(|(_, d)| {
                d.visible
                    && d.casts_shadow
                    && d.index_count > 0
                    && slot.reaches(&room.lights, (d.min, d.max))
            });
            pass.set_pipeline(&self.shadow);
            pass.set_bind_group(2, &self.views_group, &[i as u32 * self.view_stride]);
            for (d, _) in casters {
                if !meshes_bound {
                    room.bind_meshes(&mut pass);
                    meshes_bound = true;
                }
                room.draw(&mut pass, d);
            }
            if slot.actors {
                let offset = i as u32 * self.view_stride;
                actors.draw_shadows(&mut pass, &self.views_group, offset, |b| {
                    slot.reaches(&room.lights, b)
                });
                meshes_bound = false;
            }
        }
        self.drawn += self.stale.len() as u32;
        self.stale.clear();
    }
}

/// Bindings 5 to 7 of the scene's frame group: the atlas, its comparison
/// sampler and the slots.
pub(crate) fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 3] {
    let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    };
    [
        entry(
            5,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        ),
        entry(
            6,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
        ),
        entry(
            7,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ),
    ]
}

/// World bounds of `draws`; `None` if there are none.
fn bounds<'a>(draws: impl Iterator<Item = &'a DrawItem>) -> Option<(Vec3, Vec3)> {
    draws
        .map(|d| (d.min, d.max))
        .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))
}

/// Any up vector not parallel to `forward`.
fn up_for(forward: Vec3) -> Vec3 {
    if forward.y.abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    }
}

/// An orthographic map along the sun's rays, fitted to the casters within
/// `min..max`; receivers beyond them compare at its far plane.
fn sun_view(sun: &Sun, (min, max): (Vec3, Vec3)) -> Option<(Mat4, Vec2)> {
    let rays = -Vec3::from(sun.direction).try_normalize()?;
    let view = look_to_mat4(Vec3::ZERO, rays, up_for(rays));
    let (mut lo, mut hi) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    for i in 0..8 {
        let corner = Vec3::select(BVec3::new(i & 1 != 0, i & 2 != 0, i & 4 != 0), max, min);
        let p = view.transform_point3(corner);
        (lo, hi) = (lo.min(p), hi.max(p));
    }
    // A centimetre of margin gives even a flat box, a lone floor, some depth.
    let (lo, hi) = (lo - 0.01, hi + 0.01);
    let proj = directx::orthographic(lo.x, hi.x, lo.y, hi.y, -hi.z, -lo.z);
    Some((proj * view, vec2((hi.x - lo.x).max(hi.y - lo.y), 0.0)))
}

/// A perspective map along a spot's cone. A point light is treated as a
/// spot of 120° pointing straight down: nothing beside or above it casts a
/// shadow from it (known limitation).
fn light_view(light: &Light, room_bounds: Option<(Vec3, Vec3)>) -> (Mat4, Vec2) {
    let position = Vec3::from(light.position);
    let (axis, cone_deg) = match light.kind {
        LightKind::Spot { target, cone_deg } => (spot_axis(light.position, target), cone_deg),
        LightKind::Point => (Vec3::NEG_Y, 120.0),
    };
    let far = if light.range > 0.0 {
        light.range
    } else {
        // An empty room's maps hold nothing, so any far plane does.
        room_bounds.map_or(1.0, |(min, max)| {
            (position - min).abs().max((position - max).abs()).length()
        })
    };
    // One perspective map holds at most 170°; a wider cone is shadowed only within it.
    let fov = cone_deg.clamp(1.0, 170.0).to_radians();
    let view = look_to_mat4(position, axis, up_for(axis));
    // Casters within 1% of the far plane, such as a lamp's own shade, are clipped.
    let proj = directx::perspective(fov, 1.0, far * 0.01, far);
    (proj * view, vec2(0.0, 2.0 * (fov * 0.5).tan()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(rects: &[Rect], held: &[Option<Rect>]) -> Vec<Rect> {
        let mut out = Vec::new();
        super::place(rects, held, &mut Placing::default(), &mut out);
        out
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.size && b.x < a.x + a.size && a.y < b.y + b.size && b.y < a.y + a.size
    }

    #[test]
    fn sun_takes_a_quadrant_and_higher_ranks_get_larger_slots() {
        let (sun, lights) = allocate(1024, true, 5, 1);
        assert_eq!(sun.map(|r| r.size), Some(512));
        let sizes: Vec<u32> = lights.iter().map(|r| r.size).collect();
        assert_eq!(sizes, [512, 512, 256, 256, 256]);
    }

    #[test]
    fn one_light_alone_gets_the_whole_atlas() {
        let (sun, lights) = allocate(1024, false, 1, 1);
        assert_eq!((sun, lights[0].size), (None, 1024));
    }

    #[test]
    fn slots_are_disjoint_and_inside_the_atlas() {
        for (sun, count) in [(true, 0), (true, 3), (false, 7), (true, 32), (false, 200)] {
            let (s, lights) = allocate(4096, sun, count, 1);
            let all: Vec<Rect> = s.into_iter().chain(lights.iter().copied()).collect();
            assert_eq!(lights.len(), count);
            for (i, &a) in all.iter().enumerate() {
                assert!(a.size > 0 && a.x + a.size <= 4096 && a.y + a.size <= 4096);
                assert!(all[i + 1..].iter().all(|&b| !overlap(a, b)), "{all:?}");
            }
        }
    }

    #[test]
    fn no_square_is_below_the_minimum() {
        let (sun, lights) = allocate(1024, true, 100, 256);
        assert_eq!(sun.map(|r| r.size), Some(512));
        let sizes: Vec<u32> = lights.iter().map(|r| r.size).collect();
        assert_eq!(sizes, [256; 12]);
        assert_eq!(allocate(1024, true, 1, 1024), (None, vec![]));
    }

    #[test]
    fn a_light_keeps_its_square_when_another_joins_or_leaves() {
        let (_, rects) = allocate(4096, false, 3, 1);
        let alone = place(&rects, &[None]);
        let joined = place(&rects, &[None, Some(alone[0])]);
        assert_eq!(joined[1], alone[0]);
        assert_ne!(joined[0], alone[0]);
        assert_eq!(place(&rects, &[Some(joined[0])]), [joined[0]]);
    }

    #[test]
    fn a_light_moves_only_when_its_rank_needs_another_size() {
        let (_, rects) = allocate(1024, true, 5, 1);
        let sizes = |placed: &[Rect]| placed.iter().map(|r| r.size).collect::<Vec<_>>();
        let first = place(&rects, &[None, None, None]);
        assert_eq!(sizes(&first), [512, 512, 256]);
        let swapped = place(&rects, &[Some(first[1]), Some(first[0]), Some(first[2])]);
        assert_eq!(swapped, [first[1], first[0], first[2]]);
        let demoted = place(&rects, &[None, None, Some(first[0])]);
        assert_eq!(sizes(&demoted), [512, 512, 256]);
        assert!(demoted[0] == first[0] || demoted[1] == first[0]);
        assert_ne!(demoted[2], first[0]);
    }
}
