// SPDX-License-Identifier: Apache-2.0

//! A room's meshes, textures, materials and draws on the GPU.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use sariema_format::{
    Background, Group, Light, Material, RoomPack, Sun, Texture, TextureFormat, Vertex,
};

use crate::scene::{Kind, Layouts};
use crate::{Gpu, RenderError};

/// Mirrors `Draw` in common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct DrawUniform {
    model: [[f32; 4]; 4],
    /// Inverse transpose of the model's 3×3 part, columns padded to vec4;
    /// the first's padding holds the presence.
    normal: [[f32; 4]; 3],
}

/// Mirrors `Material` in scene.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    emissive: [f32; 3],
    roughness: f32,
    uv_offset: [f32; 2],
    uv_scale: [f32; 2],
    metallic: f32,
    rim: f32,
    drawn: f32,
    face: f32,
}

pub(crate) struct DrawItem {
    pub name: Option<String>,
    /// Shown now: its node's visibility, or its own without a node, with
    /// its group on and its piece in the state holding it.
    pub visible: bool,
    pub casts_shadow: bool,
    pub kind: Kind,
    pub material: u32,
    /// World-space bounds, as its node has moved it.
    pub min: Vec3,
    pub max: Vec3,
    pub index_count: u32,
    first_index: u32,
    /// 1 drawn whole, 0 gone; between, an opaque draw dithers and a
    /// blended one fades. Negative, it dithers in the complementary pattern.
    pub presence: f32,
    own: bool,
    group_on: bool,
    state_on: bool,
    node: Option<u32>,
    base: Mat4,
    base_min: Vec3,
    base_max: Vec3,
}

impl DrawItem {
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Whether it uses a dithered pipeline: opaque and partly present.
    pub fn dithered(&self) -> bool {
        self.presence.abs() < 1.0 && self.kind.is_opaque()
    }
}

pub(crate) struct MaterialGpu {
    pub group: wgpu::BindGroup,
    pub drawn: bool,
}

struct NodeGpu {
    draws: Vec<u32>,
    visible: bool,
}

struct PieceGpu {
    states: Vec<Vec<u32>>,
    state: u32,
}

pub(crate) struct Room {
    pub draws: Vec<DrawItem>,
    pub materials: Vec<MaterialGpu>,
    pub lights: Vec<Light>,
    /// By light: whether its group, if any, is on.
    pub light_on: Vec<bool>,
    /// By light: whether the game switched it off, whatever its group.
    pub light_off: Vec<bool>,
    pub sun: Option<Sun>,
    pub ambient: [[f32; 3]; 9],
    pub background: Background,
    groups: Vec<Group>,
    nodes: Vec<NodeGpu>,
    pieces: Vec<PieceGpu>,
    /// Bounds of casters shown, hidden or moved since the shadows last took them.
    pub toggled: Vec<(Vec3, Vec3)>,
    /// Whether a light was switched on or off since the renderer last looked.
    pub lights_switched: bool,
    /// How much of it is drawn, 0 to 1.
    pub presence: f32,
    /// What its lights' intensities are multiplied by.
    pub light_scale: f32,
    vertices: wgpu::Buffer,
    /// Each mesh's indices already offset to its first vertex: WebGL2 draws
    /// with no base vertex.
    indices: wgpu::Buffer,
    draw_group: wgpu::BindGroup,
    draw_stride: u32,
    draw_buffer: wgpu::Buffer,
    /// What `draw_buffer` holds, and the draws changed since it was written.
    draw_bytes: Vec<u8>,
    dirty: Option<(usize, usize)>,
}

impl Room {
    /// Call before [`Room::draw`] when at least one draw has indices.
    pub fn bind_meshes(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
    }

    /// Draws `draws[i]` with the pipeline and material already set.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, i: usize) {
        let d = &self.draws[i];
        pass.set_bind_group(1, &self.draw_group, &[i as u32 * self.draw_stride]);
        pass.draw_indexed(d.first_index..d.first_index + d.index_count, 0, 0..1);
    }

    /// Switches group `g`'s draws and lights; false if there is none.
    pub fn set_group(&mut self, g: usize, on: bool) -> bool {
        let Some(group) = self.groups.get_mut(g) else {
            return false;
        };
        if group.on == on {
            return true;
        }
        group.on = on;
        for k in 0..self.groups[g].draws.len() {
            let d = self.groups[g].draws[k] as usize;
            self.draws[d].group_on = on;
            self.refresh(d);
        }
        for k in 0..self.groups[g].lights.len() {
            let l = self.groups[g].lights[k] as usize;
            self.light_on[l] = on;
        }
        self.lights_switched |= !self.groups[g].lights.is_empty();
        true
    }

    /// Shows state `state` of piece `p`; false if there is no such state.
    pub fn set_piece(&mut self, p: usize, state: u32) -> bool {
        let Some(piece) = self.pieces.get_mut(p) else {
            return false;
        };
        if state as usize >= piece.states.len() {
            return false;
        }
        piece.state = state;
        for s in 0..self.pieces[p].states.len() {
            let on = s as u32 == state;
            for k in 0..self.pieces[p].states[s].len() {
                let d = self.pieces[p].states[s][k] as usize;
                self.draws[d].state_on = on;
                self.refresh(d);
            }
        }
        true
    }

    /// Shows or hides node `n`; false if there is none.
    pub fn set_node_visible(&mut self, n: usize, visible: bool) -> bool {
        let Some(node) = self.nodes.get_mut(n) else {
            return false;
        };
        node.visible = visible;
        for k in 0..self.nodes[n].draws.len() {
            let d = self.nodes[n].draws[k] as usize;
            self.refresh(d);
        }
        true
    }

    /// Moves node `n`'s draws by `offset` from where the pack placed them;
    /// false if there is none.
    pub fn set_node_offset(&mut self, n: usize, offset: Mat4) -> bool {
        let Some(node) = self.nodes.get(n) else {
            return false;
        };
        for k in 0..node.draws.len() {
            let d = self.nodes[n].draws[k] as usize;
            let item = &mut self.draws[d];
            let model = offset * item.base;
            let (min, max) = moved_bounds(offset, item.base_min, item.base_max);
            if item.visible && item.casts_shadow {
                self.toggled.push((item.min, item.max));
                self.toggled.push((min, max));
            }
            item.min = min;
            item.max = max;
            self.write_draw(d, model);
        }
        true
    }

    /// Sets every draw's presence, dithered in the complementary pattern
    /// when `inverted`.
    pub fn set_presence(&mut self, presence: f32, inverted: bool) {
        self.presence = presence;
        let presence = if inverted { -presence } else { presence };
        for d in 0..self.draws.len() {
            if self.draws[d].presence != presence {
                self.draws[d].presence = presence;
                let at = d * self.draw_stride as usize + PRESENCE_OFFSET;
                self.draw_bytes[at..at + 4].copy_from_slice(&presence.to_le_bytes());
                self.mark(d);
            }
        }
    }

    /// Writes the draws changed since the last call.
    pub fn flush(&mut self, gpu: &Gpu) {
        if let Some((first, last)) = self.dirty.take() {
            let stride = self.draw_stride as usize;
            let bytes = &self.draw_bytes[first * stride..last * stride + size_of::<DrawUniform>()];
            gpu.write_at(&self.draw_buffer, (first * stride) as u64, bytes);
        }
    }

    fn refresh(&mut self, d: usize) {
        let node_visible = self.draws[d].node.map(|n| self.nodes[n as usize].visible);
        let item = &mut self.draws[d];
        let shown = node_visible.unwrap_or(item.own) && item.group_on && item.state_on;
        if shown != item.visible && item.casts_shadow {
            self.toggled.push((item.min, item.max));
        }
        item.visible = shown;
    }

    fn write_draw(&mut self, d: usize, model: Mat4) {
        let mut uniform = draw_uniform(&model.to_cols_array());
        uniform.normal[0][3] = self.draws[d].presence;
        let at = d * self.draw_stride as usize;
        self.draw_bytes[at..at + size_of::<DrawUniform>()]
            .copy_from_slice(bytemuck::bytes_of(&uniform));
        self.mark(d);
    }

    fn mark(&mut self, d: usize) {
        self.dirty = Some(match self.dirty {
            Some((first, last)) => (first.min(d), last.max(d)),
            None => (d, d),
        });
    }
}

/// Byte offset of a draw's presence in its uniform: the w of the normal
/// matrix's first column.
const PRESENCE_OFFSET: usize = 64 + 12;

/// The box around `min..max` moved by `m`.
fn moved_bounds(m: Mat4, min: Vec3, max: Vec3) -> (Vec3, Vec3) {
    if m == Mat4::IDENTITY {
        return (min, max);
    }
    let (mut lo, mut hi) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
    for i in 0..8 {
        let corner = Vec3::select(
            glam::BVec3::new(i & 1 != 0, i & 2 != 0, i & 4 != 0),
            max,
            min,
        );
        let p = m.transform_point3(corner);
        (lo, hi) = (lo.min(p), hi.max(p));
    }
    (lo, hi)
}

/// A room being uploaded a share at a time: its buffers and textures exist,
/// their contents arrive with each [`PendingRoom::upload`].
pub(crate) struct PendingRoom {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    textures: Vec<wgpu::Texture>,
    /// Each mesh's first index and first vertex.
    ranges: Vec<(u32, u32)>,
    /// Where each mesh's mirrored copy of its triangles starts, if a draw
    /// needs one.
    mirrored_first: Vec<Option<u32>>,
    next: Work,
    scratch: Vec<u32>,
    pub uploaded: u64,
    pub total: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Work {
    Mesh(usize),
    Mirrored(usize),
    /// Texture, mip level and the first row of blocks to write.
    Texture(usize, u32, u32),
    Done,
}

impl PendingRoom {
    /// Checks what `decode` leaves to the renderer and creates the room's
    /// buffers and textures, empty.
    pub fn begin(gpu: &Gpu, room: &RoomPack) -> Result<PendingRoom, RenderError> {
        let limits = gpu.device.limits();
        validate(room, &limits, gpu.device.features()).map_err(RenderError::Room)?;
        let mut ranges = Vec::with_capacity(room.meshes.len());
        let (mut indices, mut vertices) = (0u32, 0u32);
        for mesh in &room.meshes {
            ranges.push((indices, vertices));
            indices += mesh.indices.len() as u32;
            vertices += mesh.vertices.len() as u32;
        }
        // A mirroring transform reverses the winding, so its draws use a copy
        // of the mesh's triangles wound the other way.
        let mut mirrored_first: Vec<Option<u32>> = vec![None; room.meshes.len()];
        for d in room.draws.iter().filter(|d| mirrors(&d.transform)) {
            let mesh = d.mesh as usize;
            if mirrored_first[mesh].is_none() {
                mirrored_first[mesh] = Some(indices);
                indices += room.meshes[mesh].indices.len() as u32;
            }
        }
        let usage = wgpu::BufferUsages::COPY_DST;
        // A buffer of size 0 cannot be bound.
        let vertex_bytes = (vertices as u64 * size_of::<Vertex>() as u64).max(4);
        let index_bytes = (indices as u64 * 4).max(4);
        let textures: Vec<wgpu::Texture> = room
            .textures
            .iter()
            .map(|t| create_texture(gpu, t))
            .collect();
        let texture_bytes: u64 = room
            .textures
            .iter()
            .flat_map(|t| &t.mips)
            .map(|m| m.len() as u64)
            .sum();
        Ok(PendingRoom {
            vertices: gpu.buffer("vertices", vertex_bytes, usage | wgpu::BufferUsages::VERTEX),
            indices: gpu.buffer("indices", index_bytes, usage | wgpu::BufferUsages::INDEX),
            textures,
            ranges,
            mirrored_first,
            next: Work::Mesh(0),
            scratch: Vec::new(),
            uploaded: 0,
            total: vertex_bytes + index_bytes + texture_bytes,
        })
    }

    pub fn done(&self) -> bool {
        self.next == Work::Done
    }

    /// Writes the next parts of `room`, the pack [`PendingRoom::begin`] was
    /// given, until about `budget` bytes went or nothing is left: at least
    /// one part, so a budget below a part's size still makes progress.
    /// Whether everything is written.
    pub fn upload(&mut self, gpu: &Gpu, room: &RoomPack, budget: u64) -> bool {
        let mut spent = 0u64;
        self.skip_empty(room);
        while self.next != Work::Done && (spent == 0 || spent < budget) {
            let bytes = self.step(gpu, room, budget.saturating_sub(spent).max(1));
            spent += bytes;
            self.uploaded += bytes;
            self.skip_empty(room);
        }
        self.done()
    }

    /// Moves past the parts that write nothing, so that the last write
    /// leaves the upload done.
    fn skip_empty(&mut self, room: &RoomPack) {
        loop {
            self.next = match self.next {
                Work::Mesh(m) if m >= room.meshes.len() => Work::Mirrored(0),
                Work::Mirrored(m) if m >= room.meshes.len() => Work::Texture(0, 0, 0),
                Work::Mirrored(m) if self.mirrored_first[m].is_none() => Work::Mirrored(m + 1),
                Work::Texture(t, _, _) if t >= room.textures.len() => Work::Done,
                _ => return,
            };
        }
    }

    fn step(&mut self, gpu: &Gpu, room: &RoomPack, left: u64) -> u64 {
        match self.next {
            Work::Done => 0,
            Work::Mesh(m) => {
                let mesh = &room.meshes[m];
                let (first, base) = self.ranges[m];
                let vertices: &[u8] = bytemuck::cast_slice(&mesh.vertices);
                self.scratch.clear();
                self.scratch.extend(mesh.indices.iter().map(|i| base + i));
                let indices: &[u8] = bytemuck::cast_slice(&self.scratch);
                if !vertices.is_empty() {
                    gpu.write_at(
                        &self.vertices,
                        base as u64 * size_of::<Vertex>() as u64,
                        vertices,
                    );
                }
                if !indices.is_empty() {
                    gpu.write_at(&self.indices, first as u64 * 4, indices);
                }
                self.next = Work::Mesh(m + 1);
                (vertices.len() + indices.len()) as u64
            }
            Work::Mirrored(m) => {
                self.next = Work::Mirrored(m + 1);
                let Some(first) = self.mirrored_first[m] else {
                    return 0;
                };
                let base = self.ranges[m].1;
                self.scratch.clear();
                let triangles = room.meshes[m].indices.as_chunks::<3>().0;
                self.scratch.extend(
                    triangles
                        .iter()
                        .flat_map(|&[a, b, c]| [a, c, b].map(|i| base + i)),
                );
                let bytes: &[u8] = bytemuck::cast_slice(&self.scratch);
                if !bytes.is_empty() {
                    gpu.write_at(&self.indices, first as u64 * 4, bytes);
                }
                bytes.len() as u64
            }
            Work::Texture(t, level, row) => {
                let texture = &room.textures[t];
                let (written, next) = write_mip(gpu, &self.textures[t], texture, level, row, left);
                self.next = match next {
                    Some(row) => Work::Texture(t, level, row),
                    None if level as usize + 1 < texture.mips.len() => {
                        Work::Texture(t, level + 1, 0)
                    }
                    None => Work::Texture(t + 1, 0, 0),
                };
                written
            }
        }
    }

    /// The uploaded room, its materials and draws set up; call once
    /// [`PendingRoom::upload`] gave true.
    pub fn finish(self, gpu: &Gpu, layouts: &Layouts, room: &RoomPack) -> Room {
        let views: Vec<wgpu::TextureView> = self
            .textures
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .collect();
        let materials = material_groups(gpu, layouts, &room.materials, &[], &views);
        let limits = gpu.device.limits();
        let align = limits.min_uniform_buffer_offset_alignment as usize;
        let draw_stride = size_of::<DrawUniform>().next_multiple_of(align);
        let mut draw_bytes = strided(
            room.draws.iter().map(|d| draw_uniform(&d.transform)),
            draw_stride,
        );
        // A bind group cannot point into an empty buffer.
        draw_bytes.resize(draw_bytes.len().max(draw_stride), 0);
        let draw_buffer = gpu.buffer_with(
            "draws",
            &draw_bytes,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let draw_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("draws"),
            layout: &layouts.draw,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &draw_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<DrawUniform>() as u64),
                }),
            }],
        });
        let mut node_of = vec![None; room.draws.len()];
        for (n, node) in room.nodes.iter().enumerate() {
            for &d in &node.draws {
                node_of[d as usize] = Some(n as u32);
            }
        }
        let shown = room.shown_at_load();
        let mut group_on = vec![true; room.draws.len()];
        let mut light_on = vec![true; room.lights.len()];
        for g in room.groups.iter().filter(|g| !g.on) {
            g.draws.iter().for_each(|&d| group_on[d as usize] = false);
            g.lights.iter().for_each(|&l| light_on[l as usize] = false);
        }
        let mut state_on = vec![true; room.draws.len()];
        for p in &room.pieces {
            for (k, state) in p.states.iter().enumerate() {
                if k != p.state as usize {
                    state
                        .draws
                        .iter()
                        .for_each(|&d| state_on[d as usize] = false);
                }
            }
        }
        let draws = (room.draws.iter().enumerate())
            .map(|(i, d)| {
                let mesh = d.mesh as usize;
                let first_index = match self.mirrored_first[mesh] {
                    Some(first) if mirrors(&d.transform) => first,
                    _ => self.ranges[mesh].0,
                };
                let kind = Kind::of(&room.materials[d.material as usize]);
                let node_visible = node_of[i].map(|n| room.nodes[n as usize].visible);
                DrawItem {
                    name: d.name.clone(),
                    visible: shown[i] && node_visible.unwrap_or(true),
                    // A depth map holds no opacity: blended and additive surfaces block no light.
                    casts_shadow: d.casts_shadow && kind.is_opaque(),
                    kind,
                    material: d.material,
                    min: d.bounds.min.into(),
                    max: d.bounds.max.into(),
                    index_count: room.meshes[mesh].indices.len() as u32,
                    first_index,
                    presence: 1.0,
                    own: d.visible,
                    group_on: group_on[i],
                    state_on: state_on[i],
                    node: node_of[i],
                    base: Mat4::from_cols_array(&d.transform),
                    base_min: d.bounds.min.into(),
                    base_max: d.bounds.max.into(),
                }
            })
            .collect();
        Room {
            draws,
            materials,
            lights: room.lights.clone(),
            light_on,
            light_off: vec![false; room.lights.len()],
            sun: room.sun.clone(),
            ambient: room.ambient.sh9,
            background: room.background.clone(),
            groups: room.groups.clone(),
            nodes: (room.nodes.iter())
                .map(|n| NodeGpu {
                    draws: n.draws.clone(),
                    visible: n.visible,
                })
                .collect(),
            pieces: (room.pieces.iter())
                .map(|p| PieceGpu {
                    states: p.states.iter().map(|s| s.draws.clone()).collect(),
                    state: p.state,
                })
                .collect(),
            toggled: Vec::new(),
            lights_switched: false,
            presence: 1.0,
            light_scale: 1.0,
            vertices: self.vertices,
            indices: self.indices,
            draw_group,
            draw_stride: draw_stride as u32,
            draw_buffer,
            draw_bytes,
            dirty: None,
        }
    }
}

/// Checks what `decode` leaves to the renderer, then uploads everything.
pub(crate) fn upload(gpu: &Gpu, layouts: &Layouts, room: &RoomPack) -> Result<Room, RenderError> {
    let mut pending = PendingRoom::begin(gpu, room)?;
    pending.upload(gpu, room, u64::MAX);
    Ok(pending.finish(gpu, layouts, room))
}

/// A bind group per material over one uniform buffer of them; `faces[i]`,
/// where given, is the actor face material `i` shows.
pub(crate) fn material_groups(
    gpu: &Gpu,
    layouts: &Layouts,
    materials: &[Material],
    faces: &[Option<u32>],
    textures: &[wgpu::TextureView],
) -> Vec<MaterialGpu> {
    let align = gpu.device.limits().min_uniform_buffer_offset_alignment as usize;
    let stride = size_of::<MaterialUniform>().next_multiple_of(align);
    let uniforms = materials
        .iter()
        .enumerate()
        .map(|(i, m)| material_uniform(m, faces.get(i).copied().flatten()));
    let mut bytes = strided(uniforms, stride);
    // A bind group cannot point into an empty buffer.
    bytes.resize(bytes.len().max(stride), 0);
    let buffer = gpu.buffer_with("materials", &bytes, wgpu::BufferUsages::UNIFORM);
    materials
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let texture = |t: Option<u32>| t.map_or(&layouts.white, |t| &textures[t as usize]);
            let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&m.name),
                layout: &layouts.material,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &buffer,
                            offset: (i * stride) as u64,
                            size: wgpu::BufferSize::new(size_of::<MaterialUniform>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(texture(m.base_color_texture)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(texture(m.emissive_texture)),
                    },
                ],
            });
            MaterialGpu {
                group,
                drawn: m.drawn,
            }
        })
        .collect()
}

fn validate(
    room: &RoomPack,
    limits: &wgpu::Limits,
    features: wgpu::Features,
) -> Result<(), String> {
    for (i, mesh) in room.meshes.iter().enumerate() {
        let count = mesh.indices.len();
        if !count.is_multiple_of(3) {
            return Err(format!("mesh {i} has {count} indices, not whole triangles"));
        }
        let vertices = mesh.vertices.len();
        if let Some(index) = mesh.indices.iter().find(|&&ix| ix as usize >= vertices) {
            return Err(format!(
                "mesh {i} uses vertex {index} but has {vertices} vertices"
            ));
        }
    }
    let total = |per_mesh: fn(&sariema_format::Mesh) -> usize, size: usize| {
        room.meshes.iter().map(per_mesh).sum::<usize>() as u64 * size as u64
    };
    let vertex_bytes = total(|m| m.vertices.len(), size_of::<Vertex>());
    let index_bytes = total(|m| m.indices.len(), size_of::<u32>());
    if vertex_bytes.max(index_bytes) > limits.max_buffer_size {
        return Err(format!(
            "its meshes take {vertex_bytes} vertex and {index_bytes} index bytes; the device allows {} per buffer",
            limits.max_buffer_size
        ));
    }
    for (i, texture) in room.textures.iter().enumerate() {
        if let Some(why) = texture_problem(texture, limits.max_texture_dimension_2d, features) {
            return Err(format!("texture {i} ({}) {why}", texture.name));
        }
    }
    let textures = room.textures.len();
    for (i, m) in room.materials.iter().enumerate() {
        let used = [m.base_color_texture, m.emissive_texture];
        if let Some(t) = used.into_iter().flatten().find(|&t| t as usize >= textures) {
            return Err(format!("material {i} uses texture {t} of {textures}"));
        }
    }
    for (i, d) in room.draws.iter().enumerate() {
        if d.mesh as usize >= room.meshes.len() || d.material as usize >= room.materials.len() {
            return Err(format!(
                "draw {i} uses mesh {} of {} and material {} of {}",
                d.mesh,
                room.meshes.len(),
                d.material,
                room.materials.len()
            ));
        }
    }
    Ok(())
}

fn gpu_format(format: TextureFormat) -> wgpu::TextureFormat {
    match format {
        TextureFormat::Rgba8Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        TextureFormat::Astc4x4Srgb => wgpu::TextureFormat::Astc {
            block: wgpu::AstcBlock::B4x4,
            channel: wgpu::AstcChannel::UnormSrgb,
        },
    }
}

/// One texel block of a texture format, in texels and bytes; uncompressed
/// formats have 1×1 blocks.
struct Block {
    width: u32,
    height: u32,
    bytes: u32,
}

impl Block {
    fn of(format: wgpu::TextureFormat) -> Block {
        let (width, height) = format.block_dimensions();
        let bytes = format
            .block_copy_size(None)
            .expect("colour formats have one aspect");
        Block {
            width,
            height,
            bytes,
        }
    }
}

pub(crate) fn texture_problem(
    t: &Texture,
    max_side: u32,
    features: wgpu::Features,
) -> Option<String> {
    let format = gpu_format(t.format);
    if !features.contains(format.required_features()) {
        return Some(format!(
            r#"is {:?}, which this device cannot sample; set "textures": "rgba8" in the room description"#,
            t.format
        ));
    }
    let (w, h) = (t.width, t.height);
    if w == 0 || h == 0 || w > max_side || h > max_side {
        return Some(format!(
            "is {w}×{h}; the device takes 1 to {max_side} per side"
        ));
    }
    let block = Block::of(format);
    if !(w.is_multiple_of(block.width) && h.is_multiple_of(block.height)) {
        return Some(format!(
            "is {w}×{h}, which is not whole {}×{} blocks",
            block.width, block.height
        ));
    }
    let levels = 32 - w.max(h).leading_zeros() as usize;
    if t.mips.is_empty() || t.mips.len() > levels {
        return Some(format!(
            "has {} mip levels where a {w}×{h} texture takes 1 to {levels}",
            t.mips.len()
        ));
    }
    t.mips.iter().enumerate().find_map(|(level, mip)| {
        let blocks = |side: u32, block: u32| (side >> level).max(1).div_ceil(block) as usize;
        let expected = blocks(w, block.width) * blocks(h, block.height) * block.bytes as usize;
        (mip.len() != expected).then(|| {
            format!(
                "has {} bytes in mip {level} instead of {expected}",
                mip.len()
            )
        })
    })
}

/// An empty texture for `t`, which must have passed [`texture_problem`].
pub(crate) fn create_texture(gpu: &Gpu, t: &Texture) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&t.name),
        size: mip_size(t, 0),
        mip_level_count: t.mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: gpu_format(t.format),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn mip_size(t: &Texture, level: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: (t.width >> level).max(1),
        height: (t.height >> level).max(1),
        depth_or_array_layers: 1,
    }
}

/// Writes rows of blocks of mip `level` of `t` from `row` on, as many as
/// fit in `budget` bytes (one at least); the bytes written and the next row,
/// `None` once the mip is whole.
fn write_mip(
    gpu: &Gpu,
    texture: &wgpu::Texture,
    t: &Texture,
    level: u32,
    row: u32,
    budget: u64,
) -> (u64, Option<u32>) {
    let format = gpu_format(t.format);
    let block = Block::of(format);
    // A copy covers whole blocks, also for mips smaller than one.
    let extent = mip_size(t, level).physical_size(format);
    let row_bytes = extent.width / block.width * block.bytes;
    let block_rows = extent.height / block.height;
    let rows = ((budget / row_bytes as u64).max(1) as u32).min(block_rows - row);
    let mip = &t.mips[level as usize];
    let start = (row * row_bytes) as usize;
    let bytes = &mip[start..start + (rows * row_bytes) as usize];
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d {
                x: 0,
                y: row * block.height,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row_bytes),
            rows_per_image: Some(rows),
        },
        wgpu::Extent3d {
            width: extent.width,
            height: rows * block.height,
            depth_or_array_layers: 1,
        },
    );
    let next = row + rows;
    (bytes.len() as u64, (next < block_rows).then_some(next))
}

/// `t` must have passed [`texture_problem`].
pub(crate) fn upload_texture(gpu: &Gpu, t: &Texture) -> wgpu::TextureView {
    let texture = create_texture(gpu, t);
    for level in 0..t.mips.len() as u32 {
        write_mip(gpu, &texture, t, level, 0, u64::MAX);
    }
    texture.create_view(&Default::default())
}

/// The items' bytes, each starting at a multiple of `stride`.
pub(crate) fn strided<T: Pod>(items: impl Iterator<Item = T>, stride: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for item in items {
        bytes.extend_from_slice(bytemuck::bytes_of(&item));
        bytes.resize(bytes.len().next_multiple_of(stride), 0);
    }
    bytes
}

fn material_uniform(m: &Material, face: Option<u32>) -> MaterialUniform {
    MaterialUniform {
        base_color: m.base_color,
        emissive: m.emissive,
        roughness: m.roughness,
        uv_offset: m.uv_offset,
        uv_scale: m.uv_scale,
        metallic: m.metallic,
        rim: m.rim,
        drawn: m.drawn as u8 as f32,
        face: face.map_or(-1.0, |f| f as f32),
    }
}

fn mirrors(transform: &[f32; 16]) -> bool {
    Mat3::from_mat4(Mat4::from_cols_array(transform)).determinant() < 0.0
}

fn draw_uniform(transform: &[f32; 16]) -> DrawUniform {
    let model = Mat4::from_cols_array(transform);
    let normal = Mat3::from_mat4(model).inverse().transpose();
    let mut normal: [[f32; 4]; 3] =
        [normal.x_axis, normal.y_axis, normal.z_axis].map(|c| c.extend(0.0).into());
    normal[0][3] = 1.0;
    DrawUniform {
        model: model.to_cols_array_2d(),
        normal,
    }
}
