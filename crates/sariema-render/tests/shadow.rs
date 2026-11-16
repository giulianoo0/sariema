// SPDX-License-Identifier: Apache-2.0

use glam::{Vec3, vec3};
use image::RgbaImage;
use sariema_core::{Camera, room_camera};
use sariema_format::demo::demo_room;
use sariema_format::{
    Aabb, Ambient, Light, LightKind, Mobility, RoomCamera, RoomPack, Sun, hemisphere_sh9,
};
use sariema_render::{
    GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, compare_images, render_to_image,
};

const W: u32 = 960;
const H: u32 = 540;
const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/golden/demo_shadow.png"
);
const RED_BOX_TOP: Vec3 = vec3(-2.5, 1.0, -1.0);

fn camera_of(room: &RoomPack) -> Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

fn headless(quality: Quality, room: &RoomPack) -> Headless {
    let mut h = Headless::new(W, H, quality).unwrap();
    h.renderer().load_room(room).unwrap();
    h
}

fn render(quality: Quality, room: &RoomPack) -> RgbaImage {
    headless(quality, room).render(&camera_of(room), 0.0)
}

fn with_max_shadowed(max_shadowed_lights: u32) -> Quality {
    Quality {
        max_shadowed_lights,
        ..Quality::high()
    }
}

/// Mean luma of the 5×5 pixels around where `world` lands on screen.
fn luma_at(img: &RgbaImage, camera: &Camera, world: Vec3) -> f32 {
    let ndc = (camera.proj * camera.view).project_point3(world);
    let x = ((ndc.x * 0.5 + 0.5) * W as f32) as i32;
    let y = ((0.5 - ndc.y * 0.5) * H as f32) as i32;
    let mut sum = 0.0;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let p = img.get_pixel((x + dx) as u32, (y + dy) as u32);
            sum += (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0;
        }
    }
    sum / 25.0
}

/// Where the ray from `light` through `p` meets the floor.
fn on_floor(light: Vec3, p: Vec3) -> Vec3 {
    light + (p - light) * (light.y / (light.y - p.y))
}

/// `p` mirrored across the vertical plane through a spot's position and target,
/// where the spot's cone gives the same light.
fn mirror_across_spot(p: Vec3, position: Vec3, target: Vec3) -> Vec3 {
    let along = (target - position).with_y(0.0).normalize();
    let normal = vec3(-along.z, 0.0, along.x);
    p - 2.0 * (p - position).dot(normal) * normal
}

fn spot(position: [f32; 3], target: [f32; 3], importance: f32) -> Light {
    Light {
        name: "spot".into(),
        kind: LightKind::Spot {
            target,
            cone_deg: 60.0,
        },
        color: [1.0, 0.9, 0.75],
        intensity: 60.0,
        position,
        range: 15.0,
        flicker: None,
        casts_shadow: true,
        mobility: Mobility::Static,
        importance,
    }
}

/// The demo room's floor, made plain white, with its red box and spot, seen
/// from straight above the box. Its ambient is a fifth of the demo's, so the
/// luma in a shadow shows how much of the light it blocks.
fn box_room() -> RoomPack {
    let mut room = demo_room();
    room.draws.truncate(2);
    room.materials[0].base_color_texture = None;
    room.lights.truncate(1);
    room.ambient = Ambient {
        sh9: hemisphere_sh9([0.6, 0.7, 0.85], [0.3, 0.25, 0.2], 0.1),
    };
    room.camera = RoomCamera {
        fov_y_deg: 40.0,
        pitch_deg: 90.0,
        distance: 12.0,
        focus: RED_BOX_TOP.with_y(0.0).into(),
    };
    room
}

fn spot_target(light: &Light) -> Vec3 {
    match light.kind {
        LightKind::Spot { target, .. } => target.into(),
        LightKind::Point => panic!("not a spot"),
    }
}

/// Luma behind the red box from `light`, over luma where the box casts no
/// shadow but the light is the same.
fn shadow_ratio(img: &RgbaImage, room: &RoomPack, light: &Light) -> f32 {
    let camera = camera_of(room);
    let position = Vec3::from(light.position);
    let behind = on_floor(position, RED_BOX_TOP);
    let beside = mirror_across_spot(behind, position, spot_target(light));
    luma_at(img, &camera, behind) / luma_at(img, &camera, beside)
}

#[test]
fn box_casts_shadow_on_floor() {
    let room = box_room();
    let ratio = shadow_ratio(&render(Quality::high(), &room), &room, &room.lights[0]);
    assert!(ratio <= 0.6, "shadowed / lit luma = {ratio}");
}

#[test]
fn shadow_maps_are_static() {
    let room = demo_room();
    let mut h = headless(Quality::high(), &room);
    h.render(&camera_of(&room), 0.0);
    let drawn = h.renderer().shadow_passes_rendered();
    assert_eq!(drawn, 1, "the spot's map is drawn on the first frame");
    for frame in 1..=5 {
        h.render(&camera_of(&room), frame as f32 / 60.0);
    }
    assert_eq!(h.renderer().shadow_passes_rendered(), drawn);
}

#[test]
fn toggling_a_caster_rerenders_once() {
    let room = demo_room();
    let mut h = headless(Quality::high(), &room);
    h.render(&camera_of(&room), 0.0);
    let drawn = h.renderer().shadow_passes_rendered();
    assert!(h.renderer().set_draw_visible("box_red", false));
    h.render(&camera_of(&room), 0.0);
    h.render(&camera_of(&room), 0.0);
    assert_eq!(h.renderer().shadow_passes_rendered(), drawn + 1);
}

#[test]
fn toggling_a_caster_out_of_range_rerenders_nothing() {
    let mut room = demo_room();
    let mut far = room.draws[1].clone();
    far.name = Some("far_box".into());
    far.transform = glam::Mat4::from_translation(vec3(40.0, 0.0, -40.0)).to_cols_array();
    far.bounds = Aabb::around(&room.meshes[far.mesh as usize], &far.transform);
    room.draws.push(far);
    let mut h = headless(Quality::high(), &room);
    h.render(&camera_of(&room), 0.0);
    let drawn = h.renderer().shadow_passes_rendered();
    assert!(h.renderer().set_draw_visible("far_box", false));
    h.render(&camera_of(&room), 0.0);
    assert_eq!(h.renderer().shadow_passes_rendered(), drawn);
}

#[test]
fn no_shadow_light_means_no_shadow_pass() {
    let mut room = demo_room();
    for light in &mut room.lights {
        light.casts_shadow = false;
    }
    room.sun = None;
    let mut h = headless(Quality::high(), &room);
    for frame in 0..3 {
        h.render(&camera_of(&room), frame as f32 / 60.0);
    }
    assert_eq!(h.renderer().shadow_passes_rendered(), 0);
    assert_eq!(h.renderer().last_frame_stats().shadows_skipped, 0);
}

#[test]
fn max_shadowed_lights_picks_the_highest_ranked() {
    let mut room = box_room();
    let box_base = [-2.5, 0.0, -1.0];
    // The first spot ranks higher by importance. Behind the box from either
    // spot, the other one's light is not blocked.
    room.lights = vec![
        spot([-3.0, 6.0, 3.0], box_base, 2.0),
        spot([2.0, 6.0, -1.0], box_base, 1.0),
    ];
    let camera = camera_of(&room);
    let behind: Vec<Vec3> = (room.lights.iter())
        .map(|l| on_floor(l.position.into(), RED_BOX_TOP))
        .collect();
    let lumas = |room: &RoomPack, max_shadowed: u32| {
        let mut h = headless(with_max_shadowed(max_shadowed), room);
        let img = h.render(&camera, 0.0);
        let lumas: Vec<f32> = behind.iter().map(|&p| luma_at(&img, &camera, p)).collect();
        (lumas, h.renderer().last_frame_stats().shadows_skipped)
    };
    let (unshadowed, skipped) = lumas(&room, 0);
    assert_eq!(skipped, 2);
    // Fully shadowed from spot i, the floor behind the box gets only the other spot's light.
    let other_only: Vec<f32> = (0..2)
        .map(|i| {
            let mut without = room.clone();
            without.lights.remove(i);
            lumas(&without, 0).0[i]
        })
        .collect();
    let near = |a: f32, b: f32| (a - b).abs() <= 1.0 / 255.0;
    let shadowed = |lumas: &[f32]| -> Vec<bool> {
        (0..2)
            .map(|i| {
                assert!(near(lumas[i], other_only[i]) || near(lumas[i], unshadowed[i]));
                near(lumas[i], other_only[i])
            })
            .collect()
    };

    let (one, skipped) = lumas(&room, 1);
    assert_eq!((shadowed(&one), skipped), (vec![true, false], 1), "{one:?}");
    let (two, skipped) = lumas(&room, 2);
    assert_eq!((shadowed(&two), skipped), (vec![true, true], 0), "{two:?}");
}

#[test]
fn halving_the_atlas_keeps_the_shadow() {
    let room = box_room();
    let full = render(Quality::high(), &room);
    let half = render(
        Quality {
            shadow_atlas_size: Quality::high().shadow_atlas_size / 2,
            ..Quality::high()
        },
        &room,
    );
    let d = compare_images(&full, &half).unwrap();
    assert!(d.differing_pixels_ratio <= GOLDEN_MAX_RATIO, "{d:?}");
    let ratio = shadow_ratio(&half, &room, &room.lights[0]);
    assert!(ratio <= 0.6, "shadowed / lit luma = {ratio}");
}

#[test]
fn sun_casts_shadow_on_floor() {
    let mut room = box_room();
    room.lights.clear();
    let towards_sun = vec3(-1.0, 1.0, 1.0).normalize();
    room.sun = Some(Sun {
        direction: towards_sun.into(),
        color: [1.0; 3],
        intensity: 2.0,
        casts_shadow: true,
        mobility: Mobility::Static,
    });
    let img = render(Quality::high(), &room);
    let camera = camera_of(&room);
    let behind = on_floor(RED_BOX_TOP + towards_sun * 10.0, RED_BOX_TOP);
    let beside = RED_BOX_TOP.with_y(0.0) + (RED_BOX_TOP.with_y(0.0) - behind);
    let ratio = luma_at(&img, &camera, behind) / luma_at(&img, &camera, beside);
    assert!(ratio <= 0.6, "shadowed / lit luma = {ratio}");
}

#[test]
fn point_light_casts_shadow_downwards() {
    let mut room = box_room();
    room.lights = vec![Light {
        kind: LightKind::Point,
        position: [-2.5, 3.0, 1.5],
        ..spot([0.0; 3], [0.0; 3], 1.0)
    }];
    let behind = on_floor(room.lights[0].position.into(), RED_BOX_TOP);
    let camera = camera_of(&room);
    let lit = luma_at(&render(with_max_shadowed(0), &room), &camera, behind);
    let shadowed = luma_at(&render(Quality::high(), &room), &camera, behind);
    assert!(shadowed <= 0.6 * lit, "{shadowed} against {lit} unshadowed");
}

fn sunny_demo_room() -> RoomPack {
    let mut room = demo_room();
    room.sun = Some(Sun {
        direction: vec3(-0.6, 0.6, -0.4).normalize().into(),
        color: [1.0, 0.85, 0.7],
        intensity: 1.2,
        casts_shadow: true,
        mobility: Mobility::Static,
    });
    room
}

fn render_sunny() -> RgbaImage {
    let room = sunny_demo_room();
    render_to_image(&room, &camera_of(&room), W, H, 0.0).unwrap()
}

#[test]
fn matches_shadow_golden() {
    let img = render_sunny();
    let golden = image::open(GOLDEN).unwrap().to_rgba8();
    let d = compare_images(&img, &golden).unwrap();
    if !(d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO) {
        let actual = concat!(env!("CARGO_TARGET_TMPDIR"), "/demo_shadow.actual.png");
        img.save(actual).unwrap();
        panic!("{d:?}; this render is in {actual}");
    }
}

#[test]
#[ignore = "rewrites tests/golden/demo_shadow.png"]
fn write_shadow_golden() {
    render_sunny().save(GOLDEN).unwrap();
}
