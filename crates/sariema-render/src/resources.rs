// SPDX-License-Identifier: Apache-2.0

//! A room's meshes, textures, materials and draws on the GPU.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use sariema_format::{Background, Light, Material, RoomPack, Sun, Texture, TextureFormat, Vertex};

use crate::scene::{Kind, Layouts};
use crate::{Gpu, RenderError};

/// Mirrors `Draw` in common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct DrawUniform {
    model: [[f32; 4]; 4],
    /// Inverse transpose of the model's 3×3 part, columns padded to vec4.
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
    _pad: f32,
}

pub(crate) struct DrawItem {
    pub name: Option<String>,
    pub visible: bool,
    pub casts_shadow: bool,
    pub kind: Kind,
    pub material: u32,
    /// World-space bounds.
    pub min: Vec3,
    pub max: Vec3,
    pub index_count: u32,
    first_index: u32,
    base_vertex: i32,
}

impl DrawItem {
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
}

pub(crate) struct MaterialGpu {
    pub group: wgpu::BindGroup,
    pub drawn: bool,
}

pub(crate) struct Room {
    pub draws: Vec<DrawItem>,
    pub materials: Vec<MaterialGpu>,
    pub lights: Vec<Light>,
    pub sun: Option<Sun>,
    pub ambient: [[f32; 3]; 9],
    pub background: Background,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    draw_group: wgpu::BindGroup,
    draw_stride: u32,
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
        pass.draw_indexed(
            d.first_index..d.first_index + d.index_count,
            d.base_vertex,
            0..1,
        );
    }
}

/// Checks what `decode` leaves to the renderer, then uploads everything.
pub(crate) fn upload(gpu: &Gpu, layouts: &Layouts, room: &RoomPack) -> Result<Room, RenderError> {
    let limits = gpu.device.limits();
    validate(room, &limits, gpu.device.features()).map_err(RenderError::Room)?;

    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut ranges = Vec::with_capacity(room.meshes.len());
    for mesh in &room.meshes {
        ranges.push((indices.len() as u32, vertices.len() as i32));
        vertices.extend_from_slice(&mesh.vertices);
        indices.extend_from_slice(&mesh.indices);
    }
    // A mirroring transform reverses the winding, so its draws use a copy of
    // the mesh's triangles wound the other way.
    let mut mirrored_first: Vec<Option<u32>> = vec![None; room.meshes.len()];
    for d in room.draws.iter().filter(|d| mirrors(&d.transform)) {
        let mesh = d.mesh as usize;
        if mirrored_first[mesh].is_none() {
            mirrored_first[mesh] = Some(indices.len() as u32);
            let triangles = room.meshes[mesh].indices.as_chunks::<3>().0;
            indices.extend(triangles.iter().flat_map(|&[a, b, c]| [a, c, b]));
        }
    }
    let vertices = gpu.buffer_with(
        "vertices",
        bytemuck::cast_slice(&vertices),
        wgpu::BufferUsages::VERTEX,
    );
    let indices = gpu.buffer_with(
        "indices",
        bytemuck::cast_slice(&indices),
        wgpu::BufferUsages::INDEX,
    );

    let textures: Vec<wgpu::TextureView> = room
        .textures
        .iter()
        .map(|t| upload_texture(gpu, t))
        .collect();
    let align = limits.min_uniform_buffer_offset_alignment as usize;
    let material_stride = size_of::<MaterialUniform>().next_multiple_of(align);
    let material_buffer = gpu.buffer_with(
        "materials",
        &strided(room.materials.iter().map(material_uniform), material_stride),
        wgpu::BufferUsages::UNIFORM,
    );
    let materials = room
        .materials
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
                            buffer: &material_buffer,
                            offset: (i * material_stride) as u64,
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
        .collect();

    let draw_stride = size_of::<DrawUniform>().next_multiple_of(align);
    let mut draw_bytes = strided(
        room.draws.iter().map(|d| draw_uniform(&d.transform)),
        draw_stride,
    );
    // A bind group cannot point into an empty buffer.
    draw_bytes.resize(draw_bytes.len().max(draw_stride), 0);
    let draw_buffer = gpu.buffer_with("draws", &draw_bytes, wgpu::BufferUsages::UNIFORM);
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
    let draws = room
        .draws
        .iter()
        .map(|d| {
            let (first_index, base_vertex) = ranges[d.mesh as usize];
            let first_index = match mirrored_first[d.mesh as usize] {
                Some(first) if mirrors(&d.transform) => first,
                _ => first_index,
            };
            let kind = Kind::of(&room.materials[d.material as usize]);
            DrawItem {
                name: d.name.clone(),
                visible: d.visible,
                // Glows add light and block none.
                casts_shadow: d.casts_shadow && kind != Kind::Additive,
                kind,
                material: d.material,
                min: d.bounds.min.into(),
                max: d.bounds.max.into(),
                index_count: room.meshes[d.mesh as usize].indices.len() as u32,
                first_index,
                base_vertex,
            }
        })
        .collect();

    Ok(Room {
        draws,
        materials,
        lights: room.lights.clone(),
        sun: room.sun.clone(),
        ambient: room.ambient.sh9,
        background: room.background.clone(),
        vertices,
        indices,
        draw_group,
        draw_stride: draw_stride as u32,
    })
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

/// Texel blocks of `format`: width, height and bytes.
fn block(format: wgpu::TextureFormat) -> (u32, u32, u32) {
    let (width, height) = format.block_dimensions();
    let bytes = format
        .block_copy_size(None)
        .expect("colour formats have one aspect");
    (width, height, bytes)
}

fn texture_problem(t: &Texture, max_side: u32, features: wgpu::Features) -> Option<String> {
    let format = gpu_format(t.format);
    if !features.contains(format.required_features()) {
        return Some(format!(
            "is {:?}, which this device cannot sample; pack the room with --textures rgba8",
            t.format
        ));
    }
    let (w, h) = (t.width, t.height);
    if w == 0 || h == 0 || w > max_side || h > max_side {
        return Some(format!(
            "is {w}×{h}; the device takes 1 to {max_side} per side"
        ));
    }
    let (block_w, block_h, block_bytes) = block(format);
    if !(w.is_multiple_of(block_w) && h.is_multiple_of(block_h)) {
        return Some(format!(
            "is {w}×{h}, which is not whole {block_w}×{block_h} blocks"
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
        let expected = blocks(w, block_w) * blocks(h, block_h) * block_bytes as usize;
        (mip.len() != expected).then(|| {
            format!(
                "has {} bytes in mip {level} instead of {expected}",
                mip.len()
            )
        })
    })
}

/// `t` must have passed [`texture_problem`].
fn upload_texture(gpu: &Gpu, t: &Texture) -> wgpu::TextureView {
    let format = gpu_format(t.format);
    let (block_w, block_h, block_bytes) = block(format);
    let size = |level: u32| wgpu::Extent3d {
        width: (t.width >> level).max(1),
        height: (t.height >> level).max(1),
        depth_or_array_layers: 1,
    };
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&t.name),
        size: size(0),
        mip_level_count: t.mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, mip) in (0..).zip(&t.mips) {
        // A copy covers whole blocks, also for mips smaller than one.
        let extent = size(level).physical_size(format);
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            mip,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(extent.width / block_w * block_bytes),
                rows_per_image: Some(extent.height / block_h),
            },
            extent,
        );
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

fn material_uniform(m: &Material) -> MaterialUniform {
    MaterialUniform {
        base_color: m.base_color,
        emissive: m.emissive,
        roughness: m.roughness,
        uv_offset: m.uv_offset,
        uv_scale: m.uv_scale,
        metallic: m.metallic,
        rim: m.rim,
        drawn: m.drawn as u8 as f32,
        _pad: 0.0,
    }
}

fn mirrors(transform: &[f32; 16]) -> bool {
    Mat3::from_mat4(Mat4::from_cols_array(transform)).determinant() < 0.0
}

fn draw_uniform(transform: &[f32; 16]) -> DrawUniform {
    let model = Mat4::from_cols_array(transform);
    let normal = Mat3::from_mat4(model).inverse().transpose();
    DrawUniform {
        model: model.to_cols_array_2d(),
        normal: [normal.x_axis, normal.y_axis, normal.z_axis].map(|c| c.extend(0.0).into()),
    }
}
