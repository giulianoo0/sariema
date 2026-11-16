// SPDX-License-Identifier: Apache-2.0

//! Actors: skinned packs lit and shadowed like the room. A loaded pack is an
//! [`ActorModel`]; each [`ActorId`] shows one model with its own pose, face
//! cells and part visibility. Every actor's frame data lives in one buffer,
//! written once a frame, which each actor's bind group views: a storage
//! buffer, or on the downlevel path a uniform buffer viewed
//! `Quality::actor_uniform_rows` rows at a time, as Bevy and Filament bind
//! their joint palettes where storage buffers are missing.

use glam::{Mat3, Mat4, Vec2, Vec3};
use sariema_core::Camera;
use sariema_format::actor::{ActorPack, FaceAtlas, SkinnedVertex};

use crate::resources::{self, MaterialGpu};
use crate::scene::{Kind, Layouts};
use crate::{Gpu, Quality, RenderError, RenderPath, cull};

mod draw;
mod pipelines;

pub(crate) use draw::Layer;
use pipelines::Pipelines;

/// A loaded actor pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActorModel(u32);

/// One actor showing an [`ActorModel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActorId(u32);

/// Where an actor is and how it is posed in a frame.
#[derive(Clone, Copy, Debug)]
pub struct ActorState<'a> {
    /// The actor's model space to the world. A mirroring matrix shows the
    /// inside of its meshes.
    pub model: Mat4,
    /// One matrix per joint of the pack, in its order: the joint's model
    /// space times its inverse bind matrix, as `sariema_anim` computes it.
    pub palette: &'a [Mat4],
    /// The atlas cell each of the pack's faces shows, in their order.
    pub faces: &'a [u32],
    pub visible: bool,
}

/// A soft dark disc on the plane through an actor's origin, drawn when no
/// shadow map holds the actor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiscShadow {
    /// Before the model matrix's scale.
    pub radius_m: f32,
    /// Darkening at the centre, 0 to 1.
    pub opacity: f32,
    /// Fraction of the radius, from the rim inwards, over which it fades out.
    pub softness: f32,
    /// Height above the origin, keeping the disc off the floor it lies on.
    pub lift_m: f32,
}

impl DiscShadow {
    /// For an actor `height_m` tall: a radius of 0.22 × the height, opacity
    /// 0.5, softness 0.6 and a lift of 1 cm.
    pub fn for_height(height_m: f32) -> DiscShadow {
        DiscShadow {
            radius_m: 0.22 * height_m.max(0.0),
            opacity: 0.5,
            softness: 0.6,
            lift_m: 0.01,
        }
    }
}

/// Rows of four floats at the start of every actor's record, before its faces.
const HEADER_ROWS: usize = 8;
const ROW_BYTES: usize = 16;

#[derive(Clone, Copy, Debug)]
struct PartGpu {
    first_index: u32,
    index_count: u32,
    material: u32,
    kind: Kind,
    casts_shadow: bool,
}

struct ModelGpu {
    vertices: wgpu::Buffer,
    /// Each mesh's indices already offset to its first vertex: WebGL2 draws
    /// with no base vertex.
    indices: wgpu::Buffer,
    parts: Vec<PartGpu>,
    part_names: Vec<String>,
    shown: Vec<bool>,
    materials: Vec<MaterialGpu>,
    joints: usize,
    faces: Vec<FaceAtlas>,
    bounds: (Vec3, Vec3),
    disc: DiscShadow,
    /// Rows in one actor's record and the rows it takes in the buffer.
    rows: usize,
    stride: usize,
}

impl ModelGpu {
    fn palette_row(&self) -> usize {
        HEADER_ROWS + self.faces.len().div_ceil(2)
    }

    fn casts_shadow(&self) -> bool {
        self.parts.iter().any(|p| p.casts_shadow)
    }
}

struct Instance {
    model: usize,
    /// First row of its record in the buffer.
    row: usize,
    group: Option<wgpu::BindGroup>,
    placed: bool,
    visible: bool,
    shown: Vec<bool>,
    disc: Option<DiscShadow>,
    bounds: (Vec3, Vec3),
}

impl Instance {
    fn drawn(&self) -> bool {
        self.placed && self.visible
    }
}

/// What [`Actors::prepare`] found for a frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ActorStats {
    pub drawn: u32,
    pub culled: u32,
}

pub(crate) struct Actors {
    layout: wgpu::BindGroupLayout,
    pipelines: Option<Pipelines>,
    models: Vec<ModelGpu>,
    instances: Vec<Instance>,
    /// The buffer's contents, `rows.len()` rows.
    rows: Vec<[f32; 4]>,
    buffer: Option<wgpu::Buffer>,
    capacity_rows: usize,
    align_rows: usize,
    /// Rows each bind group views on the downlevel path; `None` on the full
    /// path, where it views the actor's own record.
    uniform_rows: Option<usize>,
    dirty: bool,
    /// World bounds, before and after, of the casting actors changed since
    /// the shadows last looked.
    moved: Vec<(Vec3, Vec3)>,
    frustum_culling: bool,
    casts_into_maps: bool,
    /// The frame's actors in view, and those of them drawing a disc.
    in_view: Vec<u32>,
    discs: Vec<u32>,
}

impl Actors {
    pub fn new(gpu: &Gpu, quality: &Quality) -> Actors {
        let limits = gpu.device.limits();
        let uniform_rows =
            (quality.path == RenderPath::Downlevel).then_some(quality.actor_uniform_rows as usize);
        let (ty, align) = match uniform_rows {
            Some(rows) => (
                wgpu::BufferBindingType::Uniform,
                (
                    limits.min_uniform_buffer_offset_alignment,
                    (rows * ROW_BYTES) as u64,
                ),
            ),
            None => (
                wgpu::BufferBindingType::Storage { read_only: true },
                (limits.min_storage_buffer_offset_alignment, 0),
            ),
        };
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("actor"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(align.1),
                    },
                    count: None,
                }],
            });
        let align = align.0 as usize;
        Actors {
            layout,
            pipelines: None,
            models: Vec::new(),
            instances: Vec::new(),
            rows: Vec::new(),
            buffer: None,
            capacity_rows: 0,
            align_rows: align.div_ceil(ROW_BYTES).max(1),
            uniform_rows,
            dirty: false,
            moved: Vec::new(),
            frustum_culling: quality.frustum_culling,
            casts_into_maps: quality.actor_shadow_slots > 0,
            in_view: Vec::new(),
            discs: Vec::new(),
        }
    }

    /// Uploads `pack` and creates the pipelines its materials need.
    pub fn load(
        &mut self,
        gpu: &Gpu,
        layouts: &Layouts,
        shadow_view: &wgpu::BindGroupLayout,
        quality: &Quality,
        pack: &ActorPack,
    ) -> Result<ActorModel, RenderError> {
        let limits = gpu.device.limits();
        validate(pack, &limits, gpu.device.features(), self.uniform_rows)
            .map_err(RenderError::Actor)?;
        if self.pipelines.is_none() {
            self.pipelines = Some(Pipelines::new(
                gpu,
                layouts,
                &self.layout,
                shadow_view,
                quality,
                quality.single_depth_pass(),
            )?);
        }
        let pipelines = self.pipelines.as_mut().expect("created above");
        let mut vertices: Vec<SkinnedVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut firsts = Vec::with_capacity(pack.meshes.len());
        for mesh in &pack.meshes {
            firsts.push(indices.len() as u32);
            let base = vertices.len() as u32;
            vertices.extend_from_slice(&mesh.vertices);
            indices.extend(mesh.indices.iter().map(|i| base + i));
        }
        // A buffer cannot be empty.
        let vertex_bytes: &[u8] = bytemuck::cast_slice(&vertices);
        let index_bytes: &[u8] = bytemuck::cast_slice(&indices);
        let vertices = gpu.buffer_with(
            "actor vertices",
            if vertex_bytes.is_empty() {
                &[0; 4]
            } else {
                vertex_bytes
            },
            wgpu::BufferUsages::VERTEX,
        );
        let indices = gpu.buffer_with(
            "actor indices",
            if index_bytes.is_empty() {
                &[0; 4]
            } else {
                index_bytes
            },
            wgpu::BufferUsages::INDEX,
        );
        let textures: Vec<wgpu::TextureView> = pack
            .textures
            .iter()
            .map(|t| resources::upload_texture(gpu, t))
            .collect();
        let faces_of: Vec<Option<u32>> = (0..pack.materials.len() as u32)
            .map(|m| {
                (pack.faces.iter())
                    .position(|f| f.materials.contains(&m))
                    .map(|f| f as u32)
            })
            .collect();
        let materials =
            resources::material_groups(gpu, layouts, &pack.materials, &faces_of, &textures);
        let parts: Vec<PartGpu> = pack
            .parts
            .iter()
            .map(|p| {
                let kind = Kind::of(&pack.materials[p.material as usize]);
                PartGpu {
                    first_index: firsts[p.mesh as usize],
                    index_count: pack.meshes[p.mesh as usize].indices.len() as u32,
                    material: p.material,
                    kind,
                    casts_shadow: p.casts_shadow && kind.is_opaque(),
                }
            })
            .collect();
        for part in &parts {
            pipelines.ensure(gpu, part.kind, quality);
        }
        let joints = pack.joints.len();
        let rows = HEADER_ROWS + pack.faces.len().div_ceil(2) + 4 * joints;
        self.models.push(ModelGpu {
            vertices,
            indices,
            parts,
            part_names: pack.parts.iter().map(|p| p.name.clone()).collect(),
            shown: pack.parts.iter().map(|p| p.visible).collect(),
            materials,
            joints,
            faces: pack.faces.clone(),
            bounds: (pack.bounds.min.into(), pack.bounds.max.into()),
            disc: DiscShadow::for_height(bind_height(pack)),
            rows,
            stride: rows.next_multiple_of(self.align_rows),
        });
        Ok(ActorModel(self.models.len() as u32 - 1))
    }

    /// A hidden actor showing `model`, with its pack's part visibility and a
    /// [`DiscShadow::for_height`] its bind pose, until [`Actors::set`] places it.
    pub fn spawn(&mut self, gpu: &Gpu, model: ActorModel) -> Result<ActorId, RenderError> {
        let index = model.0 as usize;
        let m = self
            .models
            .get(index)
            .ok_or_else(|| RenderError::Actor(format!("no actor model {}", model.0)))?;
        let row = self.rows.len();
        let mut rows = vec![[0.0; 4]; m.stride];
        rows[0..4].copy_from_slice(&Mat4::IDENTITY.to_cols_array_2d());
        self.rows.extend_from_slice(&rows);
        self.instances.push(Instance {
            model: index,
            row,
            group: None,
            placed: false,
            visible: false,
            shown: m.shown.clone(),
            disc: Some(m.disc),
            bounds: m.bounds,
        });
        let id = self.instances.len() - 1;
        self.write_header(id);
        let m = &self.models[index];
        for (f, face) in m.faces.iter().enumerate() {
            let row = &mut self.rows[row + HEADER_ROWS + f / 2];
            row[(f % 2) * 2..(f % 2) * 2 + 2]
                .copy_from_slice(&face_offset(face, face.start).to_array());
        }
        if self.rows.len() > self.capacity_rows {
            self.grow(gpu);
        } else {
            self.instances[id].group = Some(self.group(gpu, id));
        }
        self.dirty = true;
        Ok(ActorId(id as u32))
    }

    /// A buffer twice the size needed, and every actor's view into it. A
    /// uniform view spans `uniform_rows` whatever the record's size, so the
    /// buffer holds that many more past its last record.
    fn grow(&mut self, gpu: &Gpu) {
        self.capacity_rows = (self.rows.len() * 2).max(self.align_rows);
        let usage = match self.uniform_rows {
            Some(_) => wgpu::BufferUsages::UNIFORM,
            None => wgpu::BufferUsages::STORAGE,
        };
        let rows = self.capacity_rows + self.uniform_rows.unwrap_or(0);
        self.buffer = Some(gpu.buffer(
            "actors",
            (rows * ROW_BYTES) as u64,
            usage | wgpu::BufferUsages::COPY_DST,
        ));
        for id in 0..self.instances.len() {
            self.instances[id].group = Some(self.group(gpu, id));
        }
    }

    fn group(&self, gpu: &Gpu, id: usize) -> wgpu::BindGroup {
        let instance = &self.instances[id];
        let rows = (self.uniform_rows).unwrap_or(self.models[instance.model].rows);
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("actor"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: self.buffer.as_ref().expect("grown before"),
                    offset: (instance.row * ROW_BYTES) as u64,
                    size: wgpu::BufferSize::new((rows * ROW_BYTES) as u64),
                }),
            }],
        })
    }

    /// Row 4's w and row 7: where the palette starts and the disc.
    fn write_header(&mut self, id: usize) {
        let instance = &self.instances[id];
        let model = &self.models[instance.model];
        let record = &mut self.rows[instance.row..instance.row + model.rows];
        record[4][3] = model.palette_row() as f32;
        record[7] = match instance.disc {
            Some(d) => [d.radius_m, d.opacity, d.softness, d.lift_m],
            None => [0.0; 4],
        };
    }

    fn instance(&self, id: ActorId) -> Result<usize, RenderError> {
        let index = id.0 as usize;
        if index < self.instances.len() {
            Ok(index)
        } else {
            Err(RenderError::Actor(format!("no actor {}", id.0)))
        }
    }

    pub fn set(&mut self, id: ActorId, state: &ActorState) -> Result<(), RenderError> {
        let index = self.instance(id)?;
        let instance = &self.instances[index];
        let model = &self.models[instance.model];
        if state.palette.len() != model.joints {
            return Err(RenderError::Actor(format!(
                "actor {} has {} joints and was given {} palette matrices",
                id.0,
                model.joints,
                state.palette.len()
            )));
        }
        if state.faces.len() > model.faces.len() {
            return Err(RenderError::Actor(format!(
                "actor {} has {} faces and was given {} cells",
                id.0,
                model.faces.len(),
                state.faces.len()
            )));
        }
        if let Some((face, &cell)) = (model.faces.iter())
            .zip(state.faces)
            .find(|(f, c)| **c >= f.columns * f.rows)
        {
            return Err(RenderError::Actor(format!(
                "face {} has {} cells and was given cell {cell}",
                face.name,
                face.columns * face.rows
            )));
        }
        let before = (instance.drawn(), instance.bounds);
        let record = &mut self.rows[instance.row..instance.row + model.rows];
        record[0..4].copy_from_slice(&state.model.to_cols_array_2d());
        let normal = Mat3::from_mat4(state.model).inverse().transpose();
        for (row, column) in
            record[4..7]
                .iter_mut()
                .zip([normal.x_axis, normal.y_axis, normal.z_axis])
        {
            row[..3].copy_from_slice(&column.to_array());
        }
        for (f, (face, &cell)) in model.faces.iter().zip(state.faces).enumerate() {
            let offset = face_offset(face, cell);
            let row = &mut record[HEADER_ROWS + f / 2];
            row[(f % 2) * 2..(f % 2) * 2 + 2].copy_from_slice(&offset.to_array());
        }
        let palette = &mut record[model.palette_row()..];
        for (rows, m) in palette.as_chunks_mut::<4>().0.iter_mut().zip(state.palette) {
            *rows = m.to_cols_array_2d();
        }
        let bounds = cull::transformed_box(state.model, model.bounds.0, model.bounds.1);
        let casts = model.casts_shadow();
        let instance = &mut self.instances[index];
        instance.placed = true;
        instance.visible = state.visible;
        instance.bounds = bounds;
        if casts {
            if before.0 {
                self.moved.push(before.1);
            }
            if instance.drawn() {
                self.moved.push(bounds);
            }
        }
        self.dirty = true;
        Ok(())
    }

    /// Shows or hides the actor's parts called `name`; false if it has none.
    pub fn set_part_visible(&mut self, id: ActorId, name: &str, visible: bool) -> bool {
        let Ok(index) = self.instance(id) else {
            return false;
        };
        let instance = &mut self.instances[index];
        let model = &self.models[instance.model];
        let mut found = false;
        for (i, part) in model.part_names.iter().enumerate() {
            if part == name {
                if model.parts[i].casts_shadow && instance.shown[i] != visible && instance.drawn() {
                    self.moved.push(instance.bounds);
                }
                instance.shown[i] = visible;
                found = true;
            }
        }
        found
    }

    pub fn set_disc(&mut self, id: ActorId, disc: Option<DiscShadow>) -> Result<(), RenderError> {
        let index = self.instance(id)?;
        self.instances[index].disc = disc;
        self.write_header(index);
        self.dirty = true;
        Ok(())
    }

    /// Writes the frame data, if anything changed.
    pub fn upload(&mut self, gpu: &Gpu) {
        if !self.dirty {
            return;
        }
        if let Some(buffer) = &self.buffer {
            gpu.write(buffer, bytemuck::cast_slice(&self.rows));
        }
        self.dirty = false;
    }

    /// Bounds of the casting actors that moved, appeared or vanished since
    /// [`Actors::forget_moves`].
    pub fn moved(&self) -> &[(Vec3, Vec3)] {
        &self.moved
    }

    pub fn forget_moves(&mut self) {
        self.moved.clear();
    }

    /// Picks the frame's actors in view, and those of them without a shadow
    /// map holding them, which `mapped` tells from their bounds.
    pub fn prepare(
        &mut self,
        camera: &Camera,
        mapped: impl Fn((Vec3, Vec3)) -> bool,
    ) -> ActorStats {
        let planes = cull::planes(camera.proj * camera.view);
        let mut stats = ActorStats::default();
        self.in_view.clear();
        self.discs.clear();
        for (i, instance) in self.instances.iter().enumerate() {
            if !instance.drawn() {
                continue;
            }
            let (min, max) = instance.bounds;
            if self.frustum_culling && !cull::box_visible(&planes, min, max) {
                stats.culled += 1;
                continue;
            }
            stats.drawn += 1;
            self.in_view.push(i as u32);
            let model = &self.models[instance.model];
            let in_a_map = self.casts_into_maps && model.casts_shadow() && mapped(instance.bounds);
            if instance.disc.is_some() && !in_a_map {
                self.discs.push(i as u32);
            }
        }
        stats
    }
}

/// Height of the visible parts in the bind pose, which no clip stretches.
fn bind_height(pack: &ActorPack) -> f32 {
    let (low, high) = (pack.parts.iter())
        .filter(|p| p.visible)
        .flat_map(|p| &pack.meshes[p.mesh as usize].vertices)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v.pos[1]), hi.max(v.pos[1]))
        });
    (high - low).max(0.0)
}

/// `origin + (cell % columns, cell / columns) × step`.
fn face_offset(face: &FaceAtlas, cell: u32) -> Vec2 {
    let at = Vec2::new((cell % face.columns) as f32, (cell / face.columns) as f32);
    Vec2::from(face.origin) + at * Vec2::from(face.step)
}

/// Checks what the device must hold of `pack`, after the format's own checks;
/// `uniform_rows` is the downlevel path's record size.
fn validate(
    pack: &ActorPack,
    limits: &wgpu::Limits,
    features: wgpu::Features,
    uniform_rows: Option<usize>,
) -> Result<(), String> {
    sariema_format::actor::validate(pack)?;
    let vertices: usize = pack.meshes.iter().map(|m| m.vertices.len()).sum();
    let indices: usize = pack.meshes.iter().map(|m| m.indices.len()).sum();
    let vertex_bytes = (vertices * size_of::<SkinnedVertex>()) as u64;
    let index_bytes = (indices * size_of::<u32>()) as u64;
    if vertex_bytes.max(index_bytes) > limits.max_buffer_size {
        return Err(format!(
            "its meshes take {vertex_bytes} vertex and {index_bytes} index bytes; the device allows {} per buffer",
            limits.max_buffer_size
        ));
    }
    let rows = HEADER_ROWS + pack.faces.len().div_ceil(2) + 4 * pack.joints.len();
    if let Some(most) = uniform_rows.filter(|&most| rows > most) {
        return Err(format!(
            "its {} joints and {} faces take {rows} rows a frame; the downlevel path binds \
             Quality::actor_uniform_rows, {most}",
            pack.joints.len(),
            pack.faces.len()
        ));
    }
    let record = (rows * ROW_BYTES) as u64;
    let binding = limits
        .max_storage_buffer_binding_size
        .min(limits.max_buffer_size);
    if uniform_rows.is_none() && record > binding {
        return Err(format!(
            "its {} joints take {record} bytes a frame; the device binds at most {binding}",
            pack.joints.len()
        ));
    }
    for (i, texture) in pack.textures.iter().enumerate() {
        if let Some(why) =
            resources::texture_problem(texture, limits.max_texture_dimension_2d, features)
        {
            return Err(format!("texture {i} ({}) {why}", texture.name));
        }
    }
    Ok(())
}
