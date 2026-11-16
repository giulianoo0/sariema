// SPDX-License-Identifier: Apache-2.0

//! Each actor turns at its own rate, at most `motion.turn_rate`; 0 gives
//! it the setting back.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_format::script::{Command, Event, Kind};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

fn spawn(out: &mut Vec<u8>, index: u32, kind: Kind) {
    send(
        out,
        Command::Spawn {
            entity: entity(index),
            kind,
            asset: 0,
        },
    );
}

fn face(out: &mut Vec<u8>, index: u32, yaw: f32) {
    send(
        out,
        Command::Face {
            entity: entity(index),
            yaw,
        },
    );
}

fn rate(out: &mut Vec<u8>, index: u32, rate: f32) {
    send(
        out,
        Command::SetTurnRate {
            entity: entity(index),
            rate,
        },
    );
}

/// Steps `steps` times with `logic`'s commands; the yaws of actors 0 and 1
/// at every step, and every `rejected` event's code and entity index.
fn run(
    logic: impl FnMut(u64, &[Event<'_>], &mut Vec<u8>) + 'static,
    steps: usize,
) -> (Vec<[f32; 2]>, Vec<(String, u32)>) {
    let rejected = Rc::new(RefCell::new(Vec::new()));
    let log = rejected.clone();
    let mut logic = logic;
    let game = native(move |n, events, out| {
        for e in events {
            if let Event::Rejected { code, entity, .. } = e {
                log.borrow_mut().push((code.to_string(), entity.index));
            }
        }
        logic(n, events, out);
    });
    let mut sim = Simulation::new(Settings::default(), Vec::new(), game, 0);
    let actions = Actions::for_settings(&sim.state().settings.input);
    let mut yaws = Vec::new();
    for _ in 0..steps {
        sim.step(&actions);
        let yaw = |i: u32| {
            sim.state()
                .world
                .node(entity(i))
                .map_or(0.0, |n| n.local.yaw)
        };
        yaws.push([yaw(0), yaw(1)]);
    }
    let rejected = rejected.take();
    (yaws, rejected)
}

#[test]
fn an_actor_turns_at_its_own_rate_and_others_at_the_setting() {
    let (yaws, rejected) = run(
        |n, _, out| match n {
            0 => {
                spawn(out, 0, Kind::Actor);
                spawn(out, 1, Kind::Actor);
            }
            1 => {
                rate(out, 1, 90.0);
                face(out, 0, 180.0);
                face(out, 1, 180.0);
            }
            _ => {}
        },
        32,
    );
    assert_eq!(rejected, []);
    let [own, setting] = [1, 0].map(|k| yaws[30][k]);
    assert!((own - 45.0).abs() < 1e-3, "{own}");
    assert_eq!(setting, 180.0);
    for w in yaws.windows(2) {
        assert!((w[1][1] - w[0][1]).abs() <= 1.5 + 1e-4, "{w:?}");
    }
}

#[test]
fn a_rate_of_zero_gives_the_actor_the_setting_back() {
    let (yaws, _) = run(
        |n, _, out| match n {
            0 => spawn(out, 1, Kind::Actor),
            1 => rate(out, 1, 90.0),
            2 => {
                rate(out, 1, 0.0);
                face(out, 1, 180.0);
            }
            _ => {}
        },
        24,
    );
    assert_eq!(yaws[21][1], 180.0);
}

#[test]
fn a_rate_past_the_setting_or_below_zero_or_for_a_prop_is_refused() {
    let (yaws, rejected) = run(
        |n, _, out| match n {
            0 => {
                spawn(out, 0, Kind::Empty);
                spawn(out, 1, Kind::Actor);
            }
            1 => {
                rate(out, 1, 90.0);
                rate(out, 1, 600.0);
                rate(out, 1, -1.0);
                rate(out, 0, 90.0);
                face(out, 1, 180.0);
            }
            _ => {}
        },
        12,
    );
    assert_eq!(
        rejected,
        [
            ("E_TURN_RATE".to_string(), 1),
            ("E_TURN_RATE".to_string(), 1),
            ("E_NOT_ACTOR".to_string(), 0),
        ]
    );
    assert!((yaws[10][1] - 15.0).abs() < 1e-3, "{:?}", yaws[10]);
}
