// SPDX-License-Identifier: Apache-2.0

use glam::{Vec3, vec3};
use image::RgbaImage;
use sariema_core::{Camera, room_camera};
use sariema_format::demo::demo_room;
use sariema_format::{Ambient, Background, Look, RoomCamera, RoomPack, Shading};
use sariema_render::{
    GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, compare_images, render_to_image,
};

const W: u32 = 960;
const H: u32 = 540;
const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/golden");

/// `real-noite` from LOOKS in the Origem game's post.ts.
fn real_noite() -> Look {
    Look {
        real: 1.0,
        night: 1.0,
        tilt: 0.0,
        focus: 0.0,
        bloom_threshold: 0.45,
        bloom_strength: 0.45,
        tint: [0.9, 0.95, 1.0],
        exposure: 2.1,
    }
}

/// `desenho-dia` from LOOKS in the Origem game's post.ts.
fn desenho_dia() -> Look {
    Look {
        real: 0.0,
        night: 0.0,
        tilt: 1.0,
        focus: 26.5,
        bloom_threshold: 0.3,
        bloom_strength: 0.6,
        tint: [1.0, 0.96, 0.85],
        exposure: 2.0,
    }
}

fn camera_of(room: &RoomPack) -> Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

fn headless(room: &RoomPack) -> Headless {
    let mut h = Headless::new(W, H, Quality::high()).unwrap();
    h.renderer().load_room(room).unwrap();
    h
}

/// The demo room seen through each look, in order.
fn renders(looks: &[Look]) -> Vec<RgbaImage> {
    let room = demo_room();
    let mut h = headless(&room);
    looks
        .iter()
        .map(|look| {
            h.renderer().set_look(look);
            h.render(&camera_of(&room), 0.0)
        })
        .collect()
}

fn with(change: impl FnOnce(&mut Look)) -> Look {
    let mut look = demo_room().look;
    change(&mut look);
    look
}

fn luma(p: &image::Rgba<u8>) -> f32 {
    0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
}

fn to_pixel(camera: &Camera, world: Vec3) -> (u32, u32) {
    let ndc = (camera.proj * camera.view).project_point3(world);
    (
        ((ndc.x * 0.5 + 0.5) * W as f32) as u32,
        ((0.5 - ndc.y * 0.5) * H as f32) as u32,
    )
}

/// Screen rectangle, min and max corners, around the draw called `name`.
fn screen_bounds(room: &RoomPack, name: &str) -> ((u32, u32), (u32, u32)) {
    let draw = room
        .draws
        .iter()
        .find(|d| d.name.as_deref() == Some(name))
        .unwrap();
    let (min, max) = (Vec3::from(draw.bounds.min), Vec3::from(draw.bounds.max));
    let camera = camera_of(room);
    let corners: Vec<(u32, u32)> = (0..8)
        .map(|i| {
            let pick = |bit: u32, lo: f32, hi: f32| if i & bit == 0 { lo } else { hi };
            let corner = vec3(
                pick(1, min.x, max.x),
                pick(2, min.y, max.y),
                pick(4, min.z, max.z),
            );
            to_pixel(&camera, corner)
        })
        .collect();
    let xs = corners.iter().map(|c| c.0);
    let ys = corners.iter().map(|c| c.1);
    (
        (xs.clone().min().unwrap(), ys.clone().min().unwrap()),
        (xs.max().unwrap(), ys.max().unwrap()),
    )
}

/// Squared luma differences between neighbouring pixels in rows `rows`.
fn high_frequency_energy(img: &RgbaImage, rows: std::ops::Range<u32>) -> f64 {
    let mut energy = 0.0;
    for y in rows {
        for x in 0..W - 1 {
            let here = luma(img.get_pixel(x, y));
            let right = luma(img.get_pixel(x + 1, y)) - here;
            let below = luma(img.get_pixel(x, (y + 1).min(H - 1))) - here;
            energy += (right * right + below * below) as f64;
        }
    }
    energy
}

fn mean_abs_difference(a: &RgbaImage, b: &RgbaImage) -> f64 {
    let sum: u64 = a
        .pixels()
        .zip(b.pixels())
        .map(|(p, q)| (0..3).map(|c| p[c].abs_diff(q[c]) as u64).sum::<u64>())
        .sum();
    sum as f64 / (3 * W * H) as f64
}

/// Mean HSL saturation of the 9×9 pixels where the centre of `name`'s bounds lands.
fn saturation_of(img: &RgbaImage, room: &RoomPack, name: &str) -> f32 {
    let draw = room
        .draws
        .iter()
        .find(|d| d.name.as_deref() == Some(name))
        .unwrap();
    let centre = (Vec3::from(draw.bounds.min) + Vec3::from(draw.bounds.max)) * 0.5;
    let (x, y) = to_pixel(&camera_of(room), centre);
    let mut sum = 0.0;
    for dy in 0..9 {
        for dx in 0..9 {
            let p = img.get_pixel(x + dx - 4, y + dy - 4);
            let max = p[0].max(p[1]).max(p[2]) as f32 / 255.0;
            let min = p[0].min(p[1]).min(p[2]) as f32 / 255.0;
            let spread = 1.0 - (max + min - 1.0).abs();
            sum += if spread > 0.0 {
                (max - min) / spread
            } else {
                0.0
            };
        }
    }
    sum / 81.0
}

#[test]
fn bloom_spreads_light_beyond_emissive_box() {
    let [glowing, plain] = renders(&[
        with(|l| l.bloom_strength = 0.8),
        with(|l| l.bloom_strength = 0.0),
    ])
    .try_into()
    .unwrap();
    let ((left, top), (_, bottom)) = screen_bounds(&demo_room(), "box_emissive");
    let (x, y) = (left - 12, (top + bottom) / 2);
    let (a, b) = (luma(glowing.get_pixel(x, y)), luma(plain.get_pixel(x, y)));
    assert!(a > b, "luma {a} with bloom, {b} without");
}

#[test]
fn tilt_zero_is_sharp_everywhere() {
    let looks: Vec<Look> = [0.0, 5.0, 12.0, 100.0]
        .map(|focus| {
            with(|l| {
                l.tilt = 0.0;
                l.focus = focus;
            })
        })
        .into();
    let images = renders(&looks);
    for img in &images[1..] {
        let d = compare_images(&images[0], img).unwrap();
        assert!(d.max_channel_delta <= 1, "{d:?}");
    }
}

#[test]
fn tilt_blurs_bottom_band_only_when_on() {
    let look = |tilt| {
        with(|l| {
            l.tilt = tilt;
            l.focus = desenho_dia().focus;
        })
    };
    let [sharp, tilted] = renders(&[look(0.0), look(1.0)]).try_into().unwrap();
    let drop = |rows: std::ops::Range<u32>| {
        1.0 - high_frequency_energy(&tilted, rows.clone()) / high_frequency_energy(&sharp, rows)
    };
    let bottom = drop(H * 4 / 5..H);
    let centre = drop(H / 3..H * 2 / 3);
    assert!(bottom > 0.5, "bottom fifth lost {bottom} of its detail");
    assert!(centre < 0.1, "central third lost {centre} of its detail");
}

#[test]
fn tilt_is_continuous() {
    let look = |tilt| {
        with(|l| {
            l.tilt = tilt;
            l.focus = desenho_dia().focus;
        })
    };
    let images = renders(&[look(0.0), look(0.5), look(0.52), look(1.0)]);
    let step = mean_abs_difference(&images[1], &images[2]);
    let whole = mean_abs_difference(&images[0], &images[3]);
    assert!(
        step < whole / 20.0,
        "0.50 to 0.52 moved {step}, 0 to 1 moved {whole}"
    );
}

#[test]
fn real_grade_desaturates_but_spares_drawn() {
    let [full, real] = renders(&[with(|l| l.real = 0.0), with(|l| l.real = 1.0)])
        .try_into()
        .unwrap();
    let room = demo_room();
    for name in ["box_red", "box_green"] {
        let (before, after) = (
            saturation_of(&full, &room, name),
            saturation_of(&real, &room, name),
        );
        assert!(
            after < 0.7 * before,
            "{name}: saturation {before} became {after}"
        );
    }
    let (before, after) = (
        saturation_of(&full, &room, "box_drawn"),
        saturation_of(&real, &room, "box_drawn"),
    );
    assert!(
        (after - before).abs() < 0.05 * before,
        "drawn box: saturation {before} became {after}"
    );
}

/// An unlit floor filling the frame, seen from straight above, whose radiance
/// rises from left to right from the end of Hejl's toe at 0.004 to where the
/// curve gives 0.02 on screen.
fn dark_ramp_room() -> RoomPack {
    let mut room = demo_room();
    room.draws.truncate(1);
    room.lights.clear();
    room.sun = None;
    room.ambient = Ambient { sh9: [[0.0; 3]; 9] };
    room.background = Background {
        top: [0.0; 3],
        bottom: [0.0; 3],
    };
    let floor = &mut room.materials[0];
    floor.base_color_texture = None;
    floor.shading = Shading::Unlit;
    floor.base_color = [0.002494, 0.002494, 0.002494, 1.0];
    floor.emissive = [0.004; 3];
    for v in &mut room.meshes[0].vertices {
        let t = if v.pos[0] > 0.0 { 255 } else { 0 };
        v.color = [t, t, t, 255];
    }
    let fov_y_deg: f32 = 40.0;
    let aspect = W as f32 / H as f32;
    room.camera = RoomCamera {
        fov_y_deg,
        pitch_deg: 90.0,
        // The 10 m floor spans the frame's width exactly.
        distance: 5.0 / ((fov_y_deg / 2.0).to_radians().tan() * aspect),
        focus: [0.0; 3],
    };
    room.look = Look {
        exposure: 1.0,
        bloom_strength: 0.0,
        real: 0.0,
        tilt: 0.0,
        ..room.look
    };
    room
}

#[test]
fn dither_breaks_banding() {
    let room = dark_ramp_room();
    let img = render_to_image(&room, &camera_of(&room), W, H, 0.0).unwrap();
    let row: Vec<[u8; 3]> = (0..W)
        .map(|x| {
            let p = img.get_pixel(x, H / 2);
            [p[0], p[1], p[2]]
        })
        .collect();
    let last = row[W as usize - 1];
    assert!(
        last.iter().all(|&c| (4..=6).contains(&c)),
        "ramp ends at {last:?}"
    );
    let mut distinct = row.clone();
    distinct.sort();
    distinct.dedup();
    let longest_run = row
        .chunk_by(|a, b| a == b)
        .map(|run| run.len())
        .max()
        .unwrap();
    assert!(distinct.len() > 8, "{} distinct values", distinct.len());
    assert!(longest_run <= 160, "{longest_run} px of one value");
}

fn golden_path(name: &str) -> String {
    format!("{GOLDEN_DIR}/demo_post_{name}.png")
}

fn render_look(look: Look) -> RgbaImage {
    let mut room = demo_room();
    room.look = look;
    render_to_image(&room, &camera_of(&room), W, H, 0.0).unwrap()
}

fn golden_looks() -> [(&'static str, Look); 2] {
    [("real_noite", real_noite()), ("desenho_dia", desenho_dia())]
}

#[test]
fn matches_post_goldens() {
    for (name, look) in golden_looks() {
        let img = render_look(look);
        let golden = image::open(golden_path(name)).unwrap().to_rgba8();
        let d = compare_images(&img, &golden).unwrap();
        if !(d.max_channel_delta <= GOLDEN_MAX_DELTA
            || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO)
        {
            let actual = format!(
                "{}/demo_post_{name}.actual.png",
                env!("CARGO_TARGET_TMPDIR")
            );
            img.save(&actual).unwrap();
            panic!("{name}: {d:?}; this render is in {actual}");
        }
    }
}

#[test]
#[ignore = "rewrites tests/golden/demo_post_*.png"]
fn write_post_goldens() {
    for (name, look) in golden_looks() {
        render_look(look).save(golden_path(name)).unwrap();
    }
}

#[test]
fn render_still_creates_no_pipelines() {
    let room = demo_room();
    let mut h = headless(&room);
    let pipelines = h.renderer().pipelines_created();
    let buffers = h.renderer().buffers_created();
    for look in [real_noite(), desenho_dia(), room.look.clone()] {
        h.renderer().set_look(&look);
        h.render(&camera_of(&room), 0.0);
    }
    assert_eq!(h.renderer().pipelines_created(), pipelines);
    assert_eq!(h.renderer().buffers_created(), buffers);
}
