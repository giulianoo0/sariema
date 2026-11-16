// SPDX-License-Identifier: Apache-2.0

//! A clip path over a sill: the hero anticipates in place, goes over the
//! sill with its collider off where no walk could take it, its clip slowed
//! on one span without a jump in the pose, and lands on free floor with its
//! collider on again, so the sill blocks the walk back.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_anim::PlayMode;
use sariema_core::STEP_SECONDS;
use sariema_format::script::{ClipMode, ClipSpan, Command, Ease, Event, name_id};
use sariema_physics::ActorId;

use common::{engine, entity, native, send, spawn_hero, wired};

const START: [f32; 3] = [3.5, 0.0, -1.9];
const TOP: [f32; 3] = [3.5, 0.9, -3.0];
const LANDING: [f32; 3] = [3.5, 0.0, -4.4];
/// In the air over the sill, whose top is at 0.9 m.
const HIGH: [f32; 3] = [3.5, 1.3, -3.0];

/// Up on the sill, slowed on the top, down to `landing`; with `gap`, up in
/// the air over the sill instead, waiting there between two spans.
fn spans(length: f32, landing: [f32; 3], gap: bool) -> Vec<ClipSpan> {
    let at = |f: f32| f * length;
    if gap {
        return vec![
            ClipSpan {
                from: at(0.10),
                until: at(0.30),
                rate: 1.0,
                to: Some(HIGH),
                yaw: None,
                ease: Ease::OutQuad,
                arc: 0.0,
            },
            ClipSpan {
                from: at(0.60),
                until: at(0.85),
                rate: 1.0,
                to: Some(landing),
                yaw: None,
                ease: Ease::InQuad,
                arc: 0.0,
            },
        ];
    }
    vec![
        ClipSpan {
            from: at(0.15),
            until: at(0.45),
            rate: 1.0,
            to: Some(TOP),
            yaw: None,
            ease: Ease::OutQuad,
            arc: 0.0,
        },
        ClipSpan {
            from: at(0.45),
            until: at(0.55),
            rate: 0.4,
            to: None,
            yaw: None,
            ease: Ease::Linear,
            arc: 0.0,
        },
        ClipSpan {
            from: at(0.55),
            until: at(0.85),
            rate: 1.0,
            to: Some(landing),
            yaw: None,
            ease: Ease::InQuad,
            arc: 0.1,
        },
    ]
}

struct Step {
    root: Vec3,
    clock: Option<(u32, f32)>,
    animator: f32,
    overlap: Option<f32>,
    grounded: bool,
}

struct Run {
    trace: Vec<Step>,
    arrived: u64,
    duration: f32,
}

/// The hero at `START` goes over the sill to `landing` from step 5, and
/// walks south from step 200.
fn run(landing: [f32; 3]) -> Run {
    run_spans(landing, false)
}

fn run_spans(landing: [f32; 3], gap: bool) -> Run {
    let length = Rc::new(RefCell::new(0.0f32));
    let arrived = Rc::new(RefCell::new(None));
    let (len, got) = (length.clone(), arrived.clone());
    let logic = native(move |n, events, out| {
        for e in events {
            if matches!(e, Event::Arrived { .. }) && got.borrow().is_none() {
                *got.borrow_mut() = Some(n);
            }
        }
        match n {
            0 => spawn_hero(out, START, 0.0),
            5 => {
                let mut bytes = Vec::new();
                spans(*len.borrow(), landing, gap)
                    .iter()
                    .for_each(|s| s.put(&mut bytes));
                send(
                    out,
                    Command::ClipPath {
                        entity: entity(0),
                        clip: name_id("wave"),
                        mode: ClipMode::Hold,
                        blend: 0.1,
                        from: 0.0,
                        hold: 0.0,
                        spans: &bytes,
                    },
                );
            }
            200 => send(
                out,
                Command::MoveIntent {
                    entity: entity(0),
                    velocity: [0.0, 0.0, 1.5],
                },
            ),
            _ => {}
        }
    });
    let mut engine = engine(logic, wired(), &[]);
    let model = &engine.animation().models[0];
    let clip = model.rig.clip("wave").expect("the hero has a wave");
    let duration = model.rig.clip_info(clip).duration;
    *length.borrow_mut() = duration;
    let actions = sariema_runtime::input::Actions::for_settings(&engine.state().settings.input);
    let mut trace = Vec::new();
    for _ in 0..320 {
        engine.step(&actions);
        let physics = engine.physics().unwrap();
        let collider = physics.world.actor_collider(ActorId(0));
        let a = physics.world.actor(ActorId(0));
        let (_, _, animator) = engine.animation().clip_times().next().unwrap();
        trace.push(Step {
            root: engine.state().world.node(entity(0)).unwrap().local.position,
            clock: engine.state().clip_time(entity(0)),
            animator,
            overlap: physics
                .world
                .actor_overlap(&collider, a.position, a.yaw)
                .map(|o| o.1),
            grounded: a.grounded,
        });
    }
    let arrived = arrived.borrow().expect("arrived");
    Run {
        trace,
        arrived,
        duration,
    }
}

#[test]
fn the_hero_goes_over_the_sill_collider_off_and_lands_on_free_floor() {
    let Run {
        trace,
        arrived: landed,
        duration,
    } = run(LANDING);
    let stood = trace[5].root;
    assert!(
        trace[6..6 + (0.15 * duration / STEP_SECONDS) as usize]
            .iter()
            .all(|s| s.root == stood),
        "still while the clip anticipates"
    );
    let inside = trace
        .iter()
        .filter(|s| s.overlap.is_some_and(|d| d < -0.05))
        .count();
    assert!(
        inside > 3,
        "the body crosses the sill, collider off: {inside} steps"
    );
    let top = trace.iter().map(|s| s.root.y).fold(0.0, f32::max);
    assert!(top >= 0.9, "up on the sill: {top}");
    for (k, s) in trace.iter().enumerate() {
        if let Some((c, t)) = s.clock
            && c == name_id("wave")
        {
            let want = PlayMode::HoldLast.clip_time(f64::from(t), duration) as f32;
            assert_eq!(
                s.animator, want,
                "the pose stands at the clock's time, step {k}"
            );
        }
    }
    let slow: Vec<f32> = (trace.windows(2))
        .filter_map(|w| match (w[0].clock, w[1].clock) {
            (Some((_, a)), Some((_, b))) if a > 0.46 * duration && b < 0.54 * duration => {
                Some(b - a)
            }
            _ => None,
        })
        .collect();
    assert!(
        !slow.is_empty() && slow.iter().all(|d| (d - 0.4 * STEP_SECONDS).abs() < 1e-5),
        "0.4 of the clip's rate on the slowed span: {slow:?}"
    );
    let at = trace[landed as usize].root;
    assert!(
        at.distance(Vec3::from(LANDING)) <= 0.011,
        "lands where it was told, standing a skin over the floor: {at}"
    );
    assert!(
        trace[landed as usize..200]
            .iter()
            .all(|s| s.overlap.is_none_or(|d| d > -1e-3)),
        "free floor after the landing"
    );

    let back = trace[319].root;
    assert!(
        back.z < -3.5 - 0.2,
        "the sill blocks the walk back once the collider is on: {back}"
    );
}

#[test]
fn a_path_ending_above_the_floor_gives_the_body_back_to_gravity() {
    let high = [LANDING[0], 0.25, LANDING[2]];
    let Run { trace, arrived, .. } = run(high);
    let at = trace[arrived as usize].root;
    assert!((at.y - 0.25).abs() < 0.02, "{at}");
    let settled = trace[199].root;
    assert!(
        settled.y < 0.02 && trace[199].grounded,
        "it falls onto the floor once the path ends: {settled}"
    );
}

#[test]
fn between_two_spans_the_root_stays_pinned_where_the_first_left_it() {
    let Run {
        trace,
        arrived,
        duration,
    } = run_spans(LANDING, true);
    let gap: Vec<&Step> = (trace.iter())
        .filter(|s| {
            s.clock.is_some_and(|(c, t)| {
                c == name_id("wave") && t > 0.31 * duration && t < 0.59 * duration
            })
        })
        .collect();
    assert!(gap.len() > 10, "the gap lasts {} steps", gap.len());
    for s in &gap {
        assert!(
            s.root.distance(Vec3::from(HIGH)) < 1e-4,
            "pinned in the air between the spans, not fallen onto the sill: {}",
            s.root
        );
    }
    let at = trace[arrived as usize].root;
    assert!(
        at.distance(Vec3::from(LANDING)) <= 0.011,
        "the second span still lands where it was told: {at}"
    );
}
