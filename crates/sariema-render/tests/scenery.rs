// SPDX-License-Identifier: Apache-2.0

//! Groups, piece states, nodes, several rooms drawn at once and rooms
//! uploaded a share at a time.

use glam::{Mat4, Vec3};
use image::RgbaImage;
use sariema_core::{Camera, room_camera};
use sariema_format::demo::demo_room;
use sariema_format::{
    Aabb, Draw, Group, Light, LightKind, Mobility, Node, Piece, PieceState, RoomPack,
};
use sariema_render::{Headless, Quality, compare_images};

mod common;
use common::downlevel;

const W: u32 = 480;
const H: u32 = 270;

fn camera(room: &RoomPack) -> Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

fn draw_index(room: &RoomPack, name: &str) -> u32 {
    (room.draws.iter())
        .position(|d| d.name.as_deref() == Some(name))
        .unwrap() as u32
}

/// The demo room with `west` holding the red box and the spot, and `rest`
/// everything else.
fn grouped() -> RoomPack {
    let mut room = demo_room();
    let red = draw_index(&room, "box_red");
    room.groups = vec![
        Group {
            name: "west".into(),
            on: true,
            draws: vec![red],
            lights: vec![0],
        },
        Group {
            name: "rest".into(),
            on: true,
            draws: (0..room.draws.len() as u32).filter(|&d| d != red).collect(),
            lights: vec![1],
        },
    ];
    room
}

fn headless(quality: Quality, room: &RoomPack) -> Headless {
    let mut h = Headless::new(W, H, quality).unwrap();
    h.renderer().load_room(room).unwrap();
    h
}

fn render(quality: Quality, room: &RoomPack) -> RgbaImage {
    headless(quality, room).render(&camera(room), 0.0)
}

fn max_delta(a: &RgbaImage, b: &RgbaImage) -> u8 {
    compare_images(a, b).unwrap().max_channel_delta
}

/// Every draw of `b` and every light, appended to `a`; `a` keeps its sky.
fn merge(a: &RoomPack, b: &RoomPack) -> RoomPack {
    let mut room = a.clone();
    let (meshes, materials, textures) = (
        a.meshes.len() as u32,
        a.materials.len() as u32,
        a.textures.len() as u32,
    );
    room.meshes.extend(b.meshes.iter().cloned());
    room.textures.extend(b.textures.iter().cloned());
    room.materials
        .extend(b.materials.iter().cloned().map(|mut m| {
            m.base_color_texture = m.base_color_texture.map(|t| t + textures);
            m
        }));
    room.draws.extend(b.draws.iter().cloned().map(|mut d| {
        d.mesh += meshes;
        d.material += materials;
        d
    }));
    room.lights.extend(b.lights.iter().cloned());
    room
}

/// A room 6 m in front of the demo's: one green box lit by a red point light.
fn neighbour() -> RoomPack {
    let mut room = demo_room();
    let green = draw_index(&room, "box_green") as usize;
    let mut draw: Draw = room.draws[green].clone();
    draw.name = Some("neighbour_box".into());
    draw.casts_shadow = false;
    let moved =
        Mat4::from_translation(Vec3::new(0.0, 0.0, 4.0)) * Mat4::from_cols_array(&draw.transform);
    draw.transform = moved.to_cols_array();
    draw.bounds = Aabb::around(&room.meshes[draw.mesh as usize], &draw.transform);
    room.draws = vec![draw];
    room.lights = vec![Light {
        name: "neighbour_light".into(),
        kind: LightKind::Point,
        color: [1.0, 0.2, 0.1],
        intensity: 6.0,
        position: [0.0, 1.5, 5.0],
        range: 4.0,
        flicker: None,
        casts_shadow: false,
        mobility: Mobility::Static,
        importance: 1.0,
    }];
    room.name = "neighbour".into();
    room
}

#[test]
fn a_group_switched_off_hides_its_draws_and_lights() {
    for quality in [Quality::high(), downlevel()] {
        let room = grouped();
        let mut h = headless(quality.clone(), &room);
        let on = h.render(&camera(&room), 0.0);
        let (pipelines, buffers) = (
            h.renderer().pipelines_created(),
            h.renderer().buffers_created(),
        );
        assert!(h.renderer().set_group(0, 0, false));
        assert!(!h.renderer().set_group(0, 9, false), "no group 9");
        let off = h.render(&camera(&room), 0.0);
        assert_eq!(h.renderer().pipelines_created(), pipelines);
        assert_eq!(h.renderer().buffers_created(), buffers);

        let mut without = room.clone();
        without.groups[0].on = false;
        let loaded_off = render(quality.clone(), &without);
        assert_eq!(
            max_delta(&off, &loaded_off),
            0,
            "switching off matches loading off"
        );
        assert!(
            max_delta(&on, &off) > 40,
            "the red box and the spot are gone"
        );

        assert!(h.renderer().set_group(0, 0, true));
        let back = h.render(&camera(&room), 0.0);
        assert_eq!(
            max_delta(&on, &back),
            0,
            "switching back on restores the frame"
        );
    }
}

#[test]
fn a_group_off_draws_what_a_pack_without_it_draws() {
    let room = grouped();
    let mut h = headless(Quality::high(), &room);
    h.renderer().set_group(0, 0, false);
    let off = h.render(&camera(&room), 0.0);
    let mut cut = demo_room();
    cut.draws.remove(draw_index(&cut, "box_red") as usize);
    cut.lights.remove(0);
    let expected = render(Quality::high(), &cut);
    assert_eq!(max_delta(&off, &expected), 0);
}

#[test]
fn a_piece_shows_only_the_draws_of_its_state() {
    let mut room = demo_room();
    let (glow, drawn) = (
        draw_index(&room, "box_emissive"),
        draw_index(&room, "box_drawn"),
    );
    room.pieces = vec![Piece {
        name: "sign".into(),
        states: vec![
            PieceState {
                name: String::new(),
                draws: vec![glow],
            },
            PieceState {
                name: "lit".into(),
                draws: vec![drawn],
            },
        ],
        state: 0,
    }];
    let mut h = headless(Quality::high(), &room);
    let start = h.render(&camera(&room), 0.0);
    assert!(h.renderer().set_piece_state(0, 0, 1));
    assert!(!h.renderer().set_piece_state(0, 0, 2), "no state 2");
    let lit = h.render(&camera(&room), 0.0);
    let mut loaded_lit = room.clone();
    loaded_lit.pieces[0].state = 1;
    assert_eq!(max_delta(&lit, &render(Quality::high(), &loaded_lit)), 0);
    assert!(max_delta(&start, &lit) > 40);
    h.renderer().set_piece_state(0, 0, 0);
    assert_eq!(max_delta(&start, &h.render(&camera(&room), 0.0)), 0);
}

#[test]
fn a_node_moves_and_hides_its_draws() {
    let mut room = demo_room();
    let blue = draw_index(&room, "box_blue");
    room.nodes = vec![Node {
        name: "blue".into(),
        pivot: Mat4::IDENTITY.to_cols_array(),
        draws: vec![blue],
        visible: true,
        hinge: None,
    }];
    let offset = Mat4::from_translation(Vec3::new(-1.0, 0.0, 2.0)) * Mat4::from_rotation_y(0.6);
    let mut moved = room.clone();
    let d = &mut moved.draws[blue as usize];
    d.transform = (offset * Mat4::from_cols_array(&d.transform)).to_cols_array();
    d.bounds = Aabb::around(&moved.meshes[d.mesh as usize], &d.transform);

    let mut h = headless(Quality::high(), &room);
    let rest = h.render(&camera(&room), 0.0);
    assert!(h.renderer().set_node_offset(0, 0, offset));
    let after = h.render(&camera(&room), 0.0);
    let expected = render(Quality::high(), &moved);
    let d = compare_images(&after, &expected).unwrap();
    assert!(d.max_channel_delta <= 2, "{d:?}");
    assert!(max_delta(&rest, &after) > 40);

    assert!(h.renderer().set_node_visible(0, 0, false));
    let hidden = h.render(&camera(&room), 0.0);
    let mut gone = room.clone();
    gone.draws[blue as usize].visible = false;
    assert_eq!(max_delta(&hidden, &render(Quality::high(), &gone)), 0);

    h.renderer().set_node_visible(0, 0, true);
    h.renderer().set_node_offset(0, 0, Mat4::IDENTITY);
    assert_eq!(max_delta(&rest, &h.render(&camera(&room), 0.0)), 0);
}

#[test]
fn a_room_uploaded_a_share_at_a_time_draws_like_one_uploaded_whole() {
    let room = demo_room();
    let whole = render(Quality::high(), &room);
    let mut h = Headless::new(W, H, Quality::high()).unwrap();
    let total = h.renderer().begin_room(0, &room).unwrap();
    let mut shares = 0;
    let mut last = 0;
    while !h.renderer().upload_room(0, &room, 256) {
        let (uploaded, all) = h.renderer().upload_progress(0).unwrap();
        assert_eq!(all, total);
        assert!(
            uploaded > last && uploaded < total,
            "{last} then {uploaded} of {total}"
        );
        last = uploaded;
        shares += 1;
    }
    assert!(shares > 4, "{shares} shares of {total} bytes");
    assert!(h.renderer().resident(0));
    h.renderer().set_drawn(&[0]);
    h.renderer().set_look(&room.look);
    let shared = h.render(&camera(&room), 0.0);
    assert_eq!(max_delta(&whole, &shared), 0);
}

#[test]
fn a_room_being_uploaded_leaves_the_drawn_one_untouched() {
    let room = demo_room();
    let mut h = headless(Quality::high(), &room);
    let before = h.render(&camera(&room), 0.0);
    h.renderer().begin_room(1, &neighbour()).unwrap();
    h.renderer().upload_room(1, &neighbour(), 1024);
    assert!(!h.renderer().resident(1));
    assert_eq!(max_delta(&before, &h.render(&camera(&room), 0.0)), 0);
}

#[test]
fn two_rooms_drawn_together_look_like_one_pack_of_both() {
    for quality in [Quality::high(), downlevel()] {
        let (a, b) = (demo_room(), neighbour());
        let mut h = headless(quality.clone(), &a);
        h.renderer().load_room_into(1, &b).unwrap();
        let alone = h.render(&camera(&a), 0.0);
        let (pipelines, buffers) = (
            h.renderer().pipelines_created(),
            h.renderer().buffers_created(),
        );
        h.renderer().set_drawn(&[0, 1]);
        let both = h.render(&camera(&a), 0.0);
        assert_eq!(h.renderer().pipelines_created(), pipelines);
        assert_eq!(h.renderer().buffers_created(), buffers);
        let expected = render(quality.clone(), &merge(&a, &b));
        let d = compare_images(&both, &expected).unwrap();
        assert!(d.max_channel_delta <= 2, "{d:?}");
        assert!(max_delta(&alone, &both) > 40);

        h.renderer().unload_room(1);
        assert!(!h.renderer().resident(1));
        assert_eq!(max_delta(&alone, &h.render(&camera(&a), 0.0)), 0);
    }
}

#[test]
fn a_room_fades_in_by_its_presence() {
    for quality in [Quality::high(), downlevel()] {
        let (mut a, mut b) = (demo_room(), neighbour());
        // Occlusion would darken the floor around the dithered pixels too.
        a.look.ao_strength = 0.0;
        b.lights.clear();
        let mut h = headless(quality.clone(), &a);
        let alone = h.render(&camera(&a), 0.0);
        h.renderer().load_room_into(1, &b).unwrap();
        h.renderer().set_drawn(&[0, 1]);
        let whole = h.render(&camera(&a), 0.0);
        h.renderer().set_room_presence(1, 0.0);
        assert_eq!(max_delta(&alone, &h.render(&camera(&a), 0.0)), 0, "absent");
        h.renderer().set_room_presence(1, 0.5);
        let half = h.render(&camera(&a), 0.0);
        let changed = |x: &RgbaImage| {
            (x.pixels().zip(alone.pixels()))
                .filter(|(p, q)| p.0.iter().zip(q.0).any(|(u, v)| u.abs_diff(v) > 8))
                .count() as f32
        };
        let ratio = changed(&half) / changed(&whole);
        assert!(
            (0.3..0.7).contains(&ratio),
            "half present changes {ratio} of the pixels"
        );
        h.renderer().set_room_presence(1, 1.0);
        assert_eq!(
            max_delta(&whole, &h.render(&camera(&a), 0.0)),
            0,
            "whole again"
        );
    }
}
