// SPDX-License-Identifier: Apache-2.0

mod common;

use common::*;
use glam::{Mat4, Quat, Vec3};
use sariema_anim::{Play, PlayMode, Rig};
use sariema_format::actor::demo::demo_arm;
use sariema_format::actor::{Interpolation, Values};

#[test]
fn samples_land_on_keys() {
    let rig = arm();
    for (time, degrees) in [(0.0, 0.0), (0.5, 90.0), (1.0, 0.0)] {
        let elbow = sample(&rig, "bend", time)[1].rotation;
        assert!(
            (angle_about(elbow, Vec3::Z) - degrees).abs() < 1e-3,
            "{time}: {elbow}"
        );
    }
}

#[test]
fn samples_between_keys_interpolate_the_rotation() {
    let rig = arm();
    for (time, degrees) in [(0.25, 45.0), (0.1, 18.0), (0.75, 45.0)] {
        let elbow = sample(&rig, "bend", time)[1].rotation;
        assert!(
            (angle_about(elbow, Vec3::Z) - degrees).abs() < 1e-3,
            "{time}: {elbow}"
        );
    }
}

#[test]
fn joints_without_a_channel_keep_their_rest_pose() {
    let rig = arm();
    let pose = sample(&rig, "bend", 0.3);
    assert_eq!(pose[0], rig.rest_pose()[0]);
    assert_eq!(pose[2], rig.rest_pose()[2]);
}

#[test]
fn clip_time_wraps_loops_and_clamps_the_rest() {
    let cases = [
        (PlayMode::Loop, -0.25, 0.75),
        (PlayMode::Loop, 1.25, 0.25),
        (PlayMode::Loop, 3.0, 0.0),
        (PlayMode::Once, -0.25, 0.0),
        (PlayMode::Once, 1.25, 1.0),
        (PlayMode::HoldLast, -0.25, 0.0),
        (PlayMode::HoldLast, 1.25, 1.0),
    ];
    for (mode, time, expected) in cases {
        let got = mode.clip_time(time, 1.0);
        assert!((got - expected).abs() < 1e-9, "{mode:?} at {time}: {got}");
    }
}

#[test]
fn sampling_clamps_before_the_start_and_past_the_end() {
    let rig = arm();
    assert_eq!(sample(&rig, "bend", -1.0), sample(&rig, "bend", 0.0));
    assert_eq!(sample(&rig, "wave", 5.0), sample(&rig, "wave", 0.8));
}

#[test]
fn a_start_offset_before_zero_or_past_the_end_follows_the_mode() {
    let rig = arm();
    let started_at = |mode, start_s| {
        let mut a = animator(&rig);
        let play = Play {
            mode: Some(mode),
            start_s,
            fade_s: Some(0.0),
            ..Play::default()
        };
        a.play(&rig, 0, clip(&rig, "bend"), play);
        a.clip_time(0)
    };
    assert!((started_at(PlayMode::Loop, -0.25) - 0.75).abs() < 1e-6);
    assert!((started_at(PlayMode::Loop, 1.5) - 0.5).abs() < 1e-6);
    assert_eq!(started_at(PlayMode::Once, -0.25), 0.0);
    assert_eq!(started_at(PlayMode::HoldLast, 1.5), 1.0);
}

#[test]
fn palette_matches_hand_computed_matrices() {
    let rig = arm();
    let mut a = animator(&rig);
    let play = Play {
        start_s: 0.5 - STEP,
        fade_s: Some(0.0),
        ..Play::default()
    };
    a.play(&rig, 0, clip(&rig, "bend"), play);
    a.step(&rig);
    a.evaluate(&rig, 1.0);
    let up = |y: f32| Mat4::from_translation(Vec3::new(0.0, y, 0.0));
    let turn = Mat4::from_rotation_z(90f32.to_radians());
    let expected = [
        Mat4::IDENTITY,
        up(0.5) * turn * up(-0.5),
        up(0.5) * turn * up(0.5) * up(-1.0),
    ];
    for (j, (got, want)) in a.palette().iter().zip(expected).enumerate() {
        assert!(
            got.abs_diff_eq(want, 1e-5),
            "joint {j}: {got} instead of {want}"
        );
    }
    assert_close(tip(&a), Vec3::new(-0.7, 0.5, 0.0), 1e-5);
    let socket = a.socket(&rig, rig.socket("tip").unwrap());
    assert_close(socket.w_axis.truncate(), Vec3::new(-0.7, 0.5, 0.0), 1e-5);
}

#[test]
fn rotation_takes_the_shortest_arc() {
    let rig = arm();
    let shoulder = sample(&rig, "spin", 0.5)[0].rotation;
    let half_turn = Quat::from_rotation_y(180f32.to_radians());
    assert!(shoulder.dot(half_turn).abs() > 1.0 - 1e-5, "{shoulder}");
    let quarter = sample(&rig, "spin", 0.25)[0].rotation;
    assert!(
        (angle_about(quarter, Vec3::Y).abs() - 175.0).abs() < 1e-2,
        "{quarter}"
    );
}

#[test]
fn scale_tracks_scale() {
    let rig = arm();
    assert_close(sample(&rig, "wave", 0.4)[2].scale, Vec3::splat(1.5), 1e-6);
    assert_close(sample(&rig, "wave", 0.2)[2].scale, Vec3::splat(1.25), 1e-6);
    assert_close(sample(&rig, "wave", 0.8)[2].scale, Vec3::ONE, 1e-6);
}

#[test]
fn step_interpolation_holds_each_key() {
    let mut pack = demo_arm();
    pack.clips[0].channels[0].interpolation = Interpolation::Step;
    let rig = Rig::new(&pack).unwrap();
    for (time, degrees) in [(0.25, 0.0), (0.5, 90.0), (0.75, 90.0), (1.0, 0.0)] {
        let elbow = sample(&rig, "bend", time)[1].rotation;
        assert!(
            (angle_about(elbow, Vec3::Z) - degrees).abs() < 1e-3,
            "{time}: {elbow}"
        );
    }
}

#[test]
fn quantized_rotations_sample_close_to_full_precision() {
    let mut pack = demo_arm();
    let Values::Quat(keys) = &pack.clips[0].channels[0].values else {
        unreachable!("the demo stores full quaternions");
    };
    let quantized = keys
        .iter()
        .map(|q| q.map(|c| (c * i16::MAX as f32).round() as i16))
        .collect();
    pack.clips[0].channels[0].values = Values::QuatSnorm16(quantized);
    let rig = Rig::new(&pack).unwrap();
    for time in [0.0, 0.3, 0.5, 0.9] {
        let exact = sample(&arm(), "bend", time)[1].rotation;
        let close = sample(&rig, "bend", time)[1].rotation;
        assert!(exact.angle_between(close).to_degrees() < 0.01, "{time}");
    }
}

#[test]
fn an_invalid_pack_is_an_error() {
    let mut pack = demo_arm();
    pack.joints[0].parent = Some(0);
    assert!(Rig::new(&pack).is_err());
}
