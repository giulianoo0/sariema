// SPDX-License-Identifier: Apache-2.0

//! Rendering without a window: into a texture read back to the CPU.

use std::sync::Arc;

use image::RgbaImage;
use sariema_core::Camera;
use sariema_format::RoomPack;

use crate::ui::{UiBatch, UiInstance};
use crate::{Gpu, Quality, RenderConfig, RenderError, Renderer, request_device};

const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A [`Renderer`] on its own device, drawing into an offscreen image of a
/// fixed size.
pub struct Headless {
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Headless {
    pub fn new(width: u32, height: u32, quality: Quality) -> Result<Headless, RenderError> {
        let (device, queue) = device()?;
        let config = RenderConfig {
            width,
            height,
            output_format: OUTPUT_FORMAT,
            quality,
        };
        let renderer = Renderer::new(device.clone(), queue, config)?;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless"),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(Headless {
            renderer,
            view: target.create_view(&Default::default()),
            target,
        })
    }

    /// [`Headless::render`] keeps returning images of the size given to
    /// [`Headless::new`], even after the renderer is resized.
    pub fn renderer(&mut self) -> &mut Renderer {
        &mut self.renderer
    }

    pub fn render(&mut self, camera: &Camera, time_s: f32) -> RgbaImage {
        self.draw(camera, time_s);
        self.image()
    }

    /// Draws a frame without reading it back.
    pub fn draw(&mut self, camera: &Camera, time_s: f32) {
        self.renderer.render(camera, time_s, &self.view);
    }

    /// Draws the HUD's quads over the last frame drawn.
    pub fn draw_ui(&mut self, instances: &[UiInstance], batches: &[UiBatch]) {
        self.renderer.render_ui(&self.view, instances, batches);
    }

    /// Blocks until the GPU has executed every frame drawn so far.
    pub fn finish(&self) -> Result<(), RenderError> {
        self.renderer
            .gpu()
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map(drop)
            .map_err(|e| RenderError::Device(e.to_string()))
    }

    /// The last frame drawn, read back from the GPU.
    pub fn image(&self) -> RgbaImage {
        let (width, height) = (self.target.width(), self.target.height());
        let texels = read_back(self.renderer.gpu(), &self.target, wgpu::TextureAspect::All);
        RgbaImage::from_raw(width, height, texels).expect("4 bytes per texel")
    }
}

/// One frame of `room` seen by `camera`, at [`Quality::high`].
pub fn render_to_image(
    room: &RoomPack,
    camera: &Camera,
    width: u32,
    height: u32,
    time_s: f32,
) -> Result<RgbaImage, RenderError> {
    let mut headless = Headless::new(width, height, Quality::high())?;
    headless.renderer.load_room(room)?;
    Ok(headless.render(camera, time_s))
}

/// Fraction of a `width × height` frame where no geometry is drawn, from a
/// depth-only render. Additive draws do not count: they add light over the
/// background without covering it.
pub fn render_coverage(
    room: &RoomPack,
    camera: &Camera,
    width: u32,
    height: u32,
) -> Result<f32, RenderError> {
    let mut headless = Headless::new(width, height, Quality::low())?;
    let renderer = &mut headless.renderer;
    renderer.load_room(room)?;
    let depth = renderer
        .gpu()
        .device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("coverage depth"),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::scene::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
    renderer.render_depth(camera, &depth.create_view(&Default::default()));
    let depths = read_back(renderer.gpu(), &depth, wgpu::TextureAspect::DepthOnly);
    let far = depths
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|&&d| f32::from_le_bytes(d) >= 1.0)
        .count();
    Ok(far as f32 / (width as u64 * height as u64) as f32)
}

fn device() -> Result<(Arc<wgpu::Device>, Arc<wgpu::Queue>), RenderError> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|_| RenderError::NoAdapter)?;
    request_device(&adapter, "sariema headless")
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

/// The texture's bytes, rows tightly packed, for formats of 4 bytes per texel.
fn read_back(gpu: &Gpu, texture: &wgpu::Texture, aspect: wgpu::TextureAspect) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row = width * 4;
    let padded_row = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("read back"),
        size: padded_row as u64 * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        extent(width, height),
    );
    gpu.queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| {
        result.expect("mapping a read-back buffer");
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("waiting for the GPU");
    let mapped = buffer.get_mapped_range(..).expect("mapped above");
    mapped
        .chunks_exact(padded_row as usize)
        .flat_map(|r| &r[..row as usize])
        .copied()
        .collect()
}
