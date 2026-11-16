// SPDX-License-Identifier: Apache-2.0

//! One frame drawn offscreen and read back, for the page's tests and
//! screenshots: the canvas's own texture cannot be copied from.

pub struct Capture {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    buffer: wgpu::Buffer,
    padded_row: u32,
}

impl Capture {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        (width, height): (u32, u32),
    ) -> Capture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let padded_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture"),
            size: padded_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Capture {
            view: texture.create_view(&Default::default()),
            texture,
            buffer,
            padded_row,
        }
    }

    pub fn fits(&self, size: (u32, u32)) -> bool {
        (self.texture.width(), self.texture.height()) == size
    }

    /// Copies the drawn texture out and maps it; `done` runs once the bytes
    /// can be read with [`Capture::rgba`].
    pub fn read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        done: impl FnOnce(bool) + Send + 'static,
    ) {
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_row),
                    rows_per_image: Some(self.texture.height()),
                },
            },
            self.texture.size(),
        );
        queue.submit([encoder.finish()]);
        (self.buffer).map_async(wgpu::MapMode::Read, .., move |result| done(result.is_ok()));
        // WebGL maps on the next poll; WebGPU settles the map itself.
        let _ = device.poll(wgpu::PollType::Poll);
    }

    /// The mapped frame as tightly packed RGBA rows, unmapping it.
    pub fn rgba(&self) -> Vec<u8> {
        let (width, height) = (self.texture.width(), self.texture.height());
        let row = (width * 4) as usize;
        let bgra = matches!(
            self.texture.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut out = Vec::with_capacity(row * height as usize);
        {
            let mapped = self.buffer.get_mapped_range(..).expect("mapped by read");
            for r in mapped.chunks_exact(self.padded_row as usize) {
                out.extend_from_slice(&r[..row]);
            }
        }
        self.buffer.unmap();
        if bgra {
            for texel in out.as_chunks_mut::<4>().0 {
                texel.swap(0, 2);
            }
        }
        out
    }
}
