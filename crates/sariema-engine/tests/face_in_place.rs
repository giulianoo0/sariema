// SPDX-License-Identifier: Apache-2.0

//! `face` turns an actor the collision module moves in place, at its turn
//! rate: walking with acceleration, then stopped or arrived, it turns where
//! it stands and does not spend the speed it had left.

mod common;

use glam::Vec3;
use sariema_format::script::{Command, Event, Kind, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, entity, native, send, wired};
use sariema_engine::{Engine, EngineSettings};

const RATE: f32 = 180.0;

/// How the walk ends before the turn.
#[derive(Clone, Copy, Debug)]
enum Ending {
    /// `stop` at step 40, mid-walk, then `face` the step after.
    Stopped,
    /// `face` the step `arrived` comes.
    Arrived,
}

/// The hero's place and yaw from the step before the one `face` is sent at.
fn turned(ending: Ending) -> Vec<(Vec3, f32)> {
    let faced = std::rc::Rc::new(std::cell::Cell::new(None));
    let at = faced.clone();
    let logic = native(move |n, events, out| {
        if events.iter().any(|e| matches!(e, Event::Rejected { .. })) {
            panic!("{events:?}");
        }
        let face = |out: &mut Vec<u8>| {
            send(
                out,
                Command::Face {
                    entity: entity(0),
                    yaw: 90.0,
                },
            );
            at.set(Some(n));
        };
        match n {
            0 => {
                send(
                    out,
                    Command::Spawn {
                        entity: entity(0),
                        kind: Kind::Actor,
                        asset: name_id("hero"),
                    },
                );
                send(
                    out,
                    Command::SetTransform {
                        entity: entity(0),
                        position: [0.6, 0.0, -5.0],
                        yaw: 0.0,
                    },
                );
                send(
                    out,
                    Command::SetTurnRate {
                        entity: entity(0),
                        rate: RATE,
                    },
                );
            }
            3 => send(
                out,
                Command::MoveTo {
                    entity: entity(0),
                    target: [5.4, 0.0, -5.0],
                    speed: 1.5,
                },
            ),
            40 if matches!(ending, Ending::Stopped) => {
                send(out, Command::Stop { entity: entity(0) })
            }
            41 if matches!(ending, Ending::Stopped) => face(out),
            _ if matches!(ending, Ending::Arrived)
                && events.iter().any(|e| matches!(e, Event::Arrived { .. })) =>
            {
                face(out)
            }
            _ => {}
        }
    });
    let mut settings = wired();
    settings.physics.controller.accel_steps = 30;
    let sim = Simulation::new(settings, Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets(), EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut trace = Vec::new();
    for _ in 0..400 {
        engine.step(&actions);
        let node = engine.state().world.node(entity(0)).unwrap();
        trace.push((node.local.position, node.local.yaw));
    }
    let faced = faced.get().unwrap_or_else(|| {
        panic!(
            "{ending:?}: the hero was told to face, at {:?}",
            trace.last()
        )
    }) as usize;
    trace.split_off(faced - 1)
}

#[test]
fn a_face_after_a_walk_turns_in_place_at_the_turn_rate() {
    for ending in [Ending::Stopped, Ending::Arrived] {
        let trace = turned(ending);
        let (start, first) = trace[0];
        let slid = (trace.iter())
            .map(|(p, _)| Vec3::new(p.x - start.x, 0.0, p.z - start.z).length())
            .fold(0.0, f32::max);
        assert!(slid < 1e-3, "{ending:?}: it slid {slid} m while turning");
        let last = trace[trace.len() - 1].1;
        let to = |yaw: f32| ((yaw - 90.0 + 180.0).rem_euclid(360.0) - 180.0).abs();
        assert!(to(last) < 0.5, "{ending:?}: it turned to {last}, not 90");
        assert!(
            to(first) > 30.0,
            "{ending:?}: it turned over time, not at once: {first}"
        );
    }
}
