// SPDX-License-Identifier: Apache-2.0

//! A cylinder zone takes the feet within its radius and between its bottom
//! and top, so a circle on a floor and the ramp above it count the same; it
//! shares its id with the sphere zones, which replace it and are replaced.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_format::script::{Command, Event, Kind};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

const ZONE: u32 = 7;
const CENTRE: [f32; 3] = [5.0, 0.75, 0.0];

/// Feet inside the cylinder of radius 1 and height 1.5 round `CENTRE`
/// (its bottom on the floor), then outside it: past the rim, above the
/// top, below the bottom.
const FEET: [[f32; 3]; 4] = [
    [5.9, 0.0, 0.0],
    [6.1, 0.5, 0.0],
    [5.0, 1.6, 0.0],
    [5.0, -0.1, 0.0],
];

/// Each crossing as (step, entered, entity index).
fn crossings(then: impl FnMut(u64, &mut Vec<u8>) + 'static, steps: usize) -> Vec<(u64, bool, u32)> {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let mut then = then;
    let logic = native(move |n, events, out| {
        for e in events {
            match e {
                Event::TriggerEnter { trigger, entity } if *trigger == ZONE => {
                    log.borrow_mut().push((n, true, entity.index))
                }
                Event::TriggerExit { trigger, entity } if *trigger == ZONE => {
                    log.borrow_mut().push((n, false, entity.index))
                }
                Event::Rejected { .. } => panic!("{e:?}"),
                _ => {}
            }
        }
        if n == 0 {
            for (i, at) in FEET.iter().enumerate() {
                send(
                    out,
                    Command::Spawn {
                        entity: entity(i as u32),
                        kind: Kind::Actor,
                        asset: 0,
                    },
                );
                send(
                    out,
                    Command::SetTransform {
                        entity: entity(i as u32),
                        position: *at,
                        yaw: 0.0,
                    },
                );
            }
            send(
                out,
                Command::ZoneCylinder {
                    zone: ZONE,
                    center: CENTRE,
                    radius: 1.0,
                    height: 1.5,
                },
            );
        }
        then(n, out);
    });
    let mut sim = Simulation::new(Settings::default(), Vec::new(), logic, 0);
    let actions = Actions::for_settings(&sim.state().settings.input);
    for _ in 0..steps {
        sim.step(&actions);
    }
    drop(sim);
    seen.take()
}

#[test]
fn a_cylinder_takes_the_feet_within_its_radius_and_height() {
    let walk_out = |n: u64, out: &mut Vec<u8>| {
        if n == 2 {
            send(
                out,
                Command::MoveTo {
                    entity: entity(0),
                    target: [7.0, 0.0, 0.0],
                    speed: 0.9,
                },
            );
        }
    };
    let seen = crossings(walk_out, 90);
    assert_eq!(seen, [(1, true, 0), (9, false, 0)]);
}

#[test]
fn a_sphere_zone_of_the_same_id_replaces_the_cylinder() {
    let to_sphere = |n: u64, out: &mut Vec<u8>| {
        if n == 2 {
            send(
                out,
                Command::Zone {
                    zone: ZONE,
                    center: CENTRE,
                    radius: 1.0,
                },
            );
        }
    };
    let seen = crossings(to_sphere, 6);
    assert_eq!(
        seen,
        [(1, true, 0), (3, true, 2), (3, true, 3), (3, false, 0)],
        "the sphere takes the feet above and below the cylinder and leaves those at its rim"
    );
}
