// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use image::RgbaImage;
use sariema_core::room_camera;
use sariema_format::RoomPack;
use sariema_format::demo::demo_room;
use sariema_import::{TextureMode, encode_texture};
use sariema_render::{
    Headless, Quality, RenderConfig, RenderError, Renderer, compare_images, render_to_image,
    request_device,
};

const W: u32 = 960;
const H: u32 = 540;

/// `demo_room` with every texture encoded again from its largest mip.
fn demo_room_with(mode: TextureMode) -> RoomPack {
    let mut room = demo_room();
    for t in &mut room.textures {
        let img = RgbaImage::from_raw(t.width, t.height, t.mips[0].clone()).unwrap();
        *t = encode_texture(&t.name, &img, mode).unwrap();
    }
    room
}

fn render(room: &RoomPack) -> RgbaImage {
    let camera = room_camera(&room.camera, W as f32 / H as f32);
    render_to_image(room, &camera, W, H, 0.0).unwrap()
}

/// Whether the adapter that [`Headless`] picks can sample ASTC; says so when it cannot.
fn adapter_has_astc() -> bool {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .unwrap();
    let has = adapter
        .features()
        .contains(wgpu::Features::TEXTURE_COMPRESSION_ASTC);
    if !has {
        eprintln!(
            "skipped: adapter {} cannot sample ASTC",
            adapter.get_info().name
        );
    }
    has
}

fn room_error(renderer: &mut Renderer, room: &RoomPack) -> String {
    match renderer.load_room(room) {
        Err(RenderError::Room(why)) => why,
        other => panic!("expected a room error, got {other:?}"),
    }
}

#[test]
fn astc_render_is_close_to_rgba8_render() {
    if !adapter_has_astc() {
        return;
    }
    let rgba8 = render(&demo_room_with(TextureMode::Rgba8));
    let astc = render(&demo_room_with(TextureMode::Astc4x4));
    let d = compare_images(&rgba8, &astc).unwrap();
    // The ratio counts channel differences above 6, stricter than above 8.
    assert!(
        d.max_channel_delta <= 24 && d.differing_pixels_ratio <= 0.05,
        "{d:?}"
    );
}

#[test]
fn requested_device_samples_astc_when_the_adapter_can() {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, _) = request_device(&adapter, "test").unwrap();
    let astc = wgpu::Features::TEXTURE_COMPRESSION_ASTC;
    assert_eq!(
        device.features().contains(astc),
        adapter.features().contains(astc)
    );
}

#[test]
fn astc_without_device_support_is_room_error() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    assert!(
        !device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_ASTC)
    );
    let config = RenderConfig {
        width: W,
        height: H,
        output_format: wgpu::TextureFormat::Rgba8Unorm,
        quality: Quality::low(),
    };
    let mut renderer = Renderer::new(Arc::new(device), Arc::new(queue), config).unwrap();
    let why = room_error(&mut renderer, &demo_room_with(TextureMode::Astc4x4));
    assert!(why.contains(r#""textures": "rgba8""#), "{why}");
}

#[test]
fn astc_side_in_partial_blocks_is_room_error() {
    if !adapter_has_astc() {
        return;
    }
    let mut room = demo_room_with(TextureMode::Astc4x4);
    // Its mips still hold the right number of blocks.
    room.textures[0].width = 30;
    let mut headless = Headless::new(W, H, Quality::low()).unwrap();
    let why = room_error(headless.renderer(), &room);
    assert!(why.contains("checker") && why.contains("blocks"), "{why}");
}
