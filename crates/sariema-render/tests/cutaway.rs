// SPDX-License-Identifier: Apache-2.0

//! Cutaways: a wall between the camera and what stands behind it is cut
//! above its height on the camera's side of the line, shows its inside in
//! its cap, fades in over its softness, casts the shadow it is drawn with,
//! and changes nothing where it cuts nothing; on both render paths.

mod common;

use common::downlevel;
use image::RgbaImage;
use sariema_core::{Camera, room_camera};
use sariema_format::demo::demo_room;
use sariema_format::{Aabb, CutLine, CutStyle, Cutaway, Draw, Material, RoomPack};
use sariema_render::{CutState, Headless, Quality};

const W: u32 = 320;
const H: u32 = 180;

fn paths() -> [Quality; 2] {
    [Quality::high(), downlevel()]
}

fn camera_of(room: &RoomPack) -> Camera {
    room_camera(&room.camera, W as f32 / H as f32)
}

/// The demo room with a grey wall 3 m tall across the front of the red box,
/// cut by a cutaway with `cap` and `soft`, casting its shadow cut when
/// `cut_shadows`.
fn walled(cap: Option<[f32; 3]>, soft: f32, cut_shadows: bool) -> RoomPack {
    let mut room = demo_room();
    let grey = Material {
        name: "wall".into(),
        base_color: [0.5, 0.5, 0.5, 1.0],
        ..room.materials[1].clone()
    };
    room.materials.push(grey);
    let mesh = room.draws[1].mesh;
    #[rustfmt::skip]
    let transform = [
        5.0, 0.0, 0.0, 0.0,
        0.0, 3.0, 0.0, 0.0,
        0.0, 0.0, 0.2, 0.0,
        -1.25, 0.0, 0.3, 1.0,
    ];
    room.draws.push(Draw {
        name: Some("wall".into()),
        mesh,
        material: room.materials.len() as u32 - 1,
        transform,
        visible: true,
        casts_shadow: true,
        bounds: Aabb::around(&room.meshes[mesh as usize], &transform),
    });
    room.cutaways.push(Cutaway {
        name: "walls".into(),
        draws: vec![room.draws.len() as u32 - 1],
        height: 0.3,
        line: CutLine::default(),
        radius: 0.0,
        soft,
        cap,
        cut_shadows,
        anchors: vec![None],
        style: CutStyle::Cut,
    });
    room
}

/// `walled` with a magenta lintel 2 m up, across the wall's thickness and
/// 0.2 m past it on either side, in the same cutaway; the wall and the
/// lintel judged whole at `anchor` when given.
fn lintelled(anchor: Option<[f32; 2]>, style: CutStyle, soft: f32) -> RoomPack {
    let mut room = walled(None, soft, true);
    let magenta = Material {
        name: "lintel".into(),
        base_color: [1.0, 0.0, 1.0, 1.0],
        ..room.materials[1].clone()
    };
    room.materials.push(magenta);
    let mesh = room.draws[1].mesh;
    #[rustfmt::skip]
    let transform = [
        2.0, 0.0, 0.0, 0.0,
        0.0, 0.3, 0.0, 0.0,
        0.0, 0.0, 0.6, 0.0,
        -1.25, 2.0, 0.3, 1.0,
    ];
    room.draws.push(Draw {
        name: Some("lintel".into()),
        mesh,
        material: room.materials.len() as u32 - 1,
        transform,
        visible: true,
        casts_shadow: true,
        bounds: Aabb::around(&room.meshes[mesh as usize], &transform),
    });
    let cutaway = &mut room.cutaways[0];
    cutaway.draws.push(room.draws.len() as u32 - 1);
    cutaway.anchors = vec![anchor; 2];
    cutaway.style = style;
    room
}

/// Cuts along +Z, the camera's side, beyond `line`, above `height`.
fn cut(line: f32, height: f32) -> CutState {
    CutState {
        toward: [0.0, 1.0],
        line,
        height,
        hero: [0.0, 0.0],
    }
}

fn render(room: &RoomPack, state: Option<CutState>, quality: &Quality) -> RgbaImage {
    let mut h = Headless::new(W, H, quality.clone()).unwrap();
    h.renderer().load_room(room).unwrap();
    if let Some(state) = state {
        assert!(h.renderer().set_cutaway(0, 0, state));
    }
    h.render(&camera_of(room), 0.0)
}

/// Pixels of the red box.
fn red(image: &RgbaImage) -> usize {
    let p = |p: &image::Rgba<u8>| {
        let [r, g, b] = [p[0], p[1], p[2]].map(u32::from);
        r > 60 && r * 10 > g * 18 && r > 2 * b
    };
    image.pixels().filter(|q| p(q)).count()
}

fn magenta(image: &RgbaImage) -> usize {
    let p =
        |p: &image::Rgba<u8>| p[0] > 40 && p[2] > 40 && (p[1] as u32) * 3 < (p[0].min(p[2]) as u32);
    image.pixels().filter(|q| p(q)).count()
}

#[test]
fn a_wall_on_the_cameras_side_is_cut_above_its_height() {
    for q in &paths() {
        let open = red(&render(&demo_room(), None, q));
        let room = walled(None, 0.0, true);
        let walled = red(&render(&room, None, q));
        let cut = red(&render(&room, Some(cut(-100.0, 0.3)), q));
        assert!(
            walled * 4 < open,
            "{:?}: the wall hides the box: {walled} of {open}",
            q.path
        );
        assert!(
            cut * 10 > open * 8,
            "{:?}: the cut shows it: {cut} of {open}",
            q.path
        );
    }
}

#[test]
fn the_inside_of_a_cut_shows_its_cap() {
    for q in &paths() {
        let capped = walled(Some([1.0, 0.0, 1.0]), 0.0, true);
        let shown = magenta(&render(&capped, Some(cut(-100.0, 0.3)), q));
        let hollow = magenta(&render(&walled(None, 0.0, true), Some(cut(-100.0, 0.3)), q));
        assert!(shown > 100, "{:?}: the cap shows: {shown}", q.path);
        assert_eq!(hollow, 0, "{:?}", q.path);
    }
}

#[test]
fn a_cutaway_that_cuts_nothing_changes_no_pixel() {
    for q in &paths() {
        let mut room = walled(Some([1.0, 0.0, 1.0]), 0.5, true);
        let whole = {
            let mut bare = room.clone();
            bare.cutaways.clear();
            render(&bare, None, q)
        };
        assert_eq!(render(&room, None, q), whole, "{:?}: never placed", q.path);
        let behind = render(&room, Some(cut(100.0, 0.3)), q);
        assert_eq!(behind, whole, "{:?}: the line beyond the wall", q.path);
        room.cutaways[0].radius = 0.5;
        let away = CutState {
            hero: [20.0, 20.0],
            ..cut(-100.0, 0.3)
        };
        assert_eq!(
            render(&room, Some(away), q),
            whole,
            "{:?}: far from the hero",
            q.path
        );
    }
}

#[test]
fn the_cut_fades_in_over_its_softness() {
    for q in &paths() {
        let hard = red(&render(&walled(None, 0.0, true), Some(cut(-0.6, 0.3)), q));
        let whole = red(&render(&walled(None, 0.0, true), None, q));
        // The wall's face is 1 m past the line: half of a 2 m fade, so it is
        // cut a softness higher.
        let soft = red(&render(&walled(None, 2.0, true), Some(cut(-0.6, 0.3)), q));
        assert!(
            whole < soft && soft < hard,
            "{:?}: {whole} < {soft} < {hard}",
            q.path
        );
    }
}

fn mean(image: &RgbaImage) -> f64 {
    let sum: u64 = image
        .pixels()
        .map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64)
        .sum();
    sum as f64 / (3 * image.pixels().len()) as f64
}

#[test]
fn a_cut_wall_casts_the_shadow_it_is_drawn_with_unless_told_otherwise() {
    for q in &paths() {
        let state = Some(cut(-100.0, 0.3));
        let cut_shadow = mean(&render(&walled(None, 0.0, true), state, q));
        let whole_shadow = mean(&render(&walled(None, 0.0, false), state, q));
        assert!(
            cut_shadow > whole_shadow + 0.5,
            "{:?}: the spot reaches the floor behind the cut wall: {cut_shadow} against {whole_shadow}",
            q.path
        );
    }
}

#[test]
fn a_radius_cuts_only_around_the_hero() {
    for q in &paths() {
        let open = red(&render(&demo_room(), None, q));
        let mut room = walled(None, 0.0, true);
        room.cutaways[0].radius = 4.0;
        let near = red(&render(&room, Some(cut(-100.0, 0.3)), q));
        let away = CutState {
            hero: [0.0, 30.0],
            ..cut(-100.0, 0.3)
        };
        let far = red(&render(&room, Some(away), q));
        assert!(
            near * 10 > open * 8,
            "{:?}: near the hero: {near} of {open}",
            q.path
        );
        assert!(far * 4 < open, "{:?}: far from it: {far} of {open}", q.path);
    }
}

#[test]
fn a_piece_on_a_cut_wall_is_cut_whole_and_nothing_is_left_floating() {
    for q in &paths() {
        // The line runs through the middle of the wall and the lintel.
        let state = Some(cut(0.3, 0.3));
        let whole = magenta(&render(&lintelled(None, CutStyle::Cut, 0.0), None, q));
        let by_part = magenta(&render(&lintelled(None, CutStyle::Cut, 0.0), state, q));
        let anchored = lintelled(Some([-1.25, 0.4]), CutStyle::Cut, 0.0);
        let by_wall = magenta(&render(&anchored, state, q));
        assert!(whole > 200, "{:?}: the lintel shows: {whole}", q.path);
        assert!(
            by_part > 20,
            "{:?}: cut part by part, its back half floats: {by_part}",
            q.path
        );
        assert_eq!(
            by_wall, 0,
            "{:?}: judged by its wall, it goes with it",
            q.path
        );
        let behind = lintelled(Some([-1.25, 0.2]), CutStyle::Cut, 0.0);
        let kept = magenta(&render(&behind, state, q));
        assert_eq!(
            kept, whole,
            "{:?}: a wall behind the line keeps it whole",
            q.path
        );
    }
}

#[test]
fn a_fading_cutaway_dissolves_what_it_reaches_whole() {
    for q in &paths() {
        let open = red(&render(&demo_room(), None, q));
        let at = Some([-1.25, 0.4]);
        let state = |line| Some(cut(line, 0.3));
        let gone = red(&render(&lintelled(at, CutStyle::Fade, 0.0), state(0.3), q));
        let lintel = magenta(&render(&lintelled(at, CutStyle::Fade, 0.0), state(0.3), q));
        // Half the softness past the line: about half its pixels dissolve.
        let half = red(&render(&lintelled(at, CutStyle::Fade, 1.0), state(-0.1), q));
        assert!(
            gone * 10 > open * 8,
            "{:?}: the box shows: {gone} of {open}",
            q.path
        );
        assert_eq!(lintel, 0, "{:?}", q.path);
        assert!(
            half * 5 > open && half * 5 < open * 4,
            "{:?}: half: {half} of {open}",
            q.path
        );
    }
}
