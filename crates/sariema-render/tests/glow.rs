// SPDX-License-Identifier: Apache-2.0

//! Highlights: a node glowing in its own colour, the albedo times the
//! vertex colour, so a dark piece glows in its hue and none turns white;
//! a glow of zero draws exactly as before, and no pipeline or buffer is
//! made for it.

use glam::Vec3;
use image::{Rgba, RgbaImage};
use sariema_core::{Camera, room_camera};
use sariema_format::demo::demo_room;
use sariema_format::{Aabb, Draw, Material, Node, RoomPack};
use sariema_render::{Headless, Quality, compare_images};

mod common;
use common::downlevel;

const W: u32 = 480;
const H: u32 = 270;

const DARK: usize = 0;
const LIGHT: usize = 1;
const BLUE: usize = 2;

fn camera(room: &RoomPack) -> Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

fn place(x: f32, z: f32) -> [f32; 16] {
    glam::Mat4::from_scale_rotation_translation(
        Vec3::splat(0.8),
        glam::Quat::IDENTITY,
        Vec3::new(x, 0.0, z),
    )
    .to_cols_array()
}

/// The demo room with a dark brown box, brown by its vertex colours on a
/// white material, and a cream one by its material, in front; nodes for
/// them and for the blue box.
fn room() -> RoomPack {
    let mut room = demo_room();
    let mut brown = room.meshes[1].clone();
    for v in &mut brown.vertices {
        v.color = [40, 22, 10, 255];
    }
    room.meshes.push(brown);
    let white = room.materials.len() as u32;
    let base = room.materials[1].clone();
    room.materials.push(Material {
        name: "white".into(),
        base_color: [1.0, 1.0, 1.0, 1.0],
        ..base.clone()
    });
    room.materials.push(Material {
        name: "cream".into(),
        base_color: [0.85, 0.78, 0.62, 1.0],
        ..base
    });
    let blue = (room.draws.iter())
        .position(|d| d.name.as_deref() == Some("box_blue"))
        .unwrap() as u32;
    let template = room.draws[blue as usize].clone();
    let mut add = |name: &str, mesh: u32, material: u32, transform: [f32; 16]| {
        let mut d = Draw {
            name: Some(name.into()),
            mesh,
            material,
            transform,
            ..template.clone()
        };
        d.bounds = Aabb::around(&room.meshes[mesh as usize], &d.transform);
        room.draws.push(d);
        room.draws.len() as u32 - 1
    };
    let dark = add("dark", 2, white, place(-1.2, 3.0));
    let light = add("light", 1, white + 1, place(1.2, 3.0));
    let node = |name: &str, draw: u32| Node {
        name: name.into(),
        pivot: glam::Mat4::IDENTITY.to_cols_array(),
        draws: vec![draw],
        visible: true,
        hinge: None,
    };
    room.nodes = vec![node("dark", dark), node("light", light), node("blue", blue)];
    room
}

fn headless(quality: Quality, room: &RoomPack) -> Headless {
    let mut h = Headless::new(W, H, quality).unwrap();
    h.renderer().load_room(room).unwrap();
    h
}

/// Where `p` lands on the image.
fn pixel(camera: &Camera, p: Vec3) -> (u32, u32) {
    let clip = camera.proj * camera.view * p.extend(1.0);
    let ndc = clip.truncate() / clip.w;
    (
        ((ndc.x * 0.5 + 0.5) * W as f32) as u32,
        ((0.5 - ndc.y * 0.5) * H as f32) as u32,
    )
}

/// The mean colour around where `p` lands.
fn at(img: &RgbaImage, room: &RoomPack, p: Vec3) -> [f32; 3] {
    let (cx, cy) = pixel(&camera(room), p);
    let mut sum = [0.0; 3];
    for dy in 0..5 {
        for dx in 0..5 {
            let Rgba([r, g, b, _]) = *img.get_pixel(cx + dx - 2, cy + dy - 2);
            sum[0] += r as f32 / 25.0;
            sum[1] += g as f32 / 25.0;
            sum[2] += b as f32 / 25.0;
        }
    }
    sum
}

/// The middle of the front face of the box standing at `x`, `z`.
fn front(img: &RgbaImage, room: &RoomPack, x: f32, z: f32) -> [f32; 3] {
    at(img, room, Vec3::new(x, 0.4, z + 0.4))
}

fn luma([r, g, b]: [f32; 3]) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Max minus min over max: 0 for grey or white.
fn saturation(c: [f32; 3]) -> f32 {
    let (hi, lo) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    if hi <= 0.0 { 0.0 } else { (hi - lo) / hi }
}

/// The hue in degrees, by the angle of the colour around the grey axis.
fn hue([r, g, b]: [f32; 3]) -> f32 {
    let (x, y) = (2.0 * r - g - b, 3f32.sqrt() * (g - b));
    y.atan2(x).to_degrees()
}

fn hue_apart(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = (hue(a) - hue(b)).abs() % 360.0;
    d.min(360.0 - d)
}

#[test]
fn a_glowing_node_brightens_in_its_own_colour_and_never_turns_white() {
    for quality in [Quality::high(), downlevel()] {
        let room = room();
        let mut h = headless(quality.clone(), &room);
        let before = h.render(&camera(&room), 0.0);
        // A light glow, as a game shows something within reach.
        for node in [DARK, LIGHT, BLUE] {
            assert!(h.renderer().set_node_glow(0, node, 0.25, 0.6));
        }
        assert!(!h.renderer().set_node_glow(0, 9, 0.25, 0.6), "no node 9");
        let after = h.render(&camera(&room), 0.0);

        let (dark0, dark1) = (
            front(&before, &room, -1.2, 3.0),
            front(&after, &room, -1.2, 3.0),
        );
        assert!(
            luma(dark1) > luma(dark0) + 25.0,
            "{quality:?}: {dark0:?} → {dark1:?}"
        );
        assert!(
            hue_apart(dark0, dark1) < 12.0,
            "still brown: {dark0:?} → {dark1:?}"
        );
        assert!(
            saturation(dark1) > 0.3,
            "not washed out: {dark0:?} → {dark1:?}"
        );

        let (light0, light1) = (
            front(&before, &room, 1.2, 3.0),
            front(&after, &room, 1.2, 3.0),
        );
        assert!(luma(light1) > luma(light0) + 5.0, "{light0:?} → {light1:?}");
        assert!(
            hue_apart(light0, light1) < 12.0,
            "still cream: {light0:?} → {light1:?}"
        );
        assert!(light1[0] >= light1[2] + 10.0, "not white: {light1:?}");

        let (blue0, blue1) = (
            front(&before, &room, 2.5, -1.0),
            front(&after, &room, 2.5, -1.0),
        );
        let gain = [0, 1, 2].map(|c| blue1[c] - blue0[c]);
        assert!(gain[2] > gain[0], "blue gains more blue than red: {gain:?}");
        assert!(
            hue_apart(blue0, blue1) < 12.0,
            "still blue: {blue0:?} → {blue1:?}"
        );
    }
}

#[test]
fn a_rim_glow_lights_faces_seen_edge_on_more_than_faces_seen_head_on() {
    let room = room();
    let mut h = headless(Quality::high(), &room);
    let before = h.render(&camera(&room), 0.0);
    let gain = |h: &mut Headless, (fill, rim): (f32, f32)| {
        h.renderer().set_node_glow(0, DARK, fill, rim);
        let after = h.render(&camera(&room), 0.0);
        let top = Vec3::new(-1.2, 0.8, 3.0);
        (
            luma(front(&after, &room, -1.2, 3.0)) - luma(front(&before, &room, -1.2, 3.0)),
            luma(at(&after, &room, top)) - luma(at(&before, &room, top)),
        )
    };
    // The camera looks down at about 45°: the top faces it more than the front.
    let (front_rim, top_rim) = gain(&mut h, (0.0, 1.5));
    assert!(
        front_rim > 2.0 * top_rim.max(0.5),
        "{front_rim} against {top_rim}"
    );
    let (front_fill, top_fill) = gain(&mut h, (0.3, 0.0));
    assert!(
        top_fill > 0.5 * front_fill,
        "{front_fill} against {top_fill}"
    );
}

#[test]
fn a_glow_back_at_zero_draws_exactly_as_before_and_makes_nothing_new() {
    for quality in [Quality::high(), downlevel()] {
        let room = room();
        let mut h = headless(quality.clone(), &room);
        h.render(&camera(&room), 0.0);
        let before = h.render(&camera(&room), 0.0);
        let (pipelines, buffers) = (
            h.renderer().pipelines_created(),
            h.renderer().buffers_created(),
        );
        for k in 0..5 {
            h.renderer()
                .set_node_glow(0, DARK, 0.1 * k as f32, 0.2 * k as f32);
            h.render(&camera(&room), 0.0);
        }
        h.renderer().set_node_glow(0, DARK, 0.0, 0.0);
        let after = h.render(&camera(&room), 0.0);
        assert_eq!(h.renderer().pipelines_created(), pipelines, "{quality:?}");
        assert_eq!(h.renderer().buffers_created(), buffers, "{quality:?}");
        assert_eq!(
            compare_images(&before, &after).unwrap().max_channel_delta,
            0
        );
    }
}

#[test]
fn a_moved_node_keeps_its_glow() {
    let room = room();
    let mut h = headless(Quality::high(), &room);
    h.renderer().set_node_glow(0, DARK, 0.5, 1.0);
    let offset = glam::Mat4::from_translation(Vec3::new(0.3, 0.0, 0.0));
    h.renderer().set_node_offset(0, DARK, offset);
    let lit = h.render(&camera(&room), 0.0);
    h.renderer().set_node_glow(0, DARK, 0.0, 0.0);
    let unlit = h.render(&camera(&room), 0.0);
    assert!(compare_images(&lit, &unlit).unwrap().max_channel_delta > 20);
}
