// SPDX-License-Identifier: Apache-2.0

// Each test crate that declares `mod common` uses only some of these.
#![allow(dead_code)]

use std::sync::Arc;

use image::RgbaImage;
use sariema_web::settings::WebSettings;
use sariema_web::stage::Stage;

pub const W: u32 = 960;
pub const H: u32 = 540;
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub struct Target {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Target {
    pub fn new() -> Target {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) = sariema_render::request_device(&adapter, "stage test").unwrap();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Target {
            view: texture.create_view(&Default::default()),
            device,
            queue,
            texture,
        }
    }

    pub fn stage(&self, options: &str) -> Stage {
        let settings = WebSettings::from_json(options).unwrap();
        Stage::new(
            self.device.clone(),
            self.queue.clone(),
            settings,
            (W, H),
            FORMAT,
        )
        .unwrap()
    }

    pub fn image(&self) -> RgbaImage {
        let row = W * 4;
        let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (padded * H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(H),
                },
            },
            self.texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let mapped = buffer.get_mapped_range(..).unwrap();
        let texels = (mapped.chunks_exact(padded as usize))
            .flat_map(|r| r[..row as usize].to_vec())
            .collect();
        RgbaImage::from_raw(W, H, texels).unwrap()
    }
}
