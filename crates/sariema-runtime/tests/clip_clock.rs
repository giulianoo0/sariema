// SPDX-License-Identifier: Apache-2.0

//! The clip clock: a clip's time follows the steps at its rate, which can
//! change without starting it over, at once or at a time of the clip; a
//! clip can hold its first frame, start part way in, and hand over to the
//! clip queued after it with the time left over.

use sariema_core::STEP_SECONDS;
use sariema_format::script::{ClipMode, Entity};
use sariema_runtime::clips::{ClipClock, Next};
use sariema_runtime::outbox::ClipRequest;

const E: Entity = Entity {
    index: 0,
    generation: 0,
};
const A: u32 = 11;
const B: u32 = 12;
const C: u32 = 13;

fn steps(clock: &mut ClipClock, n: u32) -> Vec<ClipRequest> {
    let mut out = Vec::new();
    for _ in 0..n {
        clock.step(&mut out);
    }
    out
}

fn time(clock: &ClipClock) -> (u32, f32) {
    clock.time(E).unwrap()
}

#[test]
fn a_clip_at_one_rate_is_at_its_steps_times_its_rate() {
    let mut clock = ClipClock::default();
    clock.play(E, A, 1.3);
    assert_eq!(time(&clock), (A, 0.0));
    steps(&mut clock, 37);
    assert_eq!(time(&clock), (A, 37.0 * STEP_SECONDS * 1.3));
    assert_eq!(
        clock.time_at(E, 0.25),
        Some((A, (37.0 - 1.0 + 0.25) * STEP_SECONDS * 1.3))
    );
}

#[test]
fn a_new_rate_goes_on_from_the_time_the_clip_is_at() {
    let mut clock = ClipClock::default();
    clock.play(E, A, 1.0);
    steps(&mut clock, 30);
    let (_, at) = time(&clock);
    assert!(clock.set_rate(E, A, 0.5));
    assert_eq!(time(&clock), (A, at), "no jump in the step it changed");
    let drawn: Vec<f32> = (0..=4)
        .map(|k| clock.time_at(E, k as f32 / 4.0).unwrap().1)
        .collect();
    assert!(
        (drawn[0] - (at - STEP_SECONDS)).abs() < 1e-6 && drawn[4] == at,
        "the frames before the next step draw the step it changed in: {drawn:?}"
    );
    steps(&mut clock, 10);
    let (_, t) = time(&clock);
    assert!((t - (at + 10.0 * STEP_SECONDS * 0.5)).abs() < 1e-6, "{t}");
    assert!(
        !clock.set_rate(E, B, 2.0),
        "a rate for a clip it does not play"
    );
}

#[test]
fn a_rate_due_at_a_time_of_the_clip_takes_over_within_the_step() {
    let mut clock = ClipClock::default();
    clock.play(E, A, 1.0);
    clock.rate_at(E, 0.205, 0.4);
    clock.rate_at(E, 0.305, 1.0);
    let mut last = 0.0;
    for n in 1..=60 {
        steps(&mut clock, 1);
        let world = n as f32 * STEP_SECONDS;
        let want = if world <= 0.205 {
            world
        } else if world <= 0.205 + 0.1 / 0.4 {
            0.205 + (world - 0.205) * 0.4
        } else {
            0.305 + (world - 0.205 - 0.1 / 0.4)
        };
        let (_, t) = time(&clock);
        assert!((t - want).abs() < 1e-5, "step {n}: {t} against {want}");
        assert!(t > last);
        last = t;
    }
}

#[test]
fn a_held_clip_stays_on_its_first_frame_then_plays_from_there() {
    let mut clock = ClipClock::default();
    clock.play_from(E, A, 1.0, 0.25);
    clock.hold(E, 6);
    for _ in 0..6 {
        steps(&mut clock, 1);
        assert_eq!(time(&clock), (A, 0.25));
    }
    steps(&mut clock, 1);
    let (_, t) = time(&clock);
    assert!((t - (0.25 + STEP_SECONDS)).abs() < 1e-6, "{t}");
}

fn next(clip: u32, after: Option<f32>, length: Option<f32>) -> Next {
    Next {
        clip,
        mode: ClipMode::Once,
        blend: 0.1,
        rate: 1.0,
        after,
        length,
    }
}

#[test]
fn a_queued_clip_follows_with_the_time_left_over() {
    let mut clock = ClipClock::default();
    clock.play_from(E, A, 1.0, 0.0);
    clock.set_length(E, Some(0.51));
    clock.then(E, next(B, None, Some(0.4)));
    clock.then(E, next(C, Some(0.2), Some(1.0)));
    let early = steps(&mut clock, 30);
    assert!(early.is_empty());
    let out = steps(&mut clock, 1);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(
        (out[0].clip, out[0].mode, out[0].blend),
        (B, ClipMode::Once, 0.1)
    );
    let (clip, t) = time(&clock);
    assert_eq!(clip, B);
    assert!((t - (31.0 * STEP_SECONDS - 0.51)).abs() < 1e-5, "{t}");
    let drawn = clock.time_at(E, 0.0).unwrap();
    assert_eq!(drawn, (B, 0.0), "drawn from the start of the clip it plays");
    assert!(!clock.through(E));
    let out = steps(&mut clock, 12);
    assert_eq!(out.iter().map(|r| r.clip).collect::<Vec<_>>(), [C]);
    let (_, t) = time(&clock);
    assert!(
        (t - (43.0 * STEP_SECONDS - 0.51 - 0.2)).abs() < 1e-5,
        "C begins when B has played 0.2 s, not its whole length: {t}"
    );
    steps(&mut clock, 59);
    assert!(!clock.through(E));
    steps(&mut clock, 1);
    assert!(clock.through(E), "the last clip played to its end");
}

#[test]
fn playing_another_clip_drops_what_was_queued_and_due() {
    let mut clock = ClipClock::default();
    clock.play_from(E, A, 1.0, 0.0);
    clock.set_length(E, Some(0.1));
    clock.rate_at(E, 0.05, 0.2);
    clock.then(E, next(B, None, Some(1.0)));
    steps(&mut clock, 1);
    clock.play(E, C, 1.0);
    let out = steps(&mut clock, 30);
    assert!(out.is_empty());
    assert_eq!(time(&clock), (C, 30.0 * STEP_SECONDS));
}
