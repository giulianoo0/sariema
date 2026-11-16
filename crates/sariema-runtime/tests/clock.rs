// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use sariema_runtime::clock::FixedClock;

fn steps_for(frames: &[Duration], cap: u32) -> (u64, u64) {
    let mut clock = FixedClock::new(cap);
    let (mut run, mut dropped) = (0, 0);
    for &dt in frames {
        let tick = clock.advance(dt);
        run += tick.steps as u64;
        dropped += tick.dropped as u64;
        assert!(
            (0.0..1.0).contains(&clock.alpha()),
            "alpha {}",
            clock.alpha()
        );
    }
    (run, dropped)
}

#[test]
fn one_second_is_sixty_steps_at_any_display_rate() {
    for hz in [30u64, 60, 120, 144, 165] {
        let frames = vec![Duration::from_nanos(1_000_000_000 / hz); hz as usize];
        let (run, dropped) = steps_for(&frames, 5);
        assert!((59..=60).contains(&run), "{hz} Hz ran {run}");
        assert_eq!(dropped, 0);
    }
}

#[test]
fn exact_frame_times_never_drift() {
    let frames = vec![Duration::from_nanos(1_000_000_000 / 60 + 1); 60 * 600];
    let (run, _) = steps_for(&frames, 5);
    assert_eq!(run, 60 * 600);
}

#[test]
fn irregular_frames_run_the_same_number_of_steps_as_their_total() {
    let mut frames = Vec::new();
    let mut seed = 7u32;
    let mut total = Duration::ZERO;
    for _ in 0..2000 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let dt = Duration::from_micros(2_000 + (seed >> 16) as u64 % 30_000);
        total += dt;
        frames.push(dt);
    }
    let (run, dropped) = steps_for(&frames, 5);
    let expected = (total.as_nanos() * 60 / 1_000_000_000) as u64;
    assert_eq!(run + dropped, expected);
    assert_eq!(dropped, 0);
}

#[test]
fn a_long_stall_runs_at_most_the_cap_and_drops_the_rest() {
    let mut clock = FixedClock::new(5);
    let tick = clock.advance(Duration::from_secs(2));
    assert_eq!(tick.steps, 5);
    assert_eq!(tick.dropped, 115);
    assert!(clock.alpha() < 1.0);
    let next = clock.advance(Duration::from_nanos(16_666_667));
    assert_eq!(next.steps, 1);
}

#[test]
fn alpha_is_the_fraction_of_a_step_left_over() {
    let mut clock = FixedClock::new(5);
    let tick = clock.advance(Duration::from_nanos(25_000_000));
    assert_eq!(tick.steps, 1);
    assert!((clock.alpha() - 0.5).abs() < 1e-6, "{}", clock.alpha());
}
