// SPDX-License-Identifier: Apache-2.0

//! Each actor the collision module moves turns no faster than its own
//! rate: one at 9.5 rad/s, the other at the 14 rad/s `motion.turn_rate`
//! gives every actor, both turning half round in place.

mod common;

use sariema_format::script::{Command, Event, Kind, name_id};
use sariema_runtime::input::Actions;

use common::{engine, entity, native, send, wired};

const SLOW: f32 = 544.3;
const SETTING: f32 = 802.14;

fn spawn(out: &mut Vec<u8>, index: u32, at: [f32; 3]) {
    send(
        out,
        Command::Spawn {
            entity: entity(index),
            kind: Kind::Actor,
            asset: name_id("hero"),
        },
    );
    send(
        out,
        Command::SetTransform {
            entity: entity(index),
            position: at,
            yaw: 0.0,
        },
    );
}

/// The most each actor turned in one step, in degrees.
fn fastest_turns() -> [f32; 2] {
    let logic = native(|n, events, out| match n {
        _ if events.iter().any(|e| matches!(e, Event::Rejected { .. })) => {
            panic!("{events:?}")
        }
        0 => {
            spawn(out, 0, [3.5, 0.0, -5.0]);
            spawn(out, 1, [4.5, 0.0, -1.0]);
        }
        1 => {
            send(
                out,
                Command::SetTurnRate {
                    entity: entity(0),
                    rate: SLOW,
                },
            );
            for index in [0, 1] {
                send(
                    out,
                    Command::Face {
                        entity: entity(index),
                        yaw: 179.0,
                    },
                );
            }
        }
        _ => {}
    });
    let mut settings = wired();
    settings.motion.turn_rate = SETTING;
    let mut engine = engine(logic, settings, &[]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let yaw =
        |e: &sariema_engine::Engine<_>, i: u32| e.state().world.node(entity(i)).unwrap().local.yaw;
    engine.step(&actions);
    let mut last = [yaw(&engine, 0), yaw(&engine, 1)];
    let mut most = [0.0f32; 2];
    for _ in 0..60 {
        engine.step(&actions);
        for i in 0..2 {
            let now = yaw(&engine, i as u32);
            let turn = ((now - last[i] + 180.0).rem_euclid(360.0) - 180.0).abs();
            most[i] = most[i].max(turn);
            last[i] = now;
        }
    }
    for yaw in last {
        let off = (yaw - 179.0).rem_euclid(360.0);
        assert!(off.min(360.0 - off) < 0.5, "{last:?}");
    }
    most
}

#[test]
fn each_actor_turns_at_its_own_rate_under_collision() {
    let [slow, setting] = fastest_turns();
    assert!((slow - SLOW / 60.0).abs() < 1e-3, "{slow}");
    assert!((setting - SETTING / 60.0).abs() < 1e-3, "{setting}");
}
