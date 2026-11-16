// SPDX-License-Identifier: Apache-2.0

//! Particles and ground dots: one instanced draw after everything else of
//! the scene, no pipeline or buffer made for it after the renderer, every
//! built-in shape, additive and covering, facing the camera and lying flat,
//! and a least size in pixels.

use image::RgbaImage;
use sariema_core::{Camera, room_camera};
use sariema_format::RoomPack;
use sariema_format::demo::demo_room;
use sariema_render::{
    GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, SHAPES, Sprite, compare_images,
};

mod common;

const W: u32 = 480;
const H: u32 = 270;

fn golden(name: &str) -> String {
    format!(
        "{}/../../tests/golden/{name}.png",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn camera(room: &RoomPack) -> Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

fn headless(quality: Quality) -> (Headless, RoomPack) {
    let room = demo_room();
    let mut h = Headless::new(W, H, quality).unwrap();
    h.renderer().load_room(&room).unwrap();
    (h, room)
}

fn sprite(position: [f32; 3], size: f32, color: [f32; 4], shape: u32) -> Sprite {
    Sprite {
        position,
        size,
        color,
        shape,
        additive: 1.0,
        ..Sprite::default()
    }
}

/// Each shape standing in a row, bright and additive; the same row lying
/// flat on the floor, covering and dark; a trail of small soft dots.
fn showcase() -> Vec<Sprite> {
    let mut all = Vec::new();
    for shape in 0..SHAPES {
        let x = -2.25 + 1.5 * shape as f32;
        all.push(Sprite {
            rotation: 0.3,
            ..sprite([x, 1.8, 2.0], 0.9, [2.0, 1.4, 0.6, 1.0], shape)
        });
        all.push(Sprite {
            additive: 0.0,
            flags: Sprite::FLAT,
            ..sprite([x, 0.02, 3.2], 0.8, [0.05, 0.1, 0.4, 0.9], shape)
        });
    }
    for k in 0..8 {
        let t = k as f32 / 7.0;
        all.push(Sprite {
            flags: Sprite::FLAT,
            ..sprite([-3.0 + 6.0 * t, 0.02, 4.2], 0.2, [1.0, 0.6, 0.2, 0.55], 2)
        });
    }
    all
}

fn render(h: &mut Headless, room: &RoomPack, sprites: &[Sprite]) -> RgbaImage {
    h.renderer().set_sprites(sprites);
    h.render(&camera(room), 0.0)
}

fn within(a: &RgbaImage, b: &RgbaImage) -> bool {
    let d = compare_images(a, b).unwrap();
    d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO
}

fn luma(img: &RgbaImage, x: u32, y: u32, r: u32) -> f32 {
    let mut sum = 0.0;
    for dy in 0..=2 * r {
        for dx in 0..=2 * r {
            let p = img.get_pixel(x + dx - r, y + dy - r);
            sum += 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32;
        }
    }
    sum / ((2 * r + 1) * (2 * r + 1)) as f32
}

/// Where `p` lands on the image.
fn pixel(camera: &Camera, p: [f32; 3]) -> (u32, u32) {
    let clip = camera.proj * camera.view * glam::Vec4::new(p[0], p[1], p[2], 1.0);
    let ndc = clip.truncate() / clip.w;
    (
        ((ndc.x * 0.5 + 0.5) * W as f32) as u32,
        ((0.5 - ndc.y * 0.5) * H as f32) as u32,
    )
}

#[test]
fn sprites_are_one_draw_and_make_no_pipeline_or_buffer_after_the_renderer() {
    let (mut h, room) = headless(Quality::high());
    // The first frame draws the shadow maps too.
    render(&mut h, &room, &[]);
    render(&mut h, &room, &[]);
    let (draws, pipelines, buffers) = {
        let r = h.renderer();
        (
            r.last_frame_stats().draws,
            r.pipelines_created(),
            r.buffers_created(),
        )
    };
    render(&mut h, &room, &showcase());
    let r = h.renderer();
    let stats = r.last_frame_stats();
    assert_eq!(stats.draws, draws + 1);
    assert_eq!(stats.sprites, showcase().len() as u32);
    assert_eq!(stats.sprites_dropped, 0);
    assert_eq!(r.pipelines_created(), pipelines);
    assert_eq!(r.buffers_created(), buffers);
}

#[test]
fn sprites_past_the_quality_s_count_are_left_out_and_counted() {
    let (mut h, room) = headless(Quality {
        max_sprites: 3,
        ..Quality::high()
    });
    render(&mut h, &room, &showcase());
    let stats = h.renderer().last_frame_stats();
    assert_eq!(stats.sprites, 3);
    assert_eq!(stats.sprites_dropped, showcase().len() as u32 - 3);
}

#[test]
fn an_additive_sprite_brightens_and_a_covering_one_darkens() {
    let (mut h, room) = headless(Quality::high());
    let cam = camera(&room);
    let at = [0.5, 0.6, 3.0];
    let (x, y) = pixel(&cam, at);
    let plain = render(&mut h, &room, &[]);
    let lit = render(&mut h, &room, &[sprite(at, 0.6, [3.0, 3.0, 3.0, 1.0], 0)]);
    let dark = render(
        &mut h,
        &room,
        &[Sprite {
            additive: 0.0,
            ..sprite(at, 0.6, [0.0, 0.0, 0.0, 1.0], 2)
        }],
    );
    let (p, l, d) = (
        luma(&plain, x, y, 2),
        luma(&lit, x, y, 2),
        luma(&dark, x, y, 2),
    );
    assert!(l > p + 40.0, "{l} > {p}");
    assert!(d < p * 0.5, "{d} < {p}");
}

#[test]
fn a_sprite_below_the_least_size_is_still_drawn() {
    let at = [0.0, 0.5, 2.5];
    let tiny = [sprite(at, 0.002, [40.0, 40.0, 40.0, 1.0], 0)];
    let shown = |min_px| {
        let (mut h, room) = headless(Quality {
            sprite_min_px: min_px,
            ..Quality::high()
        });
        let (x, y) = pixel(&camera(&room), at);
        let plain = render(&mut h, &room, &[]);
        let with = render(&mut h, &room, &tiny);
        luma(&with, x, y, 2) - luma(&plain, x, y, 2)
    };
    let (never, least) = (shown(0), shown(3));
    assert!(
        least > 3.0 && least > 4.0 * never.max(0.0),
        "{least} against {never}"
    );
}

#[test]
fn the_shapes_match_their_golden() {
    let (mut h, room) = headless(Quality::high());
    let img = render(&mut h, &room, &showcase());
    let golden = image::open(golden("sprites")).unwrap().to_rgba8();
    assert!(within(&img, &golden), "{:?}", compare_images(&img, &golden));
}

#[test]
fn the_downlevel_path_draws_the_same_shapes() {
    let (mut h, room) = headless(common::downlevel());
    let img = render(&mut h, &room, &showcase());
    common::matches_golden_on_the_downlevel_path(&img, "sprites", &golden("sprites"));
}

#[test]
#[ignore = "rewrites tests/golden/sprites.png"]
fn write_golden() {
    let (mut h, room) = headless(Quality::high());
    render(&mut h, &room, &showcase())
        .save(golden("sprites"))
        .unwrap();
}
