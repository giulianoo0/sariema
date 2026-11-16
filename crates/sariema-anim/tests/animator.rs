// SPDX-License-Identifier: Apache-2.0

mod common;

use common::*;
use glam::{Mat4, Vec3};
use sariema_anim::{
    AnimSettings, Animator, ClipId, Event, EventKind, JointMask, LayerBlend, Play, PlayMode, Rig,
    Transform,
};
use sariema_format::actor::demo::demo_face;

fn instant() -> Play {
    Play {
        fade_s: Some(0.0),
        ..Play::default()
    }
}

/// The largest rotation, in degrees, of any joint between two local poses.
fn turn(a: &[Transform], b: &[Transform]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| a.rotation.angle_between(b.rotation).to_degrees())
        .fold(0.0, f32::max)
}

/// The largest joint turn of each step over 30 steps after `setup` and the
/// `play` it is then given.
fn turns_per_step(setup: impl Fn(&Rig, &mut Animator), next: &str, fade_s: f32) -> Vec<f32> {
    let rig = arm();
    let mut a = animator(&rig);
    setup(&rig, &mut a);
    a.evaluate(&rig, 1.0);
    let mut last = a.pose().to_vec();
    let play = Play {
        fade_s: Some(fade_s),
        ..Play::default()
    };
    a.play(&rig, 0, clip(&rig, next), play);
    a.evaluate(&rig, 1.0);
    assert_eq!(
        a.pose(),
        last,
        "playing must not change the pose before a step"
    );
    (0..30)
        .map(|_| {
            a.step(&rig);
            a.evaluate(&rig, 1.0);
            let moved = turn(&last, a.pose());
            last = a.pose().to_vec();
            moved
        })
        .collect()
}

fn bending(rig: &Rig, a: &mut Animator) {
    a.play(rig, 0, clip(rig, "bend"), instant());
    for _ in 0..20 {
        a.step(rig);
    }
}

/// `bend`, then `wave`, interrupted 3 steps into the fade.
fn mid_fade(rig: &Rig, a: &mut Animator) {
    bending(rig, a);
    a.play(rig, 0, clip(rig, "wave"), Play::default());
    for _ in 0..3 {
        a.step(rig);
    }
}

/// A 0.13 s fade spans about 8 steps and smoothstep's steepest slope is 1.5,
/// so no step may turn a joint by more than a quarter of an instant switch.
#[test]
fn a_crossfade_turns_no_joint_by_more_than_a_quarter_of_a_switch() {
    let jump = turns_per_step(bending, "wave", 0.0)[0];
    let faded = turns_per_step(bending, "wave", 0.13);
    let worst = faded.iter().copied().fold(0.0, f32::max);
    assert!(jump > 45.0, "an instant switch turns {jump}°");
    assert!(
        worst <= jump / 4.0,
        "a joint turned {worst}° of {jump}° in one step: {faded:?}"
    );
}

#[test]
fn interrupting_a_fade_starts_from_the_blended_pose() {
    let jump = turns_per_step(mid_fade, "spin", 0.0)[0];
    let faded = turns_per_step(mid_fade, "spin", 0.13);
    let worst = faded.iter().copied().fold(0.0, f32::max);
    assert!(jump > 45.0, "an instant switch turns {jump}°");
    assert!(
        faded[0] <= jump / 10.0,
        "the first step turned {}° of {jump}°",
        faded[0]
    );
    assert!(
        worst <= jump / 4.0,
        "a joint turned {worst}° of {jump}°: {faded:?}"
    );
}

#[test]
fn a_crossfade_ends_exactly_on_the_target_clip() {
    let rig = arm();
    let (mut faded, mut direct) = (animator(&rig), animator(&rig));
    for a in [&mut faded, &mut direct] {
        a.play(&rig, 0, clip(&rig, "bend"), instant());
        for _ in 0..20 {
            a.step(&rig);
        }
    }
    faded.play(&rig, 0, clip(&rig, "wave"), Play::default());
    direct.play(&rig, 0, clip(&rig, "wave"), instant());
    let steps_in_fade = (0.13 / STEP).ceil() as usize;
    for step in 0..30 {
        faded.step(&rig);
        direct.step(&rig);
        faded.evaluate(&rig, 1.0);
        direct.evaluate(&rig, 1.0);
        if step + 1 >= steps_in_fade {
            assert_eq!(faded.palette(), direct.palette(), "step {step}");
            assert!(!faded.is_fading(0));
        } else {
            assert!(
                faded.is_fading(0) && faded.palette() != direct.palette(),
                "step {step}"
            );
        }
    }
}

#[test]
fn playing_the_clip_already_playing_is_a_no_op() {
    let rig = arm();
    let (mut again, mut alone) = (animator(&rig), animator(&rig));
    for a in [&mut again, &mut alone] {
        a.play(&rig, 0, clip(&rig, "bend"), instant());
    }
    for step in 0..60 {
        if step % 7 == 3 {
            again.play(&rig, 0, clip(&rig, "bend"), Play::default());
        }
        again.step(&rig);
        alone.step(&rig);
        again.evaluate(&rig, 1.0);
        alone.evaluate(&rig, 1.0);
        assert_eq!(again.palette(), alone.palette(), "step {step}");
        assert!(!again.is_fading(0));
    }
}

#[test]
fn restart_plays_the_same_clip_over() {
    let rig = arm();
    let mut a = animator(&rig);
    a.play(&rig, 0, clip(&rig, "bend"), instant());
    for _ in 0..10 {
        a.step(&rig);
    }
    let restart = Play {
        restart: true,
        ..instant()
    };
    a.play(&rig, 0, clip(&rig, "bend"), restart);
    assert_eq!(a.clip_time(0), 0.0);
}

/// Steps 1-based at which each event of layer 0 fired.
fn events_by_step(rig: &Rig, a: &mut Animator, steps: usize) -> Vec<(usize, EventKind)> {
    let mut seen = Vec::new();
    for step in 1..=steps {
        a.step(rig);
        seen.extend(a.events().iter().map(|e| (step, e.kind)));
    }
    seen
}

#[test]
fn markers_and_the_end_fire_once_at_their_step() {
    let rig = arm();
    let mut a = animator(&rig);
    a.play(&rig, 0, clip(&rig, "wave"), instant());
    let tap = EventKind::Marker(0);
    let end = EventKind::Marker(1);
    assert_eq!(
        events_by_step(&rig, &mut a, 120),
        [(15, tap), (48, end), (48, EventKind::Finished)]
    );
}

#[test]
fn events_follow_the_speed_and_every_loop_wrap() {
    let rig = arm();
    let mut a = animator(&rig);
    let fast = Play {
        speed: 1.5,
        ..instant()
    };
    a.play(&rig, 0, clip(&rig, "bend"), fast);
    let kick = EventKind::Marker(0);
    let looped = EventKind::Looped;
    assert_eq!(
        events_by_step(&rig, &mut a, 100),
        [
            (20, kick),
            (40, looped),
            (60, kick),
            (80, looped),
            (100, kick)
        ]
    );
    let mut slow = animator(&rig);
    let half = Play {
        speed: 0.5,
        ..instant()
    };
    slow.play(&rig, 0, clip(&rig, "wave"), half);
    let seen = events_by_step(&rig, &mut slow, 120);
    assert_eq!(seen[0], (30, EventKind::Marker(0)));
    assert_eq!(seen.last(), Some(&(96, EventKind::Finished)));
}

#[test]
fn a_step_past_several_wraps_reports_each() {
    let rig = arm();
    let settings = AnimSettings {
        step_s: 2.5,
        ..AnimSettings::default()
    };
    let mut a = Animator::new(&rig, settings);
    a.play(&rig, 0, clip(&rig, "bend"), instant());
    a.step(&rig);
    let kinds: Vec<EventKind> = a.events().iter().map(|e| e.kind).collect();
    let (kick, looped) = (EventKind::Marker(0), EventKind::Looped);
    assert_eq!(kinds, [kick, looped, kick, looped, kick]);
    assert!((a.clip_time(0) - 0.5).abs() < 1e-6);
}

#[test]
fn a_seek_takes_the_clip_to_its_time_with_the_markers_on_the_way() {
    let rig = arm();
    let mut a = animator(&rig);
    let still = Play {
        speed: 0.0,
        ..instant()
    };
    let kinds = |a: &Animator| a.events().iter().map(|e| e.kind).collect::<Vec<_>>();
    let (kick, looped) = (EventKind::Marker(0), EventKind::Looped);
    a.play(&rig, 0, clip(&rig, "bend"), still);
    a.seek(0, 0.4);
    a.step(&rig);
    assert_eq!((kinds(&a), a.clip_time(0)), (vec![], 0.4));
    a.seek(0, 1.2);
    a.step(&rig);
    assert_eq!(kinds(&a), [kick, looped]);
    assert!((a.clip_time(0) - 0.2).abs() < 1e-6, "{}", a.clip_time(0));
    a.step(&rig);
    assert!(kinds(&a).is_empty());
    assert!((a.clip_time(0) - 0.2).abs() < 1e-6, "a seek lasts one step");
    a.play(&rig, 0, clip(&rig, "wave"), still);
    a.seek(0, 10.0);
    a.step(&rig);
    let (tap, end) = (EventKind::Marker(0), EventKind::Marker(1));
    assert_eq!(kinds(&a), [tap, end, EventKind::Finished]);
}

#[test]
fn a_marker_at_the_start_fires_on_the_first_step() {
    let rig = arm();
    let mut a = animator(&rig);
    let from_kick = Play {
        start_s: 0.5,
        ..instant()
    };
    a.play(&rig, 0, clip(&rig, "bend"), from_kick);
    a.step(&rig);
    let first: Vec<Event> = a.events().to_vec();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].kind, EventKind::Marker(0));
    assert_eq!(first[0].clip, clip(&rig, "bend"));
    a.step(&rig);
    assert!(a.events().is_empty());
}

fn run(rig: &Rig) -> Vec<u32> {
    let mut a = animator(rig);
    let mut bits = Vec::new();
    a.play(rig, 0, clip(rig, "bend"), Play::default());
    for step in 0..200 {
        match step {
            30 => a.play(rig, 0, clip(rig, "wave"), Play::default()),
            33 => a.play(
                rig,
                0,
                clip(rig, "spin"),
                Play {
                    speed: 1.3,
                    ..Play::default()
                },
            ),
            90 => a.play(rig, 1, clip(rig, "wave"), Play::default()),
            _ => {}
        }
        a.step(rig);
        a.evaluate(rig, 0.37);
        bits.extend(
            a.palette()
                .iter()
                .flat_map(|m| m.to_cols_array())
                .map(f32::to_bits),
        );
    }
    bits
}

#[test]
fn the_same_commands_give_bit_identical_palettes() {
    let rig = arm();
    let first = run(&rig);
    assert_eq!(first, run(&rig));
    assert_eq!(first, run(&arm()));
}

#[test]
fn rendering_between_two_steps_lies_between_their_poses() {
    let rig = arm();
    let mut a = animator(&rig);
    a.play(&rig, 0, clip(&rig, "bend"), Play::default());
    a.play(&rig, 1, clip(&rig, "spin"), Play::default());
    for _ in 0..90 {
        a.step(&rig);
        a.evaluate(&rig, 0.0);
        let before = a.pose().to_vec();
        a.evaluate(&rig, 1.0);
        let after = a.pose().to_vec();
        a.evaluate(&rig, 0.5);
        for ((mid, b), c) in a.pose().iter().zip(&before).zip(&after) {
            let span = b.rotation.angle_between(c.rotation);
            assert!(b.rotation.angle_between(mid.rotation) <= span + 1e-5);
            assert!(mid.rotation.angle_between(c.rotation) <= span + 1e-5);
            let lo = b.translation.min(c.translation) - 1e-6;
            let hi = b.translation.max(c.translation) + 1e-6;
            assert!(mid.translation.cmpge(lo).all() && mid.translation.cmple(hi).all());
        }
    }
}

#[test]
fn a_masked_layer_drives_only_its_joints() {
    let rig = arm();
    let mut a = animator(&rig);
    let wrist = rig.joint("wrist").unwrap();
    a.set_layer(
        1,
        LayerBlend::Override,
        Some(&JointMask::subtree(&rig, wrist)),
    );
    a.play(&rig, 0, clip(&rig, "bend"), Play::default());
    a.play(&rig, 1, clip(&rig, "wave"), Play::default());
    for _ in 0..24 {
        a.step(&rig);
    }
    a.evaluate(&rig, 1.0);
    let pose = a.pose();
    let (bend, wave) = (sample(&rig, "bend", 0.4), sample(&rig, "wave", 0.4));
    assert!(pose[1].rotation.angle_between(bend[1].rotation) < 1e-4);
    assert!(pose[2].rotation.angle_between(wave[2].rotation) < 1e-4);
    assert_close(pose[2].scale, wave[2].scale, 1e-5);
}

#[test]
fn an_additive_layer_adds_its_motion() {
    let rig = arm();
    let mut a = animator(&rig);
    a.set_layer(1, LayerBlend::Additive, None);
    a.play(&rig, 0, clip(&rig, "bend"), instant());
    a.play(&rig, 1, clip(&rig, "bend"), instant());
    for _ in 0..15 {
        a.step(&rig);
    }
    a.evaluate(&rig, 1.0);
    assert!((angle_about(a.pose()[1].rotation, Vec3::Z) - 45.0 * 2.0).abs() < 1e-3);
}

#[test]
fn hold_last_stays_and_once_fades_back() {
    let rig = arm();
    let (mut hold, mut once) = (animator(&rig), animator(&rig));
    let mode = |mode| Play {
        mode: Some(mode),
        start_s: 0.3,
        ..instant()
    };
    hold.play(&rig, 0, clip(&rig, "bend"), mode(PlayMode::HoldLast));
    once.play(
        &rig,
        0,
        clip(&rig, "bend"),
        Play {
            fade_s: Some(0.1),
            ..mode(PlayMode::Once)
        },
    );
    for _ in 0..100 {
        hold.step(&rig);
        once.step(&rig);
    }
    hold.evaluate(&rig, 1.0);
    once.evaluate(&rig, 1.0);
    assert_eq!(hold.pose(), sample(&rig, "bend", 1.0));
    assert_eq!(once.pose(), rig.rest_pose());
    assert_eq!(once.clip(0), None);
    assert_eq!(hold.clip_time(0), 1.0);
}

#[test]
fn stopping_an_overlay_fades_back_to_the_base() {
    let rig = arm();
    let mut a = animator(&rig);
    a.play(&rig, 0, clip(&rig, "bend"), instant());
    a.play(&rig, 1, clip(&rig, "spin"), instant());
    a.step(&rig);
    a.stop(1, Some(0.1));
    for _ in 0..10 {
        a.step(&rig);
    }
    a.evaluate(&rig, 1.0);
    let time = a.clip_time(0);
    assert_eq!(a.pose(), sample(&rig, "bend", time));
}

#[test]
fn faces_follow_clip_tracks_or_what_was_set() {
    let pack = demo_face();
    let rig = Rig::new(&pack).unwrap();
    let eyes = rig.face("eyes").unwrap();
    let mut a = Animator::new(&rig, AnimSettings::default());
    assert_eq!(a.faces(), [0]);
    a.set_face(eyes, rig.face_frame(eyes, "happy").unwrap());
    a.step(&rig);
    assert_eq!(a.faces(), [2]);
    let nod: ClipId = rig.clip("nod").unwrap();
    a.play(&rig, 0, nod, instant());
    let mut cells = Vec::new();
    for _ in 0..40 {
        a.step(&rig);
        cells.push(a.faces()[0]);
    }
    assert_eq!(cells[..29], [0; 29]);
    assert_eq!(cells[30..], [1; 10]);
    a.stop(0, Some(0.0));
    a.step(&rig);
    assert_eq!(a.faces(), [2]);
}

#[test]
fn sockets_follow_their_joint() {
    let rig = arm();
    let mut a = animator(&rig);
    let tip_socket = rig.socket("tip").unwrap();
    a.evaluate(&rig, 1.0);
    assert_eq!(
        a.socket(&rig, tip_socket),
        Mat4::from_translation(Vec3::new(0.0, 1.2, 0.0))
    );
    assert_eq!(rig.socket("nope"), None);
}
