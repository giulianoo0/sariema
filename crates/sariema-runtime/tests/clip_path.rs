// SPDX-License-Identifier: Apache-2.0

//! A clip path moves an actor's root only within spans of its clip, from
//! the clip's time: still while the clip anticipates and settles, slower
//! where a span slows the clip, and ending exactly on each span's point.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_core::STEP_SECONDS;
use sariema_format::script::{ClipMode, ClipSpan, Command, Ease, Kind, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

const CLIP: &str = "vault";
const LENGTH: f32 = 1.0;

fn span(from: f32, until: f32, rate: f32, to: Option<[f32; 3]>, ease: Ease) -> ClipSpan {
    ClipSpan {
        from,
        until,
        rate,
        to,
        yaw: None,
        ease,
        arc: 0.0,
    }
}

fn path(spans: &[ClipSpan], from: f32, hold: f32, out: &mut Vec<u8>) {
    let mut bytes = Vec::new();
    spans.iter().for_each(|s| s.put(&mut bytes));
    send(
        out,
        Command::ClipPath {
            entity: entity(0),
            clip: name_id(CLIP),
            mode: ClipMode::Hold,
            blend: 0.1,
            from,
            hold,
            hold_end: false,
            spans: &bytes,
        },
    );
}

struct Trace {
    /// Each step: the root, and the clip's time.
    steps: Vec<(Vec3, f32)>,
    events: Vec<(u64, String)>,
}

/// Actor 0 spawned at the origin, `start(out)` sent at step 1, `then` sent
/// at the steps it names.
fn run(
    start: impl Fn(&mut Vec<u8>) + 'static,
    then: impl Fn(u64, &mut Vec<u8>) + 'static,
    steps: u64,
) -> Trace {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let logic = native(move |n, events, out| {
        log.borrow_mut()
            .extend(events.iter().map(|e| (n, format!("{e:?}"))));
        if n == 0 {
            send(
                out,
                Command::Spawn {
                    entity: entity(0),
                    kind: Kind::Actor,
                    asset: 0,
                },
            );
        }
        if n == 1 {
            start(out);
        }
        then(n, out);
    });
    let settings = Settings::default();
    let mut sim = Simulation::new(settings.clone(), Vec::new(), logic, 1);
    sim.learn_clip(0, name_id(CLIP), LENGTH);
    sim.learn_clip(0, name_id("land"), 0.5);
    let actions = Actions::for_settings(&settings.input);
    let mut trace = Vec::new();
    for _ in 0..steps {
        sim.step(&actions);
        let node = sim.state().world.node(entity(0)).unwrap();
        let t = sim.state().clip_time(entity(0)).map_or(-1.0, |c| c.1);
        trace.push((node.local.position, t));
    }
    Trace {
        steps: trace,
        events: seen.take(),
    }
}

fn hop() -> Vec<ClipSpan> {
    vec![
        span(0.2, 0.5, 1.0, Some([1.0, 0.0, 0.0]), Ease::Linear),
        span(0.5, 0.6, 0.4, None, Ease::Linear),
        span(0.6, 0.8, 1.0, Some([1.0, 0.0, -1.0]), Ease::InQuad),
    ]
}

fn moving(t: f32) -> bool {
    (0.2..=0.5).contains(&t) || (0.6..=0.8).contains(&t)
}

#[test]
fn the_root_moves_only_within_its_spans_and_lands_on_their_points() {
    let trace = run(|out| path(&hop(), 0.0, 0.0, out), |_, _| {}, 90);
    let mut moved_at = Vec::new();
    for pair in trace.steps.windows(2) {
        let ((a, t0), (b, t1)) = (pair[0], pair[1]);
        if a != b {
            assert!(
                moving(t1) || moving(t0) || (t0 < 0.2 && t1 > 0.2) || (t0 < 0.6 && t1 > 0.6),
                "the root moved from {a} to {b} between clip times {t0} and {t1}"
            );
            moved_at.push(t1);
        }
    }
    assert!(!moved_at.is_empty());
    let still: Vec<f32> = (trace.steps.iter())
        .filter(|(_, t)| *t > 0.5 && *t < 0.6)
        .map(|(p, _)| p.distance(Vec3::X))
        .collect();
    assert!(
        !still.is_empty() && still.iter().all(|&d| d < 1e-6),
        "{still:?}"
    );
    let slow = (trace.steps.iter())
        .filter(|(_, t)| *t > 0.5 && *t < 0.6)
        .count();
    assert!(
        (13..=15).contains(&slow),
        "0.1 s of clip at 0.4 lasts 0.25 s, 15 steps: {slow}"
    );
    let (end, _) = *trace.steps.last().unwrap();
    assert!(end.distance(Vec3::new(1.0, 0.0, -1.0)) < 1e-6, "{end}");
    let arrived: Vec<u64> = (trace.events.iter())
        .filter(|(_, e)| e.starts_with("Arrived"))
        .map(|(n, _)| *n)
        .collect();
    let world = 0.5 + 0.1 / 0.4 + 0.2;
    let due = 1 + (world / STEP_SECONDS).ceil() as u64;
    assert_eq!(arrived.len(), 1, "{:?}", trace.events);
    assert!(
        arrived[0].abs_diff(due + 1) <= 1,
        "arrived the step after the last span ends: {} against {}",
        arrived[0],
        due + 1
    );
}

#[test]
fn a_held_path_starts_part_way_into_its_clip_after_its_hold() {
    let spans = [span(0.3, 0.5, 1.0, Some([0.5, 0.0, 0.0]), Ease::Linear)];
    let trace = run(move |out| path(&spans, 0.25, 0.1, out), |_, _| {}, 30);
    let held = (trace.steps.iter()).filter(|(_, t)| *t == 0.25).count();
    assert_eq!(held, 7, "the step it began and the 6 held");
    let (p, t) = trace.steps[10];
    assert!((t - (0.25 + 3.0 * STEP_SECONDS)).abs() < 1e-5, "{t}");
    assert_eq!(p, Vec3::ZERO, "still before its span");
}

#[test]
fn another_clip_stops_the_path_where_the_root_stands() {
    let trace = run(
        |out| path(&hop(), 0.0, 0.0, out),
        |n, out| {
            if n == 20 {
                send(
                    out,
                    Command::PlayClip {
                        entity: entity(0),
                        clip: name_id("land"),
                        mode: ClipMode::Once,
                        blend: 0.1,
                        speed: 1.0,
                    },
                );
            }
        },
        60,
    );
    let at = trace.steps[19].0;
    assert!(at.x > 0.0 && at.x < 1.0, "{at}");
    assert!(trace.steps[20..].iter().all(|(p, _)| *p == at));
    assert!(
        trace
            .events
            .iter()
            .any(|(n, e)| *n == 21 && e.starts_with("Blocked"))
    );
    assert!(!trace.events.iter().any(|(_, e)| e.starts_with("Arrived")));
}

fn refused(spans: Vec<ClipSpan>, code: &str) {
    let trace = run(move |out| path(&spans, 0.0, 0.0, out), |_, _| {}, 4);
    assert!(
        trace.events.iter().any(|(_, e)| e.contains(code)),
        "{code}: {:?}",
        trace.events
    );
    assert!(trace.steps.iter().all(|(p, _)| *p == Vec3::ZERO));
}

#[test]
fn spans_out_of_order_past_the_clip_or_too_fast_are_refused() {
    let to = Some([1.0, 0.0, 0.0]);
    refused(
        vec![
            span(0.4, 0.5, 1.0, to, Ease::Linear),
            span(0.3, 0.6, 1.0, to, Ease::Linear),
        ],
        "E_CLIP_SPANS",
    );
    refused(vec![span(0.4, 1.2, 1.0, to, Ease::Linear)], "E_CLIP_SPANS");
    refused(vec![span(0.4, 0.3, 1.0, to, Ease::Linear)], "E_CLIP_SPANS");
    refused(vec![span(0.4, 0.5, 0.0, to, Ease::Linear)], "E_CLIP_SPANS");
    refused(
        vec![span(0.4, 0.45, 1.0, Some([30.0, 0.0, 0.0]), Ease::Linear)],
        "E_TELEPORT_VISIBLE",
    );
}

#[test]
fn a_queued_clip_follows_the_path_with_no_still_frame() {
    let trace = run(
        |out| {
            path(&hop(), 0.0, 0.0, out);
            send(
                out,
                Command::ClipNext {
                    entity: entity(0),
                    clip: name_id("land"),
                    mode: ClipMode::Once,
                    blend: 0.1,
                    speed: 1.0,
                    after: 0.0,
                },
            );
        },
        |_, _| {},
        120,
    );
    let ends = (trace.steps.windows(2))
        .position(|w| w[1].1 < w[0].1)
        .unwrap();
    let (before, after) = (trace.steps[ends].1, trace.steps[ends + 1].1);
    let world = 0.5 + 0.1 / 0.4 + (LENGTH - 0.6);
    let left = (ends as f32 + 1.0) * STEP_SECONDS - STEP_SECONDS - world;
    assert!(before < LENGTH && after > 0.0, "{before} {after}");
    assert!(
        (after - left).abs() < 1e-4,
        "the time left over: {after} against {left}"
    );
}

#[test]
fn a_clip_at_a_speed_of_zero_or_below_is_refused() {
    let trace = run(
        |out| {
            for speed in [0.0, -1.0] {
                send(
                    out,
                    Command::PlayClip {
                        entity: entity(0),
                        clip: name_id(CLIP),
                        mode: ClipMode::Once,
                        blend: 0.1,
                        speed,
                    },
                );
                send(
                    out,
                    Command::ClipNext {
                        entity: entity(0),
                        clip: name_id("land"),
                        mode: ClipMode::Once,
                        blend: 0.1,
                        speed,
                        after: 0.0,
                    },
                );
            }
        },
        |_, _| {},
        4,
    );
    let refused = (trace.events.iter())
        .filter(|(_, e)| e.contains("E_CLIP_RATE"))
        .count();
    assert_eq!(refused, 4, "{:?}", trace.events);
    assert!(trace.steps.iter().all(|(_, t)| *t == -1.0), "no clip plays");
}
