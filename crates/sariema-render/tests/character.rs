// SPDX-License-Identifier: Apache-2.0

//! The character light: an actor in a dim room of warm side light reads at
//! least as well as under a neutral light from the front, keeps its
//! albedo's hue as far as the room asks, and its rim draws no bright lines.

use glam::{Mat4, Quat, Vec3};
use image::RgbaImage;
use sariema_anim::{Rig, Transform};
use sariema_core::{Camera, room_camera};
use sariema_format::actor::demo::demo_arm;
use sariema_format::actor::{ActorPack, SkinnedMesh, SkinnedVertex};
use sariema_format::demo::demo_room;
use sariema_format::{Aabb, CharacterLight, Mobility, RoomPack, Sun, hemisphere_sh9};
use sariema_render::{
    ActorState, GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, compare_images,
};

mod common;
use common::{downlevel, matches_golden_on_the_downlevel_path};

const W: u32 = 960;
const H: u32 = 540;
const ORANGE: [f32; 3] = [0.9, 0.35, 0.08];

fn golden(name: &str) -> String {
    format!(
        "{}/../../tests/golden/{name}.png",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn camera() -> Camera {
    room_camera(&demo_room().camera, W as f32 / H as f32)
}

/// The demo arm all orange, without its material's own rim.
fn orange_arm() -> ActorPack {
    let mut pack = demo_arm();
    for m in &mut pack.materials {
        m.base_color = [ORANGE[0], ORANGE[1], ORANGE[2], 1.0];
        m.rim = 0.0;
    }
    pack
}

const HAIR: [f32; 3] = [0.35, 0.18, 0.08];

/// The demo arm's rig and material holding, instead of its column, a
/// 0.5 m ball of `rings` × `segments` flat facets, each drawn with its own
/// face's normal as stylised hair is: every facet turns edge-on somewhere
/// along the silhouette.
fn faceted_ball(rings: u32, segments: u32) -> ActorPack {
    let mut pack = orange_arm();
    let at = |ring: u32, segment: u32| {
        let (theta, phi) = (
            std::f32::consts::PI * ring as f32 / rings as f32,
            std::f32::consts::TAU * segment as f32 / segments as f32,
        );
        Vec3::new(
            theta.sin() * phi.cos(),
            theta.cos(),
            theta.sin() * phi.sin(),
        ) * 0.5
            + Vec3::new(0.0, 0.6, 0.0)
    };
    let mut ball = SkinnedMesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    let mut facet = |corners: [Vec3; 3]| {
        let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
        if normal.length_squared() < 1e-12 {
            return;
        }
        let normal = normal.normalize().to_array();
        for corner in corners {
            ball.indices.push(ball.vertices.len() as u32);
            ball.vertices.push(SkinnedVertex {
                pos: corner.to_array(),
                normal,
                uv: [0.0; 2],
                color: [255; 4],
                joints: [0; 4],
                weights: [u16::MAX, 0, 0, 0],
            });
        }
    };
    for ring in 0..rings {
        for segment in 0..segments {
            let (a, b) = (at(ring, segment), at(ring, segment + 1));
            let (c, d) = (at(ring + 1, segment), at(ring + 1, segment + 1));
            facet([a, d, c]);
            facet([a, b, d]);
        }
    }
    pack.meshes = vec![ball];
    pack.parts.truncate(1);
    pack.materials.truncate(1);
    pack.materials[0].base_color = [HAIR[0], HAIR[1], HAIR[2], 1.0];
    pack.bounds = Aabb {
        min: [-0.55, 0.05, -0.55],
        max: [0.55, 1.15, 0.55],
    };
    pack
}

/// The demo room's floor and boxes lit by a sun alone, from `towards`.
fn lit_by(towards: Vec3, color: [f32; 3], intensity: f32, sky: [f32; 3]) -> RoomPack {
    let mut room = demo_room();
    room.lights.clear();
    room.sun = Some(Sun {
        direction: towards.normalize().into(),
        color,
        intensity,
        casts_shadow: false,
        mobility: Mobility::Static,
    });
    room.ambient.sh9 = hemisphere_sh9(sky, [0.2 * sky[0], 0.2 * sky[1], 0.2 * sky[2]], 0.3);
    room.look.ao_strength = 0.0;
    room
}

/// A neutral light from the camera's side.
fn front_lit() -> RoomPack {
    let towards = camera().position - Vec3::from(demo_room().camera.focus);
    lit_by(towards, [1.0; 3], 1.0, [1.0; 3])
}

/// A light of `color` low from the actor's side, dimmer than the front one.
fn side_lit_by(color: [f32; 3], character: CharacterLight) -> RoomPack {
    RoomPack {
        character,
        ..lit_by(
            Vec3::new(1.0, 0.25, 0.1),
            color,
            0.8,
            color.map(|c| 0.8 * c),
        )
    }
}

/// A warm side light.
fn side_lit(character: CharacterLight) -> RoomPack {
    side_lit_by([1.0, 0.55, 0.25], character)
}

fn legible() -> CharacterLight {
    CharacterLight {
        direction: [0.3, 0.6, 1.0],
        color: [1.0, 0.97, 0.92],
        intensity: 1.0,
        wrap: 0.5,
        rim: 0.35,
        rim_power: 3.0,
        min_brightness: 0.3,
        room_tint: 0.2,
    }
}

/// The arm bent halfway, turned to show its side.
fn palette(pack: &ActorPack) -> Vec<Mat4> {
    let rig = Rig::new(pack).unwrap();
    let mut local = vec![Transform::IDENTITY; rig.joints()];
    rig.sample(rig.clip("bend").unwrap(), 0.5, &mut local);
    let mut model = vec![Mat4::IDENTITY; rig.joints()];
    rig.model_space(&local, &mut model);
    (model.iter().zip(&pack.joints))
        .map(|(m, j)| *m * Mat4::from_cols_array(&j.inverse_bind))
        .collect()
}

/// The frame with the arm and the same frame without it.
fn frames(room: &RoomPack, pack: &ActorPack, quality: Quality) -> (RgbaImage, RgbaImage) {
    let mut h = Headless::new(W, H, quality).unwrap();
    let renderer = h.renderer();
    renderer.load_room(room).unwrap();
    let model = renderer.load_actor(pack).unwrap();
    let arm = renderer.spawn_actor(model).unwrap();
    // The disc darkens the floor, which is not the actor.
    renderer.set_actor_disc(arm, None).unwrap();
    let palette = palette(pack);
    let mut state = ActorState {
        model: Mat4::from_scale_rotation_translation(
            Vec3::splat(1.5),
            Quat::from_rotation_y(0.6),
            Vec3::new(0.6, 0.0, 1.2),
        ),
        palette: &palette,
        faces: &[],
        visible: true,
    };
    renderer.set_actor(arm, &state).unwrap();
    let with = h.render(&camera(), 0.0);
    state.visible = false;
    h.renderer().set_actor(arm, &state).unwrap();
    let without = h.render(&camera(), 0.0);
    (with, without)
}

/// The actor's pixels: those it changes, away from its silhouette, whose
/// blend with the room says nothing of the actor's own light.
fn actor_pixels(with: &RgbaImage, without: &RgbaImage) -> Vec<[f32; 3]> {
    let changed = |x: u32, y: u32| {
        let (a, b) = (with.get_pixel(x, y), without.get_pixel(x, y));
        (0..3).any(|c| a[c].abs_diff(b[c]) > 2)
    };
    let mut pixels = Vec::new();
    for y in 1..H - 1 {
        for x in 1..W - 1 {
            let inside = (-1i32..=1)
                .flat_map(|dy| (-1i32..=1).map(move |dx| (dx, dy)))
                .all(|(dx, dy)| changed((x as i32 + dx) as u32, (y as i32 + dy) as u32));
            if inside {
                let p = with.get_pixel(x, y);
                pixels.push([0, 1, 2].map(|c| p[c] as f32 / 255.0));
            }
        }
    }
    assert!(
        pixels.len() > 2000,
        "the arm covers {} pixels",
        pixels.len()
    );
    pixels
}

fn luma(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

fn mean(pixels: &[[f32; 3]]) -> [f32; 3] {
    let n = pixels.len() as f32;
    [0, 1, 2].map(|c| pixels.iter().map(|p| p[c]).sum::<f32>() / n)
}

/// Degrees round the colour wheel.
fn hue(p: [f32; 3]) -> f32 {
    let [r, g, b] = p;
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = (max - min).max(1e-6);
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    60.0 * h
}

fn arm(room: &RoomPack) -> Vec<[f32; 3]> {
    let (with, without) = frames(room, &orange_arm(), Quality::high());
    actor_pixels(&with, &without)
}

#[test]
fn a_side_lit_actor_reads_at_least_as_bright_as_one_lit_from_the_front() {
    let front = luma(mean(&arm(&front_lit())));
    let unaided = luma(mean(&arm(&side_lit(CharacterLight::default()))));
    let aided = luma(mean(&arm(&side_lit(legible()))));
    assert!(
        unaided < front,
        "the side light alone: {unaided} against {front}"
    );
    assert!(
        aided >= front,
        "with the character light: {aided} against {front}"
    );
}

#[test]
fn the_room_tints_the_actor_only_as_far_as_it_is_asked() {
    // A green light, which turns orange towards yellow, against a grey one
    // as bright: the tone curve shifts hue with brightness, so the two must
    // match in luminance for the hue to tell the light's colour alone.
    let green = [0.3, 1.0, 0.45];
    let grey = [0.2126 * green[0] + 0.7152 * green[1] + 0.0722 * green[2]; 3];
    let hue_under = |color: [f32; 3], room_tint: f32| {
        let character = CharacterLight {
            room_tint,
            color: [1.0; 3],
            ..legible()
        };
        hue(mean(&arm(&side_lit_by(color, character))))
    };
    let neutral = hue_under(grey, 1.0);
    let kept = (hue_under(green, 0.0) - neutral).abs();
    let half = (hue_under(green, 0.5) - neutral).abs();
    let full = (hue_under(green, 1.0) - neutral).abs();
    assert!(kept < 0.5, "{kept}° off the albedo's hue with room_tint 0");
    assert!(
        half > kept + 1.0 && full > half + 1.0,
        "{kept}°, {half}° and {full}° with room_tint 0, 0.5 and 1"
    );
}

#[test]
fn the_rim_draws_no_bright_lines() {
    // A pixel much paler than the albedo is light that is not the albedo's:
    // an edge lit white.
    let pale = |p: &[f32; 3]| p[2] > 0.6 * p[0];
    let lines = |pack: &ActorPack| {
        let (with, without) = frames(&side_lit(legible()), pack, Quality::high());
        let pixels = actor_pixels(&with, &without);
        pixels.iter().filter(|p| pale(p)).count() as f32 / pixels.len() as f32
    };
    let mut white_rim = orange_arm();
    white_rim.materials[0].rim = 0.6;
    assert!(lines(&white_rim) > 0.005, "the measure sees a white rim");
    let rimmed = lines(&orange_arm());
    assert!(rimmed < 0.0005, "{rimmed} of the arm is pale");
}

/// Inside the actor, how much of it is paler than its albedo, and the 99th
/// percentile of the change in brightness between neighbouring pixels: on
/// flat facets, the step from one facet to the next.
fn facets(with: &RgbaImage, without: &RgbaImage) -> (f32, f32) {
    let changed = |x: u32, y: u32| {
        let (a, b) = (with.get_pixel(x, y), without.get_pixel(x, y));
        (0..3).any(|c| a[c].abs_diff(b[c]) > 2)
    };
    let inside = |x: u32, y: u32| {
        (-1i32..=1)
            .flat_map(|dy| (-1i32..=1).map(move |dx| (dx, dy)))
            .all(|(dx, dy)| changed((x as i32 + dx) as u32, (y as i32 + dy) as u32))
    };
    let at = |x: u32, y: u32| {
        let p = with.get_pixel(x, y);
        [0, 1, 2].map(|c| p[c] as f32 / 255.0)
    };
    let (mut pixels, mut pale, mut steps) = (0usize, 0usize, Vec::new());
    for y in 1..H - 2 {
        for x in 1..W - 2 {
            if !inside(x, y) {
                continue;
            }
            pixels += 1;
            let p = at(x, y);
            if p[2] > 0.6 * p[0] {
                pale += 1;
            }
            for (nx, ny) in [(x + 1, y), (x, y + 1)] {
                if inside(nx, ny) {
                    steps.push((luma(at(nx, ny)) - luma(p)).abs());
                }
            }
        }
    }
    assert!(pixels > 2000, "the ball covers {pixels} pixels");
    steps.sort_by(f32::total_cmp);
    (pale as f32 / pixels as f32, steps[steps.len() * 99 / 100])
}

/// On a ball of flat facets the white material rim lines every facet seen
/// edge-on; the character light's rim may brighten a facet in its own
/// colour, but keeps the step from one facet to the next (the 99th
/// percentile of the change between neighbouring pixels) within half again
/// of what the facets show without a rim.
#[test]
fn on_flat_facets_the_rim_draws_no_bright_lines_and_keeps_the_facets_steps() {
    let measure = |pack: &ActorPack, character: CharacterLight| {
        let (with, without) = frames(&side_lit(character), pack, Quality::high());
        facets(&with, &without)
    };
    let ball = faceted_ball(6, 9);
    let unrimmed = CharacterLight {
        rim: 0.0,
        ..legible()
    };
    let (_, plain) = measure(&ball, unrimmed.clone());
    let (pale, tinted) = measure(&ball, legible());
    let mut white = ball.clone();
    white.materials[0].rim = 0.6;
    let (white_pale, white_step) = measure(&white, unrimmed);
    assert!(
        white_pale > 0.005 && white_step > 3.0 * plain,
        "the measure sees a white rim: {white_pale} pale, steps {white_step} against {plain}"
    );
    assert!(pale < 0.0005, "{pale} of the ball is pale");
    assert!(
        tinted < 1.5 * plain,
        "the rim steps {tinted} between facets against {plain} without it"
    );
}

#[test]
fn the_side_lit_actor_matches_its_golden() {
    let (with, _) = frames(&side_lit(legible()), &orange_arm(), Quality::high());
    let name = "demo_actor_character_light";
    let actual = format!("{}/{name}.actual.png", env!("CARGO_TARGET_TMPDIR"));
    with.save(&actual).unwrap();
    let expected = image::open(golden(name)).unwrap().to_rgba8();
    let d = compare_images(&with, &expected).unwrap();
    assert!(
        d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO,
        "{d:?}; this render is in {actual}"
    );
    let (with, _) = frames(&side_lit(legible()), &orange_arm(), downlevel());
    matches_golden_on_the_downlevel_path(&with, name, &golden(name));
}
