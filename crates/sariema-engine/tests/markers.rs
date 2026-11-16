// SPDX-License-Identifier: Apache-2.0

//! A clip's marker reaches the game the step after the one whose clip time
//! passes it, at normal speed, with the clip slowed, and with the world in
//! slow motion.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use sariema_format::script::{ClipMode, Command, Event, name_id};
use sariema_runtime::input::{Actions, RawInput};

use common::{engine, entity, native, send, spawn_hero, wired};

const MARKER_S: f32 = 0.25;

/// Steps the game saw the wave's `tap` marker, and each step's clip time.
fn run(speed: f32, time_scale: f32) -> (Vec<u64>, Vec<f32>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::ClipMarker { marker, .. } = e {
                assert_eq!(*marker, name_id("tap"));
                log.borrow_mut().push(n);
            }
        }
        match n {
            0 => {
                spawn_hero(out, [1.5, 0.0, -0.6], 0.0);
                send(
                    out,
                    Command::TimeScale {
                        scale: time_scale,
                        blend: 0.0,
                    },
                );
            }
            2 => send(
                out,
                Command::PlayClip {
                    entity: entity(0),
                    clip: name_id("wave"),
                    mode: ClipMode::Once,
                    blend: 0.0,
                    speed,
                },
            ),
            _ => {}
        }
    });
    let mut engine = engine(logic, wired(), &[]);
    let mut times = Vec::new();
    let raw = RawInput::default();
    let actions = Actions::for_settings(&engine.state().settings.input);
    while engine.steps() < 120 {
        let before = engine.steps();
        if time_scale == 1.0 {
            engine.step(&actions);
        } else {
            engine.frame(Duration::from_secs_f64(1.0 / 60.0), &raw);
        }
        times.resize(engine.steps() as usize, 0.0);
        if engine.steps() > before {
            let t = engine.state().clip_time(entity(0)).map_or(0.0, |(_, t)| t);
            *times.last_mut().unwrap() = t;
        }
    }
    (seen.take(), times)
}

fn crossing(times: &[f32]) -> u64 {
    times.iter().position(|&t| t >= MARKER_S).unwrap() as u64
}

#[test]
fn a_clip_marker_reaches_the_game_the_step_after_its_clip_time_passes_it() {
    for (speed, scale) in [(1.0, 1.0), (0.5, 1.0), (1.0, 0.25)] {
        let (seen, times) = run(speed, scale);
        let at = crossing(&times);
        assert_eq!(
            seen,
            vec![at + 1],
            "speed {speed}, time scale {scale}: clip times {times:?}"
        );
    }
}
