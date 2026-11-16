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
    GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, RenderConfig, RenderError, RenderPath,
    Renderer, compare_images, render_coverage, render_to_image,
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
fn culling_leaves_out_draws_off_screen_and_changes_no_pixel() {
    let mut room = demo_room();
    let mut aside = room.draws[1].clone();
    aside.name = Some("aside".into());
    let moved = glam::Mat4::from_translation(glam::vec3(200.0, 0.0, 0.0))
        * glam::Mat4::from_cols_array(&aside.transform);
    aside.transform = moved.to_cols_array();
    aside.bounds = Aabb::around(&room.meshes[aside.mesh as usize], &aside.transform);
    aside.casts_shadow = false;
    room.draws.push(aside);
    let frame = |frustum_culling| {
        let quality = Quality {
            frustum_culling,
            ..Quality::high()
        };
        let mut h = headless(quality, &room);
        let image = h.render(&camera_of(&room), 0.0);
        (image, h.renderer().last_frame_stats().draws_culled)
    };
    let ((culled, left_out), (all, none_left_out)) = (frame(true), frame(false));
    assert_eq!((left_out, none_left_out), (1, 0));
    assert_eq!(compare_images(&culled, &all).unwrap().max_channel_delta, 0);
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
    room.materials[3].alpha = AlphaMode::Additive;
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

/// The demo room with box_blue blended additively, and no light at all unless `lights_on`.
fn additive_box(shading: Shading, lights_on: bool) -> RoomPack {
    let mut room = demo_room();
    room.materials[3].shading = shading;
    room.materials[3].alpha = AlphaMode::Additive;
    if !lights_on {
        room.lights.clear();
        room.sun = None;
        room.ambient = Ambient { sh9: [[0.0; 3]; 9] };
    }
    room
}

/// The largest change box_blue makes to the frame.
fn box_blue_delta(room: &RoomPack) -> u8 {
    let mut h = headless(Quality::high(), room);
    let with = h.render(&camera_of(room), 0.0);
    h.renderer().set_draw_visible("box_blue", false);
    let without = h.render(&camera_of(room), 0.0);
    compare_images(&with, &without).unwrap().max_channel_delta
}

#[test]
fn unlit_additive_glows_in_the_dark() {
    assert!(box_blue_delta(&additive_box(Shading::Unlit, false)) > 0);
}

#[test]
fn lit_additive_adds_only_the_light_it_receives() {
    assert_eq!(box_blue_delta(&additive_box(Shading::Lit, false)), 0);
    assert!(box_blue_delta(&additive_box(Shading::Lit, true)) > 0);
}

/// The demo room with an untextured floor, turned upside down when `flipped`.
fn plain_floor(flipped: bool, double_sided: bool) -> RoomPack {
    let mut room = demo_room();
    room.materials[0].base_color_texture = None;
    room.materials[0].double_sided = double_sided;
    if flipped {
        let floor = &mut room.draws[0];
        let half_turn = glam::Mat4::from_rotation_z(std::f32::consts::PI);
        floor.transform =
            (glam::Mat4::from_cols_array(&floor.transform) * half_turn).to_cols_array();
    }
    room
}

#[test]
fn double_sided_material_draws_and_lights_back_faces() {
    let floor_luma = |room: &RoomPack| {
        let img = render(room);
        let strip: Vec<f32> = (H - 40..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .map(|(x, y)| luma(img.get_pixel(x, y)))
            .collect();
        strip.iter().sum::<f32>() / strip.len() as f32
    };
    let front = floor_luma(&plain_floor(false, false));
    let culled = floor_luma(&plain_floor(true, false));
    let both = floor_luma(&plain_floor(true, true));
    assert!(culled < 0.5 * front, "culled {culled}, front {front}");
    // Shaded with the normal turned towards the viewer, as three.js's DoubleSide does.
    assert!(
        (both - front).abs() < 0.05 * front,
        "back {both}, front {front}"
    );
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
fn rgba8_bytes_tagged_astc_is_room_error() {
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
fn coverage_counts_back_faces_of_double_sided_draws() {
    let background = |room: &RoomPack| render_coverage(room, &camera_of(room), W, H).unwrap();
    let front = background(&plain_floor(false, false));
    let culled = background(&plain_floor(true, false));
    let both = background(&plain_floor(true, true));
    assert!(culled > front + 0.1, "culled {culled}, front {front}");
    assert!((both - front).abs() < 0.001, "back {both}, front {front}");
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

/// Where the downlevel path's renders are kept, for comparing them by eye.
fn downlevel_dir() -> String {
    let dir = concat!(env!("CARGO_TARGET_TMPDIR"), "/downlevel");
    std::fs::create_dir_all(dir).unwrap();
    dir.to_string()
}

#[test]
fn a_device_with_webgl2_limits_takes_the_downlevel_path() {
    let (device, queue) = device(|_| wgpu::Limits::downlevel_webgl2_defaults());
    let mut renderer = Renderer::new(device.clone(), queue, config(Quality::high())).unwrap();
    assert_eq!(renderer.effective_quality().path, RenderPath::Downlevel);
    let room = demo_room();
    renderer.load_room(&room).unwrap();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    renderer.render(
        &camera_of(&room),
        0.0,
        &target.create_view(&Default::default()),
    );
    assert!(renderer.last_frame_stats().draws > 0);
}

#[test]
fn auto_takes_the_full_path_where_storage_buffers_exist() {
    let (device, queue) = device(|limits| limits);
    let renderer = Renderer::new(device, queue, config(Quality::high())).unwrap();
    assert_eq!(renderer.effective_quality().path, RenderPath::Full);
}

#[test]
fn forcing_the_full_path_without_storage_buffers_is_refused() {
    let (device, queue) = device(|_| wgpu::Limits::downlevel_webgl2_defaults());
    let full = Quality {
        path: RenderPath::Full,
        ..Quality::high()
    };
    match Renderer::new(device, queue, config(full)) {
        Err(RenderError::Device(why)) => assert!(why.contains("storage"), "{why}"),
        other => panic!("expected a device error, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn the_downlevel_path_matches_the_lit_golden() {
    let room = demo_room();
    let downlevel = Quality {
        path: RenderPath::Downlevel,
        ..Quality::high()
    };
    let img = headless(downlevel, &room).render(&camera_of(&room), 0.0);
    img.save(format!("{}/demo_lit.png", downlevel_dir()))
        .unwrap();
    let golden = image::open(GOLDEN).unwrap().to_rgba8();
    let d = compare_images(&img, &golden).unwrap();
    assert!(within_golden_tolerance(&img, &golden), "{d:?}");
}

#[test]
fn data_textures_shade_like_storage_buffers() {
    let room = many_lights_room();
    // One sample, so neither path draws a depth pre-pass and only the tables differ.
    let tight = Quality {
        msaa: 1,
        max_lights_per_cluster: 8,
        ..Quality::medium()
    };
    let downlevel = Quality {
        path: RenderPath::Downlevel,
        ..tight.clone()
    };
    for distance in [8.0, 40.0] {
        let mut full = headless(tight.clone(), &room);
        let mut low = headless(downlevel.clone(), &room);
        let a = full.render(&top_down(distance), 0.0);
        let b = low.render(&top_down(distance), 0.0);
        assert_eq!(
            full.renderer().last_frame_stats(),
            low.renderer().last_frame_stats()
        );
        assert!(low.renderer().last_frame_stats().cluster_overflows > 0 || distance < 10.0);
        assert_eq!(compare_images(&a, &b).unwrap().max_channel_delta, 0);
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
        max_shadowed_lights: u32::MAX,
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
    let atlas_bytes = q.shadow_atlas_size as u64 * q.shadow_atlas_size as u64 * 4;
    assert!(atlas_bytes <= q.shadow_atlas_max_bytes, "{q:?}");
    let min_slots_per_side = q.shadow_atlas_size / q.shadow_min_slot_px;
    assert!(
        q.max_shadowed_lights <= min_slots_per_side * min_slots_per_side,
        "{q:?}"
    );
}

#[test]
fn downlevel_quality_is_held_to_its_data_textures() {
    let (device, queue) = device(|_| wgpu::Limits::downlevel_webgl2_defaults());
    let limits = device.limits();
    let greedy = Quality {
        max_visible_lights: u32::MAX,
        max_lights_per_cluster: u32::MAX,
        cluster_budget: u32::MAX,
        max_shadowed_lights: u32::MAX,
        data_texture_width: u32::MAX,
        actor_uniform_rows: u32::MAX,
        ..Quality::high()
    };
    let renderer = Renderer::new(device, queue, config(greedy)).unwrap();
    let q = renderer.effective_quality();
    let side = limits.max_texture_dimension_2d as u64;
    let texels = q.data_texture_width as u64 * side;
    assert!(q.data_texture_width as u64 <= side, "{q:?}");
    assert!(q.max_visible_lights as u64 * 4 <= texels, "{q:?}");
    assert!(
        q.cluster_budget as u64 * q.max_lights_per_cluster as u64 <= texels,
        "{q:?}"
    );
    assert!((q.max_shadowed_lights as u64 + 1) * 6 <= texels, "{q:?}");
    assert!(q.max_shadowed_lights > 0, "{q:?}");
    let uniform = limits.max_uniform_buffer_binding_size;
    assert!(q.actor_uniform_rows as u64 * 16 <= uniform, "{q:?}");
}
