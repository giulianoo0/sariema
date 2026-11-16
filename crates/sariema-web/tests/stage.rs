// SPDX-License-Identifier: Apache-2.0

//! The web stage run natively: the frames it draws match the renderer's
//! goldens, actors pose by clip and time, sounds mix ahead, and pointers
//! reach the UI on step boundaries.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use image::RgbaImage;
use sariema_format::actor::demo::demo_arm;
use sariema_format::demo::demo_room;
use sariema_format::sound::{Container, Load, Loudness, Sound, SoundBank};
use sariema_render::{GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, compare_images};
use sariema_web::settings::WebSettings;
use sariema_web::stage::Stage;

const W: u32 = 960;
const H: u32 = 540;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const FRAME_MS: f64 = 1000.0 / 60.0;

fn golden(name: &str) -> RgbaImage {
    let path = format!(
        "{}/../../tests/golden/{name}.png",
        env!("CARGO_MANIFEST_DIR")
    );
    image::open(path).unwrap().to_rgba8()
}

struct Target {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Target {
    fn new() -> Target {
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

    fn stage(&self, options: &str) -> Stage {
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

    fn image(&self) -> RgbaImage {
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

fn within_tolerance(a: &RgbaImage, b: &RgbaImage) -> bool {
    let d = compare_images(a, b).unwrap();
    d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO
}

#[test]
fn the_demo_room_draws_as_its_golden() {
    let target = Target::new();
    let mut stage = target.stage(r#"{"quality":"high"}"#);
    stage
        .load_room(&sariema_format::encode(&demo_room()))
        .unwrap();
    stage.frame(0.0, &target.view).unwrap();
    let d = compare_images(&target.image(), &golden("demo_lit")).unwrap();
    assert_eq!(d.max_channel_delta, 0, "{d:?}");
}

#[test]
fn the_downlevel_path_draws_within_the_golden_tolerance() {
    let target = Target::new();
    let mut stage = target.stage(r#"{"quality":"high","path":"downlevel"}"#);
    stage
        .load_room(&sariema_format::encode(&demo_room()))
        .unwrap();
    stage.frame(0.0, &target.view).unwrap();
    assert_eq!(stage.stats()["path"], "downlevel");
    assert!(within_tolerance(&target.image(), &golden("demo_lit")));
}

#[test]
fn an_actor_holds_the_pose_of_its_clip_time() {
    let target = Target::new();
    let mut stage = target.stage(r#"{"quality":"high"}"#);
    stage
        .load_room(&sariema_format::encode(&demo_room()))
        .unwrap();
    let model = stage
        .load_actor(&sariema_format::actor::encode(&demo_arm()))
        .unwrap();
    let arm = stage.spawn_actor(model).unwrap();
    let place = Mat4::from_scale_rotation_translation(
        Vec3::splat(1.5),
        Quat::from_rotation_y(0.6),
        Vec3::new(0.6, 0.0, 1.2),
    );
    stage
        .set_actor(arm, place, Some("bend"), 0.5, 0.0, true)
        .unwrap();
    for i in 0..3 {
        stage.frame(i as f64 * FRAME_MS, &target.view).unwrap();
    }
    assert!(within_tolerance(
        &target.image(),
        &golden("demo_actor_bent")
    ));
    assert!(
        stage
            .set_actor(arm, place, Some("nope"), 0.0, 1.0, true)
            .is_err()
    );
    assert!(stage.spawn_actor(9).is_err());
}

#[test]
fn shadows_and_occlusion_turn_off_by_setting() {
    let target = Target::new();
    let mut off = target.stage(r#"{"quality":"high","shadows":false,"ambient_occlusion":false}"#);
    off.load_room(&sariema_format::encode(&demo_room()))
        .unwrap();
    off.frame(0.0, &target.view).unwrap();
    let without = target.image();
    let mut plain_room = demo_room();
    for light in &mut plain_room.lights {
        light.casts_shadow = false;
    }
    plain_room.look.ao_strength = 0.0;
    let mut plain = target.stage(r#"{"quality":"high"}"#);
    plain
        .load_room(&sariema_format::encode(&plain_room))
        .unwrap();
    plain.frame(0.0, &target.view).unwrap();
    let d = compare_images(&without, &target.image()).unwrap();
    assert_eq!(d.max_channel_delta, 0, "{d:?}");
    assert!(
        compare_images(&without, &golden("demo_lit"))
            .unwrap()
            .max_channel_delta
            > 6
    );
}

#[test]
fn steps_follow_the_page_clock() {
    let target = Target::new();
    let mut stage = target.stage("");
    let steps: u32 = (0..=60)
        .map(|i| {
            stage
                .frame(i as f64 * FRAME_MS, &target.view)
                .unwrap()
                .steps
        })
        .sum();
    assert_eq!(steps, 60);
    stage.pause();
    assert_eq!(stage.frame(1e6, &target.view).unwrap().steps, 0);
    assert!((stage.now_s() - 1.0).abs() < 1e-9);
}

fn beep_bank(rate: u32) -> Vec<u8> {
    let frames = rate / 10;
    let samples: Vec<f32> = (0..frames)
        .map(|i| 0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin())
        .collect();
    let bank = SoundBank {
        buses: sariema_audio::default_buses()
            .into_iter()
            .map(|b| sariema_format::sound::Bus {
                name: b.name,
                gain_db: 0.0,
                target_lufs: None,
            })
            .collect(),
        sounds: vec![Sound {
            name: "beep".into(),
            bus: 2,
            gain_db: 0.0,
            container: Container::Wav,
            sample_rate: rate,
            channels: 1,
            frames: frames as u64,
            loop_points: None,
            stream: false,
            load: Load::Bank,
            enter_s: None,
            leave_s: None,
            loudness: Loudness {
                integrated_lufs: -9.0,
                peak_dbfs: -6.0,
                seam_step_db: None,
            },
            data: sariema_audio::wav(&samples, 1, rate, sariema_audio::WavFormat::Pcm16),
        }],
        mixes: Vec::new(),
    };
    sariema_format::sound::encode(&bank)
}

#[test]
fn a_sound_is_mixed_a_lead_ahead_in_whole_blocks() {
    let target = Target::new();
    let mut stage = target.stage(r#"{"audio":{"lead_s":0.05,"block_frames":512}}"#);
    stage.load_sounds(&beep_bank(48_000), 48_000).unwrap();
    let mut out = Vec::new();
    stage.mix(&mut out);
    // 0.05 s at 48 kHz is 2400 frames: five blocks of 512, two channels each.
    assert_eq!(out.len(), 5 * 512 * 2);
    assert!(out.iter().all(|&s| s == 0.0));
    stage.play("beep").unwrap();
    stage.audio_played(1024);
    stage.mix(&mut out);
    assert_eq!(out.len(), 2 * 512 * 2);
    assert!(
        out.iter().any(|s| s.abs() > 0.1),
        "the beep is in the new blocks"
    );
    assert!(stage.play("missing").is_err());
}

#[test]
fn a_tap_on_a_button_arrives_as_ui_events_on_a_step() {
    let target = Target::new();
    let mut stage = target.stage("");
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let json = std::fs::read_to_string(format!("{root}/examples/ui/controls.json")).unwrap();
    let font = "../../crates/sariema-ui/fonts/Nunito.ttf";
    let files = HashMap::from([(
        font.to_string(),
        std::fs::read(format!("{root}/crates/sariema-ui/fonts/Nunito.ttf")).unwrap(),
    )]);
    stage.set_ui(&json, &files).unwrap();
    // The action button, 80 px square, 16 px from the bottom right corner.
    let (x, y) = (W as f32 - 56.0, H as f32 - 56.0);
    stage.frame(0.0, &target.view).unwrap();
    stage
        .input()
        .pointer(sariema_web::input::PointerKind::Down, 1, x, y);
    assert!(stage.take_events().is_empty(), "input waits for a step");
    stage.frame(FRAME_MS, &target.view).unwrap();
    stage
        .input()
        .pointer(sariema_web::input::PointerKind::Up, 1, x, y);
    stage.frame(2.0 * FRAME_MS, &target.view).unwrap();
    let events = stage.take_events();
    let of = |kind: &str| {
        events
            .iter()
            .any(|e| e["type"] == kind && e["node"] == "action")
    };
    assert!(of("press") && of("tap"), "{events:?}");
}
