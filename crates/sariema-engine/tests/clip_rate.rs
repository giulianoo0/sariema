// SPDX-License-Identifier: Apache-2.0

//! A clip's rate changed while it plays: the clip goes on from where it is
//! at the new rate, never started over, the animator standing at the
//! runtime's clip time and each marker reaching the game once as the clip
//! passes it.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_anim::PlayMode;
use sariema_core::STEP_SECONDS;
use sariema_format::script::{ClipMode, Command, Event, name_id};
use sariema_runtime::input::Actions;

use common::{engine, entity, native, send, spawn_hero, wired};

const MARKER_S: f32 = 0.25;

fn rate(clip: &str, rate: f32) -> Command<'static> {
    Command::ClipRate {
        entity: entity(0),
        clip: name_id(clip),
        rate,
    }
}

struct Run {
    /// Each step's clip time, as the runtime and as the animator count it.
    times: Vec<(f32, f32)>,
    duration: f32,
    markers: Vec<u64>,
    events: Vec<(u64, String)>,
}

fn run(script: fn(u64, &mut Vec<u8>), steps: u64) -> Run {
    let markers = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let (m, ev) = (markers.clone(), events.clone());
    let logic = native(move |n, seen, out| {
        for e in seen {
            if let Event::ClipMarker { .. } = e {
                m.borrow_mut().push(n);
            }
            ev.borrow_mut().push((n, format!("{e:?}")));
        }
        match n {
            0 => spawn_hero(out, [1.5, 0.0, -0.6], 0.0),
            2 => send(
                out,
                Command::PlayClip {
                    entity: entity(0),
                    clip: name_id("wave"),
                    mode: ClipMode::Loop,
                    blend: 0.2,
                    speed: 1.0,
                },
            ),
            _ => script(n, out),
        }
    });
    let mut engine = engine(logic, wired(), &[]);
    let model = &engine.animation().models[0];
    let clip = model.rig.clip("wave").expect("the hero has a wave");
    let duration = model.rig.clip_info(clip).duration;
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut times = Vec::new();
    for _ in 0..steps {
        engine.step(&actions);
        let t = engine.state().clip_time(entity(0)).map_or(-1.0, |c| c.1);
        let (_, _, at) = engine.animation().clip_times().next().unwrap();
        times.push((t, at));
    }
    Run {
        times,
        duration,
        markers: markers.take(),
        events: events.take(),
    }
}

#[test]
fn a_new_rate_goes_on_from_where_the_clip_is_with_no_restart() {
    let r = run(
        |n, out| match n {
            60 => send(out, rate("wave", 1.6)),
            140 => send(out, rate("wave", 0.5)),
            _ => {}
        },
        300,
    );
    for (k, w) in r.times.windows(2).enumerate().skip(3) {
        let step = k + 1;
        let want = match step {
            ..=60 => 1.0,
            61..=140 => 1.6,
            _ => 0.5,
        };
        let went = w[1].0 - w[0].0;
        assert!(
            (went - want * STEP_SECONDS).abs() < 1e-5,
            "step {step}: the clip went {went} s, {want} of a step"
        );
    }
    for (k, (t, at)) in r.times.iter().enumerate().skip(3) {
        let want = PlayMode::Loop.clip_time(f64::from(*t), r.duration) as f32;
        let off = (at - want).abs();
        assert!(
            off.min(r.duration - off) < 1e-5,
            "the animator stands at the clip's time, step {k}: {at} against {want}"
        );
    }
    let passed = (r.times.windows(2))
        .filter(|w| {
            let loops = |t: f32| ((t - MARKER_S) / r.duration).floor();
            w[0].0 >= 0.0 && loops(w[1].0) > loops(w[0].0)
        })
        .count();
    assert!(
        passed >= 3,
        "the test passes the marker a few times: {passed}"
    );
    assert_eq!(
        r.markers.len(),
        passed,
        "one marker each time the clip passes it: {:?}",
        r.markers
    );
}

#[test]
fn a_rate_for_a_clip_not_playing_or_below_zero_is_refused() {
    let r = run(
        |n, out| match n {
            1 => send(out, rate("wave", 2.0)),
            30 => send(out, rate("idle", 2.0)),
            40 => send(out, rate("wave", -1.0)),
            60 => send(out, rate("wave", 0.0)),
            _ => {}
        },
        80,
    );
    let refused = |code: &str| -> Vec<u64> {
        (r.events.iter())
            .filter(|(_, e)| e.contains(code))
            .map(|(n, _)| *n)
            .collect()
    };
    assert_eq!(refused("E_CLIP_NOT_PLAYING"), [2, 31]);
    assert_eq!(refused("E_CLIP_RATE"), [41]);
    let still = r.times[61].0;
    assert!(
        r.times[61..].iter().all(|t| t.0 == still),
        "a rate of 0 holds the clip where it is"
    );
    assert!(r.times[59].0 > 0.9, "the refused rates left it at 1");
}
