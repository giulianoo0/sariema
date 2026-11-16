// SPDX-License-Identifier: Apache-2.0

//! The demo arm and face actors in the demo room.

use glam::{Mat4, Vec3};
use image::RgbaImage;
use sariema_anim::{Rig, Transform};
use sariema_core::{Camera, room_camera};
use sariema_format::actor::ActorPack;
use sariema_format::actor::demo::{demo_arm, demo_face};
use sariema_format::demo::demo_room;
use sariema_format::{LightKind, Mobility, RoomPack, Sun};
use sariema_render::{
    ActorId, ActorState, GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, RenderError,
    compare_images,
};

mod common;
use common::{downlevel, matches_golden_on_the_downlevel_path};

const W: u32 = 960;
const H: u32 = 540;

fn golden(name: &str) -> String {
    format!(
        "{}/../../tests/golden/{name}.png",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn camera() -> Camera {
    room_camera(&demo_room().camera, W as f32 / H as f32)
}

/// `clip` of `pack` at `time`, as palette matrices.
fn palette(pack: &ActorPack, clip: &str, time: f32) -> Vec<Mat4> {
    let rig = Rig::new(pack).unwrap();
    let mut local = vec![Transform::IDENTITY; rig.joints()];
    rig.sample(rig.clip(clip).unwrap(), time, &mut local);
    let mut model = vec![Mat4::IDENTITY; rig.joints()];
    rig.model_space(&local, &mut model);
    (model.iter().zip(&pack.joints))
        .map(|(m, j)| *m * Mat4::from_cols_array(&j.inverse_bind))
        .collect()
}

/// Where the arm stands: inside the spot's cone, in front of the boxes.
fn arm_place() -> Mat4 {
    Mat4::from_scale_rotation_translation(
        Vec3::splat(1.5),
        glam::Quat::from_rotation_y(0.6),
        Vec3::new(0.6, 0.0, 1.2),
    )
}

/// The demo room with the arm posed at `time` into `bend`.
fn arm_scene(quality: Quality, time: f32) -> (Headless, ActorId) {
    arm_in(&demo_room(), quality, arm_place(), time)
}

/// `room` with the arm placed by `place` and posed at `time` into `bend`.
fn arm_in(room: &RoomPack, quality: Quality, place: Mat4, time: f32) -> (Headless, ActorId) {
    let mut h = Headless::new(W, H, quality).unwrap();
    let pack = demo_arm();
    let renderer = h.renderer();
    renderer.load_room(room).unwrap();
    let model = renderer.load_actor(&pack).unwrap();
    let arm = renderer.spawn_actor(model).unwrap();
    let palette = palette(&pack, "bend", time);
    let state = ActorState {
        model: place,
        palette: &palette,
        faces: &[],
        visible: true,
    };
    renderer.set_actor(arm, &state).unwrap();
    (h, arm)
}

/// Whether the arm's disc changes the frame, that is, whether it is drawn.
fn disc_drawn(room: &RoomPack, quality: Quality, place: Mat4) -> bool {
    let (mut h, arm) = arm_in(room, quality, place, 0.0);
    let with = h.render(&camera(), 0.0);
    h.renderer().set_actor_disc(arm, None).unwrap();
    let without = h.render(&camera(), 0.0);
    compare_images(&with, &without).unwrap().max_channel_delta > 0
}

fn matches_golden(image: &RgbaImage, name: &str) {
    let path = golden(name);
    let expected = image::open(&path).unwrap().to_rgba8();
    let d = compare_images(image, &expected).unwrap();
    if !(d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO) {
        let actual = format!("{}/{name}.actual.png", env!("CARGO_TARGET_TMPDIR"));
        image.save(&actual).unwrap();
        panic!("{d:?}; this render is in {actual}");
    }
}

#[test]
fn the_arm_at_rest_matches_its_golden() {
    let (mut h, _) = arm_scene(Quality::high(), 0.0);
    matches_golden(&h.render(&camera(), 0.0), "demo_actor_rest");
}

#[test]
fn the_bent_arm_matches_its_golden() {
    let (mut h, _) = arm_scene(Quality::high(), 0.5);
    matches_golden(&h.render(&camera(), 0.0), "demo_actor_bent");
}

#[test]
fn the_arm_matches_its_goldens_on_the_downlevel_path() {
    for (time, name) in [(0.0, "demo_actor_rest"), (0.5, "demo_actor_bent")] {
        let (mut h, _) = arm_scene(downlevel(), time);
        matches_golden_on_the_downlevel_path(&h.render(&camera(), 0.0), name, &golden(name));
    }
}

#[test]
fn a_record_past_the_downlevel_uniform_is_refused() {
    let pack = demo_arm();
    let rows = 8 + pack.faces.len().div_ceil(2) as u32 + 4 * pack.joints.len() as u32;
    let tight = Quality {
        actor_uniform_rows: rows - 1,
        ..downlevel()
    };
    let mut h = Headless::new(W, H, tight).unwrap();
    match h.renderer().load_actor(&pack) {
        Err(RenderError::Actor(why)) => assert!(why.contains("actor_uniform_rows"), "{why}"),
        other => panic!("expected an actor error, got {other:?}"),
    }
    let fits = Quality {
        actor_uniform_rows: rows,
        ..downlevel()
    };
    let mut h = Headless::new(W, H, fits).unwrap();
    h.renderer().load_actor(&pack).unwrap();
}

#[test]
#[ignore = "rewrites tests/golden/demo_actor_rest.png and demo_actor_bent.png"]
fn write_actor_goldens() {
    for (time, name) in [(0.0, "demo_actor_rest"), (0.5, "demo_actor_bent")] {
        let (mut h, _) = arm_scene(Quality::high(), time);
        h.render(&camera(), 0.0).save(golden(name)).unwrap();
    }
}

#[test]
fn an_actors_draws_are_counted() {
    let room = demo_room();
    let (mut h, _) = arm_scene(Quality::high(), 0.0);
    h.render(&camera(), 0.0);
    h.render(&camera(), 0.0);
    let stats = h.renderer().last_frame_stats();
    assert_eq!(stats.actors_drawn, 1, "{stats:?}");
    let visible = room.draws.iter().filter(|d| d.visible).count() as u32;
    let room_draws = 1 + visible - stats.draws_culled + stats.post_passes;
    let parts = demo_arm().parts.iter().filter(|p| p.visible).count() as u32;
    // Every visible part, and at most a disc under it.
    let actor_draws = stats.draws - room_draws;
    assert!((parts..=parts + 1).contains(&actor_draws), "{stats:?}");
}

#[test]
fn a_pose_moves_pixels_and_the_actor_is_drawn() {
    let (mut rest, _) = arm_scene(Quality::high(), 0.0);
    let (mut bent, _) = arm_scene(Quality::high(), 0.5);
    let (a, b) = (rest.render(&camera(), 0.0), bent.render(&camera(), 0.0));
    let stats = rest.renderer().last_frame_stats();
    assert_eq!((stats.actors_drawn, stats.actors_culled), (1, 0));
    assert!(compare_images(&a, &b).unwrap().differing_pixels_ratio > 0.001);
}

#[test]
fn drawing_actors_creates_no_pipeline_and_no_buffer() {
    let (mut h, arm) = arm_scene(Quality::high(), 0.0);
    h.render(&camera(), 0.0);
    let pack = demo_arm();
    let (pipelines, buffers) = (
        h.renderer().pipelines_created(),
        h.renderer().buffers_created(),
    );
    for frame in 0..5 {
        let palette = palette(&pack, "bend", frame as f32 / 10.0);
        let state = ActorState {
            model: arm_place(),
            palette: &palette,
            faces: &[],
            visible: true,
        };
        h.renderer().set_actor(arm, &state).unwrap();
        h.render(&camera(), frame as f32 / 60.0);
    }
    assert_eq!(h.renderer().pipelines_created(), pipelines);
    assert_eq!(h.renderer().buffers_created(), buffers);
}

#[test]
fn an_actor_out_of_view_is_culled_and_changes_no_pixel() {
    let frame = |frustum_culling| {
        let quality = Quality {
            frustum_culling,
            ..Quality::high()
        };
        let (mut h, _) = arm_scene(quality, 0.5);
        let pack = demo_arm();
        let renderer = h.renderer();
        let model = renderer.load_actor(&pack).unwrap();
        let aside = renderer.spawn_actor(model).unwrap();
        let palette = palette(&pack, "bend", 0.0);
        let state = ActorState {
            model: Mat4::from_translation(Vec3::new(300.0, 0.0, 0.0)),
            palette: &palette,
            faces: &[],
            visible: true,
        };
        renderer.set_actor(aside, &state).unwrap();
        renderer.set_actor_disc(aside, None).unwrap();
        let image = h.render(&camera(), 0.0);
        (image, h.renderer().last_frame_stats().actors_culled)
    };
    let ((culled, left_out), (all, none_left_out)) = (frame(true), frame(false));
    assert_eq!((left_out, none_left_out), (1, 0));
    assert_eq!(compare_images(&culled, &all).unwrap().max_channel_delta, 0);
}

#[test]
fn without_actor_shadow_slots_a_disc_darkens_the_floor() {
    let low = Quality {
        actor_shadow_slots: 0,
        ..Quality::high()
    };
    let (mut with_disc, arm) = arm_scene(low.clone(), 0.0);
    let a = with_disc.render(&camera(), 0.0);
    with_disc.renderer().set_actor_disc(arm, None).unwrap();
    let b = with_disc.render(&camera(), 0.0);
    let darker = (a.pixels().zip(b.pixels()))
        .filter(|(p, q)| p[0] < q[0])
        .count();
    let lighter = (a.pixels().zip(b.pixels()))
        .filter(|(p, q)| p[0] > q[0])
        .count();
    assert!(
        darker > 100 && lighter == 0,
        "{darker} darker, {lighter} lighter"
    );
}

#[test]
fn an_actor_in_a_shadow_slot_casts_into_it_and_gets_no_disc() {
    let (mut mapped, arm) = arm_scene(Quality::high(), 0.0);
    let with = mapped.render(&camera(), 0.0);
    mapped.renderer().set_actor_disc(arm, None).unwrap();
    let without_disc = mapped.render(&camera(), 0.0);
    assert_eq!(
        compare_images(&with, &without_disc)
            .unwrap()
            .max_channel_delta,
        0
    );
    let unshadowed = Quality {
        actor_shadow_slots: 0,
        ..Quality::high()
    };
    let (mut plain, arm) = arm_scene(unshadowed, 0.0);
    plain.renderer().set_actor_disc(arm, None).unwrap();
    let no_shadow = plain.render(&camera(), 0.0);
    let d = compare_images(&with, &no_shadow).unwrap();
    assert!(d.differing_pixels_ratio > 0.001, "{d:?}");
}

#[test]
fn an_actor_outside_the_suns_map_gets_a_disc() {
    let mut room = demo_room();
    room.lights[0].casts_shadow = false;
    room.sun = Some(Sun {
        direction: [0.2, 1.0, 0.1],
        color: [1.0, 0.95, 0.9],
        intensity: 2.0,
        casts_shadow: true,
        mobility: Mobility::Static,
    });
    let medium = Quality {
        actor_shadow_slots: 1,
        ..Quality::high()
    };
    // The sun's map is fitted to the boxes; 8 m along x the arm's bounds
    // leave it even with its widest clip.
    let beside =
        Mat4::from_translation(Vec3::new(8.0, 0.0, 0.0)) * Mat4::from_scale(Vec3::splat(1.5));
    assert!(!disc_drawn(&room, medium.clone(), arm_place()));
    assert!(disc_drawn(&room, medium, beside));
}

#[test]
fn an_actor_within_a_spots_range_but_outside_its_cone_gets_a_disc() {
    let mut room = demo_room();
    room.lights[0].kind = LightKind::Spot {
        target: [-2.5, 0.0, -1.0],
        cone_deg: 10.0,
    };
    assert!(disc_drawn(&room, Quality::high(), arm_place()));
}

#[test]
fn shadow_slots_are_redrawn_only_when_an_actor_moves() {
    let (mut h, arm) = arm_scene(Quality::high(), 0.0);
    h.render(&camera(), 0.0);
    let settled = h.renderer().shadow_passes_rendered();
    h.render(&camera(), 0.0);
    assert_eq!(h.renderer().shadow_passes_rendered(), settled);
    let pack = demo_arm();
    let palette = palette(&pack, "bend", 0.3);
    let state = ActorState {
        model: arm_place(),
        palette: &palette,
        faces: &[],
        visible: true,
    };
    h.renderer().set_actor(arm, &state).unwrap();
    h.render(&camera(), 0.0);
    assert_eq!(h.renderer().shadow_passes_rendered(), settled + 1);
}

#[test]
fn hiding_a_part_or_the_actor_changes_the_image() {
    let (mut h, arm) = arm_scene(Quality::high(), 0.0);
    let whole = h.render(&camera(), 0.0);
    assert!(h.renderer().set_actor_part_visible(arm, "hand", false));
    assert!(
        !h.renderer()
            .set_actor_part_visible(arm, "no_such_part", false)
    );
    let handless = h.render(&camera(), 0.0);
    assert!(compare_images(&whole, &handless).unwrap().max_channel_delta > 0);
}

#[test]
fn a_face_cell_picks_the_atlas_cell_shown() {
    let pack = demo_face();
    let mut h = Headless::new(W, H, Quality::high()).unwrap();
    let renderer = h.renderer();
    renderer.load_room(&demo_room()).unwrap();
    let model = renderer.load_actor(&pack).unwrap();
    let face = renderer.spawn_actor(model).unwrap();
    let palette = palette(&pack, "nod", 0.0);
    let mut frame = |cell: u32| {
        let state = ActorState {
            model: Mat4::from_scale_rotation_translation(
                Vec3::splat(3.0),
                glam::Quat::from_rotation_y(0.5),
                Vec3::new(0.5, 0.0, 1.5),
            ),
            palette: &palette,
            faces: &[cell],
            visible: true,
        };
        h.renderer().set_actor(face, &state).unwrap();
        h.render(&camera(), 0.0)
    };
    let (open, happy, open_again) = (frame(0), frame(2), frame(0));
    assert!(compare_images(&open, &happy).unwrap().max_channel_delta > 50);
    assert_eq!(
        compare_images(&open, &open_again)
            .unwrap()
            .max_channel_delta,
        0
    );
}

#[test]
fn an_actor_given_the_wrong_palette_or_cell_is_an_actor_error() {
    let (mut h, arm) = arm_scene(Quality::high(), 0.0);
    let state = ActorState {
        model: Mat4::IDENTITY,
        palette: &[Mat4::IDENTITY],
        faces: &[],
        visible: true,
    };
    assert!(matches!(
        h.renderer().set_actor(arm, &state),
        Err(RenderError::Actor(_))
    ));
    let mut pack = demo_arm();
    pack.parts[0].mesh = 99;
    assert!(matches!(
        h.renderer().load_actor(&pack),
        Err(RenderError::Actor(_))
    ));
}

#[test]
fn an_actor_glows_and_back_at_zero_draws_as_before() {
    for quality in [Quality::high(), downlevel()] {
        let (mut h, arm) = arm_scene(quality.clone(), 0.0);
        h.render(&camera(), 0.0);
        let before = h.render(&camera(), 0.0);
        let (pipelines, buffers) = (
            h.renderer().pipelines_created(),
            h.renderer().buffers_created(),
        );
        h.renderer().set_actor_glow(arm, 0.25, 0.6).unwrap();
        let lit = h.render(&camera(), 0.0);
        let brighter = |a: &RgbaImage, b: &RgbaImage| {
            a.pixels()
                .zip(b.pixels())
                .filter(|(a, b)| {
                    let sum = |p: &image::Rgba<u8>| p[0] as u32 + p[1] as u32 + p[2] as u32;
                    sum(b) > sum(a) + 30
                })
                .count()
        };
        assert!(brighter(&before, &lit) > 500, "{quality:?}");
        h.renderer().set_actor_glow(arm, 0.0, 0.0).unwrap();
        let after = h.render(&camera(), 0.0);
        assert_eq!(
            compare_images(&before, &after).unwrap().max_channel_delta,
            0
        );
        assert_eq!(h.renderer().pipelines_created(), pipelines);
        assert_eq!(h.renderer().buffers_created(), buffers);
    }
}
