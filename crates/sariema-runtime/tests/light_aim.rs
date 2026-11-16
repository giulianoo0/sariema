// SPDX-License-Identifier: Apache-2.0

//! A spot light aimed at a point in the frame of what carries it points
//! ahead of it as it moves and turns, in metres of the world whatever its
//! scale; aimed at the world it stays on that point.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::{Quat, Vec3};
use sariema_format::script::{Command, Entity, Kind};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

const LIGHT: u32 = 1;
const CARRIER: u32 = 2;
const AHEAD: [f32; 3] = [0.0, 1.0, -3.0];

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

fn aim(at: Entity, target: [f32; 3]) -> Command<'static> {
    Command::LightAim {
        entity: entity(LIGHT),
        at,
        target,
    }
}

/// A step: where the light points, and the carrier's position and yaw.
type Step = (Option<Vec3>, Vec3, f32);

fn run(script: fn(u64, &mut Vec<u8>), steps: u64) -> (Vec<Step>, Vec<String>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let log = events.clone();
    let logic = native(move |n, seen, out| {
        log.borrow_mut()
            .extend(seen.iter().map(|e| format!("{n} {e:?}")));
        if n == 0 {
            spawn(out, LIGHT, Kind::Light);
            spawn(out, CARRIER, Kind::Empty);
            send(
                out,
                Command::SetScale {
                    entity: entity(CARRIER),
                    scale: 2.25,
                },
            );
        }
        if (1..60).contains(&n) {
            send(
                out,
                Command::SetTransform {
                    entity: entity(CARRIER),
                    position: [n as f32 * 0.05, 0.0, -1.0],
                    yaw: n as f32 * 3.0,
                },
            );
        }
        script(n, out);
    });
    let settings = Settings::default();
    let mut sim = Simulation::new(settings.clone(), Vec::new(), logic, 1);
    let actions = Actions::for_settings(&settings.input);
    let mut trace = Vec::new();
    for _ in 0..steps {
        sim.step(&actions);
        let s = sim.state();
        let carrier = s.world.node(entity(CARRIER)).map(|n| n.local);
        trace.push((
            s.aims.target(&s.world, entity(LIGHT), 1.0),
            carrier.map_or(Vec3::NAN, |c| c.position),
            carrier.map_or(f32::NAN, |c| c.yaw),
        ));
    }
    let events = events.take();
    (trace, events)
}

#[test]
fn aimed_in_a_carriers_frame_it_points_ahead_as_the_carrier_moves_and_turns() {
    let (trace, _) = run(
        |n, out| {
            if n == 5 {
                send(out, aim(entity(CARRIER), AHEAD))
            }
        },
        60,
    );
    assert!(trace[..5].iter().all(|t| t.0.is_none()), "not aimed yet");
    for (k, (target, at, yaw)) in trace.iter().enumerate().skip(5) {
        let want = *at + Quat::from_rotation_y(yaw.to_radians()) * Vec3::from(AHEAD);
        let got = target.expect("aimed");
        assert!(
            got.distance(want) < 1e-4,
            "step {k}: {got} against {want}, 3 m ahead whatever the scale"
        );
    }
}

#[test]
fn aimed_at_the_world_it_stays_and_a_carrier_gone_forgets_it() {
    let (trace, _) = run(
        |n, out| match n {
            5 => send(out, aim(Entity::NONE, [4.0, 0.0, -2.0])),
            20 => send(out, aim(entity(CARRIER), AHEAD)),
            40 => send(
                out,
                Command::Despawn {
                    entity: entity(CARRIER),
                },
            ),
            _ => {}
        },
        50,
    );
    assert!(
        trace[5..20]
            .iter()
            .all(|t| t.0 == Some(Vec3::new(4.0, 0.0, -2.0)))
    );
    assert!(trace[40..].iter().all(|t| t.0.is_none()));
}

#[test]
fn aiming_what_is_not_a_light_is_refused() {
    let (_, events) = run(
        |n, out| {
            if n == 5 {
                send(
                    out,
                    Command::LightAim {
                        entity: entity(CARRIER),
                        at: Entity::NONE,
                        target: [0.0; 3],
                    },
                )
            }
        },
        8,
    );
    assert!(
        events
            .iter()
            .any(|e| e.starts_with("6 ") && e.contains("E_NOT_LIGHT")),
        "{events:?}"
    );
}
