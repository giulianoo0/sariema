// SPDX-License-Identifier: Apache-2.0

use glam::{Mat4, Vec3, vec3};
use image::RgbaImage;
use sariema_core::{Camera, room_camera};
use sariema_format::demo::demo_room;
use sariema_format::{Aabb, Ambient, Background, Look, RoomCamera, RoomPack, Shading};
use sariema_render::{
    GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, RenderPath, compare_images,
    render_to_image,
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
        ao_strength: Look::DEFAULT_AO_STRENGTH,
        ao_radius_m: Look::DEFAULT_AO_RADIUS_M,
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
        ao_strength: Look::DEFAULT_AO_STRENGTH,
        ao_radius_m: Look::DEFAULT_AO_RADIUS_M,
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

/// The demo floor alone, unlit and untextured, seen from straight above
/// `distance` m away, with no light and a black background.
fn unlit_floor_room(distance: f32) -> RoomPack {
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
    room.camera = RoomCamera {
        fov_y_deg: 40.0,
        pitch_deg: 90.0,
        distance,
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

/// Half the width, in metres, that the room camera sees at `distance`.
fn half_width_seen(distance: f32, aspect: f32) -> f32 {
    distance * 20.0_f32.to_radians().tan() * aspect
}

/// An unlit floor filling the frame whose radiance rises from left to right
/// from the end of Hejl's toe at 0.004 to where the curve gives 0.02 on screen.
fn dark_ramp_room() -> RoomPack {
    let aspect = W as f32 / H as f32;
    // The 10 m floor spans the frame's width exactly.
    let mut room = unlit_floor_room(5.0 / half_width_seen(1.0, aspect));
    let floor = &mut room.materials[0];
    floor.base_color = [0.002494, 0.002494, 0.002494, 1.0];
    floor.emissive = [0.004; 3];
    for v in &mut room.meshes[0].vertices {
        let t = if v.pos[0] > 0.0 { 255 } else { 0 };
        v.color = [t, t, t, 255];
    }
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

/// The composite's tone curve, in 0..255.
fn hejl(c: f32) -> f32 {
    let x = (c - 0.004).max(0.0);
    255.0 * x * (6.2 * x + 0.5) / (x * (6.2 * x + 1.7) + 0.06)
}

/// Where the middle row of `img` first rises through `level`, in pixels with
/// sub-pixel precision.
fn rising_edge(img: &RgbaImage, level: f32) -> f32 {
    let row: Vec<f32> = (0..img.width())
        .map(|x| luma(img.get_pixel(x, img.height() / 2)))
        .collect();
    let x = row.iter().position(|&l| l >= level).unwrap();
    x as f32 - 0.5 + (level - row[x - 1]) / (row[x] - row[x - 1])
}

fn edge_shift_at(width: u32, height: u32) -> (f32, f32) {
    let aspect = width as f32 / height as f32;
    let distance = 6.0;
    let radiance = 0.2;
    // Everything is 6 m away, so with focus 0 the whole frame blurs by `tilt`.
    let mut room = unlit_floor_room(distance);
    room.materials[0].base_color = [radiance, radiance, radiance, 1.0];
    // The floor's left side, a sharp vertical edge, at 90% of the frame's width.
    let left = 0.8 * half_width_seen(distance, aspect);
    let floor = &mut room.draws[0];
    floor.transform = Mat4::from_translation(vec3(left + 5.0, 0.0, 0.0)).to_cols_array();
    floor.bounds = Aabb::around(&room.meshes[0], &floor.transform);
    room.look.focus = 0.0;
    let camera = room_camera(&room.camera, aspect);
    let mut h = Headless::new(width, height, Quality::high()).unwrap();
    h.renderer().load_room(&room).unwrap();
    // Blurring mixes radiance, so the edge is where half the floor's radiance shows.
    let level = hejl(radiance * 0.5 * room.look.exposure);
    let sharp = rising_edge(&h.render(&camera, 0.0), level);
    room.look.tilt = 1.0;
    h.renderer().set_look(&room.look);
    (sharp, rising_edge(&h.render(&camera, 0.0), level))
}

#[test]
fn blurred_levels_line_up_with_the_scene_at_odd_sizes() {
    let width = 961;
    let (sharp, blurred) = edge_shift_at(width, 541);
    assert!((sharp - 0.9 * width as f32).abs() < 1.0, "edge at {sharp}");
    // Levels stretched by 960/961 would move this edge about 0.9 px.
    assert!(
        (blurred - sharp).abs() <= 0.5,
        "edge moved from {sharp} to {blurred}"
    );
}

/// Full-screen passes after the scene for a look of `tilt`, `bloom_strength`
/// and `ao_strength`, at `quality`.
fn post_passes(quality: Quality, tilt: f32, bloom_strength: f32, ao_strength: f32) -> u32 {
    let room = demo_room();
    let mut h = Headless::new(W, H, quality).unwrap();
    h.renderer().load_room(&room).unwrap();
    h.renderer().set_look(&with(|l| {
        l.tilt = tilt;
        l.bloom_strength = bloom_strength;
        l.ao_strength = ao_strength;
    }));
    h.render(&camera_of(&room), 0.0);
    h.renderer().last_frame_stats().post_passes
}

#[test]
fn effects_turned_off_draw_no_passes() {
    let levels = Quality::high().bloom_levels;
    // The bloom and the miniature blur start from the scene reduced to 1/2 and 1/4.
    let reduce = 2;
    // One depth resolve serves the miniature blur and the occlusion.
    let resolve = 1;
    // The estimate, then its two-pass blur.
    let ao = 1 + 2;
    let composite = 1;
    // Bright and blurred at 1/4 in two passes, then each smaller level
    // reduced and blurred in `per_level` passes; the 1/2 and 1/4 levels
    // blurred in `blur` passes each.
    for (one_pass_max, per_level, blur) in [(0, 3, 2), (u32::MAX, 1, 1)] {
        let quality = Quality {
            blur_one_pass_max_texels: one_pass_max,
            ..Quality::high()
        };
        let bloom = 2 + per_level * (levels - 1);
        let tilt = 2 * blur;
        let passes = |tilt, bloom, ao| post_passes(quality.clone(), tilt, bloom, ao);
        assert_eq!(passes(0.0, 0.0, 0.0), composite);
        assert_eq!(passes(0.0, 0.3, 0.0), reduce + bloom + composite);
        assert_eq!(passes(1.0, 0.0, 0.0), reduce + resolve + tilt + composite);
        assert_eq!(
            passes(1.0, 0.3, 0.0),
            reduce + bloom + resolve + tilt + composite
        );
        assert_eq!(passes(0.0, 0.0, 0.5), resolve + ao + composite);
        assert_eq!(
            passes(1.0, 0.0, 0.5),
            reduce + resolve + tilt + ao + composite
        );
        assert_eq!(
            passes(1.0, 0.3, 0.5),
            reduce + bloom + resolve + tilt + ao + composite
        );
        let no_occlusion = Quality {
            ao_samples: 0,
            ..quality.clone()
        };
        assert_eq!(post_passes(no_occlusion.clone(), 0.0, 0.0, 0.5), composite);
        assert_eq!(
            post_passes(no_occlusion, 1.0, 0.0, 0.5),
            reduce + resolve + tilt + composite
        );
    }
}

#[test]
fn one_pass_blurs_match_two_pass_blurs() {
    let room = demo_room();
    let look = with(|l| {
        l.tilt = 1.0;
        l.bloom_strength = 0.8;
        l.bloom_threshold = 0.2;
    });
    let render = |one_pass_max| {
        let quality = Quality {
            blur_one_pass_max_texels: one_pass_max,
            ..Quality::high()
        };
        let mut h = Headless::new(W, H, quality).unwrap();
        h.renderer().load_room(&room).unwrap();
        h.renderer().set_look(&look);
        h.render(&camera_of(&room), 0.0)
    };
    let (two, one) = (render(0), render(u32::MAX));
    assert_ne!(two, one, "the blurs ran in one pass");
    // Only the intermediate level's half-float rounding is gone.
    let d = compare_images(&two, &one).unwrap();
    assert!(d.max_channel_delta <= 1, "{d:?}");
}

#[test]
fn skipping_passes_does_not_change_the_image() {
    let barely = 1e-6;
    let pairs = [
        (with(|l| l.tilt = 0.0), with(|l| l.tilt = barely)),
        (
            with(|l| l.bloom_strength = 0.0),
            with(|l| l.bloom_strength = barely),
        ),
        (
            with(|l| l.ao_strength = 0.0),
            with(|l| l.ao_strength = barely),
        ),
    ];
    for (skipped, drawn) in pairs {
        let [a, b] = renders(&[skipped, drawn]).try_into().unwrap();
        let d = compare_images(&a, &b).unwrap();
        assert!(d.max_channel_delta <= 1, "{d:?}");
    }
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
fn matches_post_goldens_on_the_downlevel_path() {
    for (name, look) in golden_looks() {
        let mut room = demo_room();
        room.look = look;
        let mut h = Headless::new(W, H, downlevel()).unwrap();
        h.renderer().load_room(&room).unwrap();
        let img = h.render(&camera_of(&room), 0.0);
        let label = format!("demo_post_{name}");
        matches_golden_on_the_downlevel_path(&img, &label, &golden_path(name));
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

/// Saves `img` among the downlevel path's renders and checks it against the golden `name`.
fn matches_golden_on_the_downlevel_path(img: &RgbaImage, name: &str, golden: &str) {
    let dir = concat!(env!("CARGO_TARGET_TMPDIR"), "/downlevel");
    std::fs::create_dir_all(dir).unwrap();
    img.save(format!("{dir}/{name}.png")).unwrap();
    let golden = image::open(golden).unwrap().to_rgba8();
    let d = compare_images(img, &golden).unwrap();
    assert!(
        d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO,
        "{name}: {d:?}"
    );
}

fn downlevel() -> Quality {
    Quality {
        path: RenderPath::Downlevel,
        ..Quality::high()
    }
}
