// SPDX-License-Identifier: Apache-2.0

use glam::{Mat4, Vec2, Vec3, vec2, vec3};
use image::RgbaImage;
use sariema_core::{Camera, room_camera};
use sariema_format::demo::demo_room;
use sariema_format::{Aabb, Ambient, Draw, Look, RoomCamera, RoomPack, hemisphere_sh9};
use sariema_render::{
    GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, RenderPath, compare_images,
};

const W: u32 = 960;
const H: u32 = 540;
const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/golden/demo_ao.png"
);

fn camera_of(room: &RoomPack) -> Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

fn luma(p: &image::Rgba<u8>) -> f32 {
    0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
}

fn headless(room: &RoomPack, quality: Quality) -> Headless {
    let mut h = Headless::new(W, H, quality).unwrap();
    h.renderer().load_room(room).unwrap();
    h
}

fn without_occlusion(look: &Look) -> Look {
    Look {
        ao_strength: 0.0,
        ..look.clone()
    }
}

/// `room` seen by `camera` with its look's occlusion, then without it.
fn on_and_off(room: &RoomPack, quality: Quality, camera: &Camera) -> (RgbaImage, RgbaImage) {
    let mut h = headless(room, quality);
    let on = h.render(camera, 0.0);
    h.renderer().set_look(&without_occlusion(&room.look));
    (on, h.render(camera, 0.0))
}

fn demo_with_strength(ao_strength: f32) -> RoomPack {
    let mut room = demo_room();
    room.look.ao_strength = ao_strength;
    room
}

/// Where `world` lands in `camera`'s image, in pixels from the top left corner.
fn to_pixel(camera: &Camera, world: Vec3) -> Vec2 {
    let ndc = (camera.proj * camera.view).project_point3(world);
    vec2(
        (ndc.x * 0.5 + 0.5) * W as f32,
        (0.5 - ndc.y * 0.5) * H as f32,
    )
}

/// Where the ray through the centre of pixel (x, y) of a `width × height`
/// image meets the plane y = 0.
fn floor_point(camera: &Camera, (width, height): (u32, u32), x: u32, y: u32) -> Option<Vec3> {
    let inverse = (camera.proj * camera.view).inverse();
    let ndc = vec2(
        (x as f32 + 0.5) / width as f32 * 2.0 - 1.0,
        1.0 - (y as f32 + 0.5) / height as f32 * 2.0,
    );
    let near = inverse.project_point3(ndc.extend(0.0));
    let far = inverse.project_point3(ndc.extend(1.0));
    let t = near.y / (near.y - far.y);
    (0.0..1.0).contains(&t).then(|| near.lerp(far, t))
}

fn mean_luma_at(img: &RgbaImage, camera: &Camera, points: &[Vec3]) -> f32 {
    let sum: f32 = points
        .iter()
        .map(|&p| {
            let at = to_pixel(camera, p);
            luma(img.get_pixel(at.x as u32, at.y as u32))
        })
        .sum();
    sum / points.len() as f32
}

/// Luma the occlusion took from each pixel, row by row.
fn darkening(on: &RgbaImage, off: &RgbaImage) -> Vec<f32> {
    on.pixels()
        .zip(off.pixels())
        .map(|(a, b)| luma(b) - luma(a))
        .collect()
}

/// `values`, one per pixel, interpolated at `at` in pixels.
fn bilinear(values: &[f32], at: Vec2) -> f32 {
    let p = at - 0.5;
    let (x, y) = (p.x.floor(), p.y.floor());
    let (fx, fy) = (p.x - x, p.y - y);
    let get = |dx: f32, dy: f32| values[((y + dy) as u32 * W + (x + dx) as u32) as usize];
    let top = get(0.0, 0.0) * (1.0 - fx) + get(1.0, 0.0) * fx;
    let bottom = get(0.0, 1.0) * (1.0 - fx) + get(1.0, 1.0) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Points 0.05 m apart along box_red's front, a unit cube standing at
/// (-2.5, 0, -1), at height `y` and depth `z`.
fn along_red_front(y: f32, z: f32) -> Vec<Vec3> {
    (0..=16)
        .map(|i| vec3(-2.9 + 0.05 * i as f32, y, z))
        .collect()
}

#[test]
fn box_base_and_floor_beside_it_darken() {
    let room = demo_room();
    let camera = camera_of(&room);
    let (on, off) = on_and_off(&room, Quality::high(), &camera);
    // The front face is at z = -0.5. The default strength takes about 15%
    // of the luma there.
    for (name, points) in [
        ("base of the box", along_red_front(0.06, -0.5)),
        ("floor 4 cm in front", along_red_front(0.0, -0.46)),
    ] {
        let (with, without) = (
            mean_luma_at(&on, &camera, &points),
            mean_luma_at(&off, &camera, &points),
        );
        assert!(
            with < 0.9 * without,
            "{name}: luma {with} with occlusion, {without} without"
        );
    }
}

#[test]
fn open_floor_and_background_are_untouched() {
    let room = demo_with_strength(1.0);
    let camera = camera_of(&room);
    let (on, off) = on_and_off(&room, Quality::high(), &camera);
    // Floor more than 1 m from every box.
    for centre in [vec3(-3.8, 0.0, 3.8), vec3(0.2, 0.0, 0.3)] {
        let (a, b) = (
            to_pixel(&camera, centre - vec3(0.15, 0.0, -0.15)),
            to_pixel(&camera, centre + vec3(0.15, 0.0, -0.15)),
        );
        for y in a.y as u32..b.y as u32 {
            for x in a.x as u32..b.x as u32 {
                let (p, q) = (on.get_pixel(x, y), off.get_pixel(x, y));
                let d = (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap();
                assert!(d <= 1, "floor at ({x}, {y}) near {centre} moved {d} levels");
            }
        }
    }
    // The top rows look past the floor's far edge at the background alone.
    for y in 0..20 {
        for x in 0..W {
            assert_eq!(on.get_pixel(x, y), off.get_pixel(x, y), "({x}, {y})");
        }
    }
}

/// A 3 × 0.1 × 3 m slab floating 2 m up with a unit box on it, flush with
/// its left side, over the demo floor or over nothing, lit by ambient light alone.
fn ledge_room(floor: bool) -> RoomPack {
    let mut room = demo_with_strength(1.0);
    room.lights.clear();
    room.look.bloom_strength = 0.0;
    let cube = &room.draws[1];
    let placed = |name: &str, transform: Mat4| Draw {
        name: Some(name.into()),
        transform: transform.to_cols_array(),
        bounds: Aabb::around(&room.meshes[cube.mesh as usize], &transform.to_cols_array()),
        ..cube.clone()
    };
    let slab = placed(
        "slab",
        Mat4::from_translation(vec3(0.0, 2.0, 0.0)) * Mat4::from_scale(vec3(3.0, 0.1, 3.0)),
    );
    let block = placed("block", Mat4::from_translation(vec3(-1.0, 2.1, 0.0)));
    let floor_draw = room.draws[0].clone();
    room.draws = vec![slab, block];
    if floor {
        room.draws.push(floor_draw);
    }
    room
}

/// Whether each pixel differs between `a` and `b`, row by row.
fn differs(a: &RgbaImage, b: &RgbaImage) -> Vec<bool> {
    a.pixels().zip(b.pixels()).map(|(p, q)| p != q).collect()
}

/// Pixels outside `mask` within `reach` pixels of it, by the larger of the
/// horizontal and vertical distance.
fn ring(mask: &[bool], reach: i32) -> Vec<(u32, u32)> {
    let inside = |x: i32, y: i32| {
        (0..W as i32).contains(&x)
            && (0..H as i32).contains(&y)
            && mask[(y * W as i32 + x) as usize]
    };
    let mut out = Vec::new();
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            if inside(x, y) {
                continue;
            }
            let near = (-reach..=reach).any(|dy| (-reach..=reach).any(|dx| inside(x + dx, y + dy)));
            if near {
                out.push((x as u32, y as u32));
            }
        }
    }
    out
}

fn assert_no_halo(floor: bool, quality: Quality) {
    let room = ledge_room(floor);
    let camera = camera_of(&room);
    let mut h = headless(&room, quality);
    let on = h.render(&camera, 0.0);
    h.renderer().set_look(&without_occlusion(&room.look));
    let off = h.render(&camera, 0.0);
    h.renderer().set_draw_visible("slab", false);
    h.renderer().set_draw_visible("block", false);
    let behind = h.render(&camera, 0.0);
    let ledge = differs(&off, &behind);
    let around = ring(&ledge, 8);
    assert!(
        around.len() > 1000,
        "{} pixels around the ledge",
        around.len()
    );
    for &(x, y) in &around {
        let (p, q) = (on.get_pixel(x, y), off.get_pixel(x, y));
        let d = (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap();
        assert!(d <= 1, "({x}, {y}) beside the ledge moved {d} levels");
    }
    // The crease where the box meets the slab reaches the silhouette.
    let lost = darkening(&on, &off);
    let open: Vec<bool> = ledge.iter().map(|&l| !l).collect();
    let shaded_at_edge = ring(&open, 8)
        .into_iter()
        .filter(|&(x, y)| lost[(y * W + x) as usize] >= 8.0)
        .count();
    assert!(
        shaded_at_edge >= 20,
        "{shaded_at_edge} shaded pixels at the silhouette"
    );
}

#[test]
fn no_halo_against_the_background() {
    assert_no_halo(false, Quality::high());
    assert_no_halo(false, Quality::low());
}

#[test]
fn no_halo_against_far_surfaces() {
    assert_no_halo(true, Quality::high());
    assert_no_halo(true, Quality::low());
}

#[test]
fn occlusion_holds_still_as_the_camera_moves() {
    let room = demo_with_strength(1.0);
    let a = camera_of(&room);
    let (on_a, off_a) = on_and_off(&room, Quality::high(), &a);
    let lost_a = darkening(&on_a, &off_a);
    // Floor 5 to 50 cm from box_red's front and right faces, seen by every camera.
    let near_red = |p: Vec3| {
        let front = (-3.0..=-2.0).contains(&p.x) && (-0.45..=0.0).contains(&p.z);
        let right = (-1.95..=-1.5).contains(&p.x) && (-1.5..=-0.5).contains(&p.z);
        front || right
    };
    // From under one occlusion texel to over one. Past a texel the camera
    // also sees more of box_red's right face, almost edge-on, which moves the
    // occlusion beside it.
    for (step, limit) in [(0.01, 4.0), (0.03, 6.0), (0.1, 7.0)] {
        let mut moved = room.camera.clone();
        moved.focus[0] += step;
        let b = room_camera(&moved, W as f32 / H as f32);
        let (on_b, off_b) = on_and_off(&room, Quality::high(), &b);
        let lost_b = darkening(&on_b, &off_b);
        let (mut worst, mut sum, mut count, mut deepest) = (0.0_f32, 0.0, 0, 0.0_f32);
        for y in 0..H {
            for x in 0..W {
                let Some(p) = floor_point(&b, (W, H), x, y).filter(|&p| near_red(p)) else {
                    continue;
                };
                let here = lost_b[(y * W + x) as usize];
                let jump = (bilinear(&lost_a, to_pixel(&a, p)) - here).abs();
                worst = worst.max(jump);
                sum += jump;
                count += 1;
                deepest = deepest.max(here);
            }
        }
        let mean = sum / count as f32;
        assert!(count > 1000, "{step} m: {count} floor pixels");
        assert!(
            deepest > 20.0,
            "{step} m: the occlusion took at most {deepest} levels"
        );
        assert!(
            worst <= limit && mean <= 1.2,
            "{step} m: jumps of up to {worst}, {mean} on average"
        );
    }
}

#[test]
fn radius_near_the_camera_is_held_by_the_quality() {
    let mut room = demo_with_strength(1.0);
    // 2 m from box_red's front face, the radius would span about 185 pixels.
    room.camera = RoomCamera {
        fov_y_deg: 40.0,
        pitch_deg: 45.0,
        distance: 2.0,
        focus: [-2.5, 0.0, -0.3],
    };
    let quality = Quality {
        ao_max_radius_texels: 4,
        ..Quality::high()
    };
    let camera = camera_of(&room);
    let reach_px = (quality.ao_max_radius_texels * quality.ao_divisor) as f32;
    let (on, off) = on_and_off(&room, quality, &camera);
    let lost = darkening(&on, &off);
    let pixels_per_m =
        |p: Vec3| camera.proj.y_axis.y * 0.5 * H as f32 / (camera.view * p.extend(1.0)).z.abs();
    let mut beyond = 0;
    let mut shaded = 0;
    for y in 0..H {
        for x in 0..W {
            let Some(p) = floor_point(&camera, (W, H), x, y) else {
                continue;
            };
            if !(-2.9..=-2.1).contains(&p.x) || p.z < -0.5 {
                continue;
            }
            let held = reach_px / pixels_per_m(p);
            let from_face = p.z + 0.5;
            let d = lost[(y * W + x) as usize];
            // The blur and the upsample spread it by a few texels more.
            if from_face > held + 0.05 && from_face < 0.5 {
                assert!(d <= 1.0, "floor {from_face} m from the box lost {d} levels");
                beyond += 1;
            } else if from_face < 0.5 * held && d >= 8.0 {
                shaded += 1;
            }
        }
    }
    assert!(
        beyond > 1000,
        "{beyond} floor pixels beyond the held radius"
    );
    assert!(shaded > 100, "{shaded} shaded pixels at the crease");
}

/// A long wall across the frame on an untextured floor, lit by ambient
/// light alone, so the crease in front of it reaches both sides.
fn wall_room() -> RoomPack {
    let mut room = demo_with_strength(1.0);
    room.lights.clear();
    room.ambient = Ambient {
        sh9: hemisphere_sh9([0.6, 0.7, 0.85], [0.3, 0.25, 0.2], 1.0),
    };
    room.look.bloom_strength = 0.0;
    room.materials[0].base_color_texture = None;
    let floor_transform = Mat4::from_scale(vec3(4.0, 1.0, 4.0)).to_cols_array();
    let floor = Draw {
        transform: floor_transform,
        bounds: Aabb::around(&room.meshes[0], &floor_transform),
        ..room.draws[0].clone()
    };
    let wall_transform = (Mat4::from_translation(vec3(0.0, 0.0, -1.0))
        * Mat4::from_scale(vec3(40.0, 1.0, 0.5)))
    .to_cols_array();
    let wall = Draw {
        transform: wall_transform,
        bounds: Aabb::around(&room.meshes[1], &wall_transform),
        ..room.draws[1].clone()
    };
    room.draws = vec![floor, wall];
    room
}

#[test]
fn occlusion_reaches_the_borders_at_awkward_sizes() {
    let (width, height) = (1366, 768);
    let room = wall_room();
    let camera = room_camera(&room.camera, width as f32 / height as f32);
    // The wall's front face is at z = -0.75; a line along x stays on one row.
    let ndc = (camera.proj * camera.view).project_point3(vec3(0.0, 0.0, -0.72));
    let crease_row = ((0.5 - ndc.y * 0.5) * height as f32) as u32;
    for divisor in [1, 4] {
        let quality = Quality {
            ao_divisor: divisor,
            ..Quality::high()
        };
        let mut h = Headless::new(width, height, quality).unwrap();
        h.renderer().load_room(&room).unwrap();
        let on = h.render(&camera, 0.0);
        h.renderer().set_look(&without_occlusion(&room.look));
        let off = h.render(&camera, 0.0);
        let lost = |x: u32| {
            (crease_row..crease_row + 3)
                .map(|y| luma(off.get_pixel(x, y)) - luma(on.get_pixel(x, y)))
                .sum::<f32>()
                / 3.0
        };
        let inside = |columns: std::ops::Range<u32>| {
            columns.clone().map(lost).sum::<f32>() / columns.len() as f32
        };
        for (edge, near) in [
            (0, inside(4..16)),
            (width - 1, inside(width - 16..width - 4)),
        ] {
            assert!(
                near > 10.0,
                "divisor {divisor}: {near} levels beside the wall"
            );
            assert!(
                lost(edge) > 0.7 * near,
                "divisor {divisor}: column {edge} lost {} levels, {near} further in",
                lost(edge)
            );
        }
        for (x, y) in (0..height)
            .flat_map(|y| [(0, y), (width - 1, y)])
            .chain((0..width).flat_map(|x| [(x, 0), (x, height - 1)]))
        {
            let open = floor_point(&camera, (width, height), x, y).is_some_and(|p| p.z > 0.25);
            let (p, q) = (on.get_pixel(x, y), off.get_pixel(x, y));
            let d = (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap();
            assert!(
                !open || d <= 1,
                "divisor {divisor}: open floor at ({x}, {y}) moved {d} levels"
            );
        }
    }
}

#[test]
fn unusable_radius_turns_occlusion_off() {
    let camera = camera_of(&demo_room());
    let mut off = headless(&demo_with_strength(0.0), Quality::high());
    let reference = off.render(&camera, 0.0);
    let passes = off.renderer().last_frame_stats().post_passes;
    for radius in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let mut room = demo_with_strength(1.0);
        room.look.ao_radius_m = radius;
        let mut h = headless(&room, Quality::high());
        let img = h.render(&camera, 0.0);
        assert_eq!(
            compare_images(&img, &reference).unwrap().max_channel_delta,
            0,
            "{radius}"
        );
        assert_eq!(
            h.renderer().last_frame_stats().post_passes,
            passes,
            "{radius}"
        );
    }
}

#[test]
fn zero_strength_is_the_same_as_occlusion_off() {
    let camera = camera_of(&demo_room());
    let mut zero = headless(&demo_with_strength(0.0), Quality::high());
    let off_tier = Quality {
        ao_samples: 0,
        ..Quality::high()
    };
    let mut off = headless(&demo_with_strength(1.0), off_tier);
    let (a, b) = (zero.render(&camera, 0.0), off.render(&camera, 0.0));
    assert_eq!(compare_images(&a, &b).unwrap().max_channel_delta, 0);
    let passes = |h: &mut Headless| h.renderer().last_frame_stats().post_passes;
    assert_eq!(passes(&mut zero), passes(&mut off));
}

fn render_golden_look() -> RgbaImage {
    render_golden_look_with(Quality::high())
}

fn render_golden_look_with(quality: Quality) -> RgbaImage {
    let room = demo_with_strength(1.0);
    headless(&room, quality).render(&camera_of(&room), 0.0)
}

#[test]
fn matches_occlusion_golden_on_the_downlevel_path() {
    let img = render_golden_look_with(downlevel());
    matches_golden_on_the_downlevel_path(&img, "demo_ao", GOLDEN);
}

#[test]
fn matches_occlusion_golden() {
    let img = render_golden_look();
    let golden = image::open(GOLDEN).unwrap().to_rgba8();
    let d = compare_images(&img, &golden).unwrap();
    if !(d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO) {
        let actual = concat!(env!("CARGO_TARGET_TMPDIR"), "/demo_ao.actual.png");
        img.save(actual).unwrap();
        panic!("{d:?}; this render is in {actual}");
    }
}

#[test]
#[ignore = "rewrites tests/golden/demo_ao.png"]
fn write_occlusion_golden() {
    render_golden_look().save(GOLDEN).unwrap();
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
