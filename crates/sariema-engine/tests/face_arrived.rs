// SPDX-License-Identifier: Apache-2.0

//! A walk that arrives stops at its mark, with acceleration, whether `face`
//! comes the step it arrives, the step after, or not at all.

mod common;

use glam::Vec3;
use sariema_format::script::{Command, Event, Kind, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, entity, native, send, wired};
use sariema_engine::{Engine, EngineSettings};

const TARGET: [f32; 3] = [5.4, 0.0, -5.0];
const SPEED: f32 = 1.9;
const STEPS: usize = 400;

/// When `face` is sent.
#[derive(Clone, Copy, Debug)]
enum Turn {
    Never,
    /// At this step, counted from the start.
    At(u64),
    OnArrived,
    After(u64),
}

/// The hero's place at every step from the one it arrives.
fn arrived(turn: Turn, accel_steps: u32) -> Vec<Vec3> {
    let arrived_at = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = arrived_at.clone();
    let logic = native(move |n, events, out| {
        assert!(
            !events.iter().any(|e| matches!(e, Event::Rejected { .. })),
            "{events:?}"
        );
        if events.iter().any(|e| matches!(e, Event::Arrived { .. })) {
            seen.set(Some(n));
        }
        let face = Command::Face {
            entity: entity(0),
            yaw: 0.0,
        };
        let since = seen.get().map(|a| n - a);
        match (n, turn, since) {
            (0, ..) => {
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
            }
            (3, ..) => send(
                out,
                Command::MoveTo {
                    entity: entity(0),
                    target: TARGET,
                    speed: SPEED,
                },
            ),
            (_, Turn::OnArrived, Some(0)) => send(out, face),
            (_, Turn::After(k), Some(s)) if s == k => send(out, face),
            (n, Turn::At(k), _) if n == k => send(out, face),
            _ => {}
        }
    });
    let mut settings = wired();
    settings.physics.controller.accel_steps = accel_steps;
    let sim = Simulation::new(settings, Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets(), EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut trace = Vec::new();
    for _ in 0..STEPS {
        engine.step(&actions);
        trace.push(engine.state().world.node(entity(0)).unwrap().local.position);
    }
    let at = arrived_at
        .get()
        .unwrap_or_else(|| panic!("{turn:?}: the hero never arrived, at {:?}", trace.last()))
        as usize;
    trace.split_off(at - 1)
}

#[test]
fn a_walk_that_arrives_stays_at_its_mark_whenever_face_comes() {
    let mut failed = Vec::new();
    for accel_steps in [0, 6, 30] {
        let seen = arrived(Turn::Never, accel_steps).len() as u64;
        // `face` sent the step before `arrived` is seen reaches the
        // collision module the step the walk arrives.
        let arriving = STEPS as u64 - seen;
        for turn in [
            Turn::Never,
            Turn::At(arriving),
            Turn::OnArrived,
            Turn::After(1),
        ] {
            let trace = arrived(turn, accel_steps);
            let mark = trace[0];
            let drift = (trace.iter())
                .map(|&p| Vec3::new(p.x - mark.x, 0.0, p.z - mark.z).length())
                .fold(0.0, f32::max);
            if drift > 1e-4 {
                failed.push(format!(
                    "accel {accel_steps}, {turn:?}: {drift} m past where it arrived"
                ));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}
