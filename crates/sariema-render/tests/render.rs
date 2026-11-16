// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use image::RgbaImage;
use sariema_core::{Camera, room_camera};
use sariema_format::demo::demo_room;
use sariema_format::{
    Aabb, AlphaMode, Ambient, Light, LightKind, Mobility, RoomCamera, RoomPack, Shading,
    TextureFormat,
};
use sariema_render::{
    GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, RenderConfig, RenderError, Renderer,
    compare_images, render_coverage, render_to_image,
};

const W: u32 = 960;
const H: u32 = 540;
const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/golden/demo_lit.png"
);

fn camera_of(room: &RoomPack) -> Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

fn render(room: &RoomPack) -> RgbaImage {
    render_to_image(room, &camera_of(room), W, H, 0.0).unwrap()
}

fn headless(quality: Quality, room: &RoomPack) -> Headless {
    let mut h = Headless::new(W, H, quality).unwrap();
    h.renderer().load_room(room).unwrap();
    h
}

fn within_golden_tolerance(a: &RgbaImage, b: &RgbaImage) -> bool {
    let d = compare_images(a, b).unwrap();
    d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO
}

fn luma(p: &image::Rgba<u8>) -> f32 {
    (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0
}

#[test]
fn renders_demo_room_matching_golden() {
    let img = render(&demo_room());
    let golden = image::open(GOLDEN).unwrap().to_rgba8();
    let d = compare_images(&img, &golden).unwrap();
    if !(d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO) {
        let actual = concat!(env!("CARGO_TARGET_TMPDIR"), "/demo_lit.actual.png");
        img.save(actual).unwrap();
        panic!("{d:?}; this render is in {actual}");
    }
}

#[test]
#[ignore = "rewrites tests/golden/demo_lit.png"]
fn write_golden() {
    render(&demo_room()).save(GOLDEN).unwrap();
}

#[test]
fn image_is_not_blank_and_is_lit() {
    let img = render(&demo_room());
    let lumas: Vec<f32> = img.pixels().map(luma).collect();
    let n = lumas.len() as f32;
    let mean = lumas.iter().sum::<f32>() / n;
    let sd = (lumas.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / n).sqrt();
    assert!(mean > 0.05 && mean < 0.9, "mean luma {mean}");
    assert!(sd > 0.02, "luma standard deviation {sd}");
}

#[test]
fn render_creates_no_pipelines() {
    let room = demo_room();
    let mut h = headless(Quality::high(), &room);
    let pipelines = h.renderer().pipelines_created();
    let buffers = h.renderer().buffers_created();
    for frame in 0..3 {
        h.render(&camera_of(&room), frame as f32 / 60.0);
    }
    assert_eq!(h.renderer().pipelines_created(), pipelines);
    assert_eq!(h.renderer().buffers_created(), buffers);
}

#[test]
fn hiding_a_draw_changes_the_image() {
    let room = demo_room();
    let mut h = headless(Quality::high(), &room);
    let before = h.render(&camera_of(&room), 0.0);
    assert!(h.renderer().set_draw_visible("box_red", false));
    assert!(!h.renderer().set_draw_visible("no_such_draw", false));
    let after = h.render(&camera_of(&room), 0.0);
    assert!(compare_images(&before, &after).unwrap().max_channel_delta > 0);
}

#[test]
fn resize_to_zero_is_ignored() {
    let room = demo_room();
    let mut h = headless(Quality::high(), &room);
    h.renderer().resize(0, H);
    h.renderer().resize(W, 0);
    let img = h.render(&camera_of(&room), 0.0);
    assert!(within_golden_tolerance(&img, &render(&room)));
}

#[test]
fn mirrored_draw_looks_like_the_original() {
    let room = demo_room();
    let mut mirrored = room.clone();
    let red = &mut mirrored.draws[1];
    let flip_x = glam::Mat4::from_scale(glam::vec3(-1.0, 1.0, 1.0));
    red.transform = (glam::Mat4::from_cols_array(&red.transform) * flip_x).to_cols_array();
    let (a, b) = (render(&room), render(&mirrored));
    let d = compare_images(&a, &b).unwrap();
    assert!(within_golden_tolerance(&a, &b), "{d:?}");
}

#[test]
fn unlit_material_ignores_lights() {
    let mut room = demo_room();
    room.draws.truncate(1);
    let floor = &mut room.materials[0];
    floor.base_color = [1.0; 4];
    floor.base_color_texture = None;
    floor.shading = Shading::Unlit;
    room.lights.clear();
    room.sun = None;
    room.ambient = Ambient { sh9: [[0.0; 3]; 9] };
    // Hejl-Burgess-Dawson maps 1.0 to about 0.84 at exposure 1; only a
    // strong exposure brings white close to 255.
    room.look.exposure = 20.0;
    let img = render(&room);
    let centre = img.get_pixel(W / 2, H / 2);
    assert!(centre.0[..3].iter().all(|&c| c >= 250), "{centre:?}");
}

#[test]
fn additive_draw_adds_light_and_hides_nothing() {
    let mut room = demo_room();
    room.materials[3].shading = Shading::Additive;
    let mut h = headless(Quality::high(), &room);
    let with = h.render(&camera_of(&room), 0.0);
    h.renderer().set_draw_visible("box_blue", false);
    let without = h.render(&camera_of(&room), 0.0);
    let brighter_or_same = with
        .pixels()
        .zip(without.pixels())
        .all(|(a, b)| (0..3).all(|c| a[c] >= b[c]));
    assert!(brighter_or_same);
    assert!(compare_images(&with, &without).unwrap().max_channel_delta > 0);
}

#[test]
fn blended_draw_shows_what_is_behind() {
    let mut room = demo_room();
    room.materials[2].alpha = AlphaMode::Blend;
    room.materials[2].base_color[3] = 0.4;
    let mut h = headless(Quality::high(), &room);
    let glass = h.render(&camera_of(&room), 0.0);
    h.renderer().set_draw_visible("box_green", false);
    let hidden = h.render(&camera_of(&room), 0.0);
    let opaque = render(&demo_room());
    assert!(compare_images(&glass, &opaque).unwrap().max_channel_delta > 0);
    assert!(compare_images(&glass, &hidden).unwrap().max_channel_delta > 0);
}

#[test]
fn compare_images_reports_size_mismatch_as_none() {
    let a = RgbaImage::new(4, 4);
    assert!(compare_images(&a, &RgbaImage::new(4, 5)).is_none());
    let d = compare_images(&a, &a).unwrap();
    assert_eq!((d.max_channel_delta, d.differing_pixels_ratio), (0, 0.0));
}

fn room_error(room: &RoomPack) -> String {
    match render_to_image(room, &camera_of(room), W, H, 0.0) {
        Err(RenderError::Room(why)) => why,
        other => panic!("expected a room error, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn index_count_not_multiple_of_three_is_room_error() {
    let mut room = demo_room();
    room.meshes[1].indices.pop();
    assert!(room_error(&room).contains("mesh 1"));
}

#[test]
fn zero_sized_texture_is_room_error() {
    let mut room = demo_room();
    room.textures[0].width = 0;
    assert!(room_error(&room).contains("checker"));
}

#[test]
fn empty_mip_chain_is_room_error() {
    let mut room = demo_room();
    room.textures[0].mips.clear();
    assert!(room_error(&room).contains("checker"));
}

#[test]
fn wrongly_sized_mip_is_room_error() {
    let mut room = demo_room();
    let size = room.textures[0].mips[0].len();
    room.textures[0].mips.push(vec![0; size]);
    assert!(room_error(&room).contains("checker"));
}

#[test]
fn astc_texture_is_room_error() {
    let mut room = demo_room();
    room.textures[0].format = TextureFormat::Astc4x4Srgb;
    assert!(room_error(&room).contains("checker"));
}

#[test]
fn coverage_of_demo_room_is_partial() {
    let room = demo_room();
    let background = render_coverage(&room, &camera_of(&room), W, H).unwrap();
    assert!(background > 0.0 && background < 0.5, "{background}");
}

#[test]
fn coverage_without_draws_is_full() {
    let mut room = demo_room();
    room.draws.clear();
    let background = render_coverage(&room, &camera_of(&room), W, H).unwrap();
    assert_eq!(background, 1.0);
}

/// A 30 m floor under a 20 × 15 grid of small point lights.
fn many_lights_room() -> RoomPack {
    let mut room = demo_room();
    room.draws.truncate(1);
    let floor = &mut room.draws[0];
    floor.transform = glam::Mat4::from_scale(glam::vec3(3.0, 1.0, 3.0)).to_cols_array();
    floor.bounds = Aabb::around(&room.meshes[0], &floor.transform);
    room.lights = (0..300)
        .map(|i| Light {
            name: format!("l{i}"),
            kind: LightKind::Point,
            color: [1.0, 0.8, 0.6],
            intensity: 1.0,
            position: [
                -14.25 + 1.5 * (i % 20) as f32,
                0.3,
                -14.0 + 2.0 * (i / 20) as f32,
            ],
            range: 1.0,
            flicker: None,
            casts_shadow: false,
            mobility: Mobility::Static,
            importance: 1.0,
        })
        .collect();
    room
}

fn top_down(distance: f32) -> Camera {
    let cam = RoomCamera {
        fov_y_deg: 60.0,
        pitch_deg: 90.0,
        distance,
        focus: [0.0; 3],
    };
    room_camera(&cam, W as f32 / H as f32)
}

#[test]
fn lights_are_dropped_only_past_the_visible_budget() {
    let room = many_lights_room();
    let mut h = headless(Quality::medium(), &room);
    assert_eq!(h.renderer().effective_quality().max_visible_lights, 256);

    h.render(&top_down(8.0), 0.0);
    let near = h.renderer().last_frame_stats();
    assert!(
        near.lights_in_frustum > 0 && near.lights_in_frustum < 256,
        "{near:?}"
    );
    assert_eq!(near.lights_dropped, 0, "{near:?}");

    h.render(&top_down(40.0), 0.0);
    let far = h.renderer().last_frame_stats();
    assert_eq!(far.lights_in_frustum, 300, "{far:?}");
    assert_eq!(far.lights_dropped, 300 - 256, "{far:?}");
    assert!(
        far.clusters > 0 && far.max_lights_overlapping_cluster > 0,
        "{far:?}"
    );
}

#[test]
fn light_out_of_range_changes_nothing() {
    let room = demo_room();
    let mut with = room.clone();
    with.lights.push(Light {
        name: "too_high".into(),
        kind: LightKind::Point,
        color: [1.0; 3],
        intensity: 500.0,
        position: [0.0, 4.0, 0.0],
        range: 1.0,
        flicker: None,
        casts_shadow: false,
        mobility: Mobility::Static,
        importance: 1.0,
    });
    let d = compare_images(&render(&room), &render(&with)).unwrap();
    assert_eq!(d.max_channel_delta, 0, "{d:?}");
}

#[test]
fn low_and_high_quality_look_the_same() {
    let room = demo_room();
    let low = headless(Quality::low(), &room).render(&camera_of(&room), 0.0);
    let high = headless(Quality::high(), &room).render(&camera_of(&room), 0.0);
    let d = compare_images(&low, &high).unwrap();
    assert!(within_golden_tolerance(&low, &high), "{d:?}");
}

#[test]
fn cluster_grid_size_does_not_change_the_image() {
    let room = many_lights_room();
    let roomy = Quality {
        max_lights_per_cluster: 255,
        ..Quality::medium()
    };
    let fine = Quality {
        cluster_tile_px: 8,
        depth_slices: 32,
        cluster_budget: 400_000,
        ..roomy.clone()
    };
    let single = Quality {
        cluster_tile_px: 4096,
        depth_slices: 1,
        ..roomy
    };
    let mut fine = headless(fine, &room);
    let a = fine.render(&top_down(8.0), 0.0);
    let mut single = headless(single, &room);
    let b = single.render(&top_down(8.0), 0.0);
    assert!(fine.renderer().last_frame_stats().clusters > 10_000);
    assert_eq!(single.renderer().last_frame_stats().clusters, 1);
    assert_eq!(compare_images(&a, &b).unwrap().max_channel_delta, 0);
}

fn device(
    limits: impl FnOnce(wgpu::Limits) -> wgpu::Limits,
) -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        required_limits: limits(adapter.limits()),
        ..Default::default()
    }))
    .unwrap();
    (Arc::new(device), Arc::new(queue))
}

fn config(quality: Quality) -> RenderConfig {
    RenderConfig {
        width: W,
        height: H,
        output_format: wgpu::TextureFormat::Rgba8Unorm,
        quality,
    }
}

#[test]
fn device_without_fragment_storage_is_rejected() {
    let (device, queue) = device(|_| wgpu::Limits::downlevel_webgl2_defaults());
    match Renderer::new(device, queue, config(Quality::high())) {
        Err(RenderError::Device(why)) => assert!(why.contains("storage"), "{why}"),
        other => panic!("expected a device error, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn quality_is_clamped_to_the_device() {
    let (device, queue) = device(|limits| limits);
    let limits = device.limits();
    let greedy = Quality {
        max_visible_lights: u32::MAX,
        max_lights_per_cluster: u32::MAX,
        cluster_budget: u32::MAX,
        cluster_tile_px: 0,
        depth_slices: u32::MAX,
        msaa: 1000,
        shadow_atlas_size: u32::MAX,
        ..Quality::high()
    };
    let renderer = Renderer::new(device, queue, config(greedy)).unwrap();
    let q = renderer.effective_quality();
    let storage = limits.max_storage_buffer_binding_size;
    assert!(q.max_visible_lights as u64 * 64 <= storage, "{q:?}");
    assert!(q.cluster_budget as u64 * q.max_lights_per_cluster as u64 * 4 <= storage);
    assert!(
        q.depth_slices >= 1 && q.depth_slices <= q.cluster_budget,
        "{q:?}"
    );
    assert!(q.cluster_tile_px >= 1, "{q:?}");
    assert!(q.msaa.is_power_of_two() && q.msaa <= 1000, "{q:?}");
    assert!(
        q.shadow_atlas_size <= limits.max_texture_dimension_2d,
        "{q:?}"
    );
}
