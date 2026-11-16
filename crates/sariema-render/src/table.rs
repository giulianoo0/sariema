// SPDX-License-Identifier: Apache-2.0

//! The arrays the scene shader reads each frame: storage buffers on the full
//! path; on the downlevel path, which has none, textures of rows that the
//! shader reads with `textureLoad`, as three.js keeps bone matrices and
//! Filament its froxel list. Every texel holds 32-bit unsigned integers, so a
//! float field keeps its exact bits and no texel is ever filtered.

use crate::{Gpu, RenderPath};

/// Only the tests read tables back.
const READ_BACK: wgpu::TextureUsages = if cfg!(test) {
    wgpu::TextureUsages::COPY_SRC
} else {
    wgpu::TextureUsages::empty()
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Texel {
    /// Four 32-bit words, for records of whole 16-byte rows.
    Four,
    /// One 32-bit word.
    One,
}

impl Texel {
    pub fn bytes(self) -> u64 {
        match self {
            Texel::Four => 16,
            Texel::One => 4,
        }
    }

    fn format(self) -> wgpu::TextureFormat {
        match self {
            Texel::Four => wgpu::TextureFormat::Rgba32Uint,
            Texel::One => wgpu::TextureFormat::R32Uint,
        }
    }
}

/// How many entries of `entry_bytes` a data texture `width` texels wide and
/// at most `side` tall holds.
pub(crate) fn data_texture_entries(width: u32, side: u32, entry_bytes: u64, texel: Texel) -> u64 {
    width as u64 * side as u64 * texel.bytes() / entry_bytes
}

/// The texel rows `bytes` take in a texture `width` texels wide: how many are
/// whole, and the texels of the last one if it is not.
fn rows(bytes: usize, texel: Texel, width: u32) -> (u32, u32) {
    let texels = (bytes as u64 / texel.bytes()) as u32;
    (texels / width, texels % width)
}

enum Store {
    Buffer(wgpu::Buffer),
    Texture {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        texel: Texel,
    },
}

pub(crate) struct Table {
    store: Store,
}

impl Table {
    /// Room for `capacity` bytes, a whole number of `texel`s; a texture is
    /// `width` texels wide on the downlevel path.
    pub fn new(
        gpu: &Gpu,
        label: &str,
        path: RenderPath,
        capacity: u64,
        texel: Texel,
        width: u32,
    ) -> Table {
        let capacity = capacity.max(texel.bytes());
        let store = match path {
            RenderPath::Downlevel => {
                let texels = capacity.div_ceil(texel.bytes());
                let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height: texels.div_ceil(width as u64) as u32,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: texel.format(),
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | READ_BACK,
                    view_formats: &[],
                });
                Store::Texture {
                    view: texture.create_view(&Default::default()),
                    texture,
                    texel,
                }
            }
            _ => Store::Buffer(gpu.buffer(
                label,
                capacity,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            )),
        };
        Table { store }
    }

    /// Writes `bytes` from the start: the whole rows, then what the last one
    /// holds, so that a short frame uploads no padding.
    pub fn write(&self, gpu: &Gpu, bytes: &[u8]) {
        let (texture, texel) = match &self.store {
            Store::Buffer(buffer) => return gpu.write(buffer, bytes),
            Store::Texture { texture, texel, .. } => (texture, *texel),
        };
        let width = texture.width();
        let row_bytes = width as usize * texel.bytes() as usize;
        let (whole, rest) = rows(bytes.len(), texel, width);
        let (head, tail) = bytes.split_at(whole as usize * row_bytes);
        for (y, (w, h), part) in [(0, (width, whole), head), (whole, (rest, 1), tail)] {
            if w > 0 && h > 0 {
                gpu.write_texture(texture, y, (w, h), row_bytes as u32, part);
            }
        }
    }

    pub fn binding(&self) -> wgpu::BindingResource<'_> {
        match &self.store {
            Store::Buffer(buffer) => buffer.as_entire_binding(),
            Store::Texture { view, .. } => wgpu::BindingResource::TextureView(view),
        }
    }
}

pub(crate) fn layout_entry(binding: u32, path: RenderPath) -> wgpu::BindGroupLayoutEntry {
    let ty = match path {
        RenderPath::Downlevel => wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        _ => wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    };
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_split_whole_rows_from_the_last() {
        assert_eq!(rows(0, Texel::One, 8), (0, 0));
        assert_eq!(rows(4 * 8, Texel::One, 8), (1, 0));
        assert_eq!(rows(4 * 19, Texel::One, 8), (2, 3));
        assert_eq!(rows(16 * 5, Texel::Four, 4), (1, 1));
    }

    /// The words of a one-word-texel `texture`, rows in order.
    fn read_words(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u32> {
        let (width, height) = (texture.width(), texture.height());
        let row = width * 4;
        let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        gpu.queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let mapped = buffer.get_mapped_range(..).unwrap();
        (mapped.chunks_exact(padded as usize))
            .flat_map(|r| bytemuck::cast_slice::<u8, u32>(&r[..row as usize]).to_vec())
            .collect()
    }

    #[test]
    fn a_texture_table_holds_what_was_written() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let gpu = Gpu::new(std::sync::Arc::new(device), std::sync::Arc::new(queue));
        let table = Table::new(&gpu, "test", RenderPath::Downlevel, 4 * 40, Texel::One, 8);
        let Store::Texture { texture, .. } = &table.store else {
            panic!("the downlevel path keeps tables in textures");
        };
        assert_eq!((texture.width(), texture.height()), (8, 5));
        let words: Vec<u32> = (0..19).map(|i| 0xA5A5_0000 | i).collect();
        table.write(&gpu, bytemuck::cast_slice(&words));
        assert_eq!(gpu.uploaded.get(), 4 * 19);
        let read = read_words(&gpu, texture);
        assert_eq!(&read[..19], &words[..]);
        assert!(read[19..].iter().all(|&w| w == 0), "{read:?}");
    }

    #[test]
    fn capacity_is_whole_entries() {
        // A light takes four 16-byte texels, a shadow slot six.
        assert_eq!(data_texture_entries(1024, 2048, 64, Texel::Four), 524_288);
        assert_eq!(data_texture_entries(1024, 2048, 96, Texel::Four), 349_525);
        assert_eq!(data_texture_entries(4, 1, 4, Texel::One), 4);
    }
}
