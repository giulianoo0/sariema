// SPDX-License-Identifier: Apache-2.0

//! `locate` answers at the next step with where an entity stands in the
//! world and its yaw, as the step it was asked in left them: on its own,
//! carried by another, and the same on every run.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_format::script::{Command, Event, Kind};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

const ASKED: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Answer {
    tick: u64,
    entity: u32,
    position: Vec3,
    yaw: f32,
}

/// After a step: the walker's world position and yaw, the rider's world
/// position.
type Placed = (Vec3, f32, Vec3);

/// A walker going +x and a rider on a cart turned a quarter round, asked
/// for at step `ASKED`; the answers, the rejections and where they stood
/// after each step.
fn run() -> (Vec<Answer>, Vec<String>, Vec<Placed>) {
    let answers = Rc::new(RefCell::new((Vec::new(), Vec::new())));
    let log = answers.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            match *e {
                Event::Located {
                    entity,
                    position,
                    yaw,
                } => log.borrow_mut().0.push(Answer {
                    tick: n,
                    entity: entity.index,
                    position: Vec3::from(position),
                    yaw,
                }),
                Event::Rejected { code, entity, .. } => {
                    log.borrow_mut().1.push(format!("{code} {}", entity.index))
                }
                _ => {}
            }
        }
        match n {
            0 => {
                for (index, kind) in [(0, Kind::Actor), (1, Kind::Empty), (2, Kind::Actor)] {
                    send(
                        out,
                        Command::Spawn {
                            entity: entity(index),
                            kind,
                            asset: 0,
                        },
                    );
                }
                send(
                    out,
                    Command::SetTransform {
                        entity: entity(1),
                        position: [4.0, 0.5, -2.0],
                        yaw: 90.0,
                    },
                );
                send(
                    out,
                    Command::SetParent {
                        entity: entity(2),
                        parent: entity(1),
                    },
                );
                send(
                    out,
                    Command::SetTransform {
                        entity: entity(2),
                        position: [0.0, 0.0, -1.0],
                        yaw: 30.0,
                    },
                );
                send(
                    out,
                    Command::MoveTo {
                        entity: entity(0),
                        target: [9.0, 0.0, 0.0],
                        speed: 1.3,
                    },
                );
            }
            ASKED => {
                for index in [0, 2] {
                    send(
                        out,
                        Command::Locate {
                            entity: entity(index),
                        },
                    );
                }
                send(
                    out,
                    Command::Locate {
                        entity: sariema_format::script::Entity {
                            index: 5,
                            generation: 0,
                        },
                    },
                );
            }
            _ => {}
        }
    });
    let mut sim = Simulation::new(Settings::default(), Vec::new(), logic, 0);
    let actions = Actions::for_settings(&sim.state().settings.input);
    let mut placed = Vec::new();
    for _ in 0..=ASKED + 2 {
        sim.step(&actions);
        let world = |i: u32| sim.state().world.pose(entity(i)).unwrap().position;
        let yaw = sim.state().world.node(entity(0)).unwrap().local.yaw;
        placed.push((world(0), yaw, world(2)));
    }
    drop(sim);
    let (answers, rejected) = answers.take();
    (answers, rejected, placed)
}

#[test]
fn locate_answers_at_the_next_step_with_the_world_place_and_yaw() {
    let (answers, rejected, placed) = run();
    assert_eq!(rejected, ["E_ENTITY_STALE 5"]);
    let (walker, walker_yaw, rider) = placed[ASKED as usize];
    assert_eq!(
        answers,
        [
            Answer {
                tick: ASKED + 1,
                entity: 0,
                position: walker,
                yaw: walker_yaw,
            },
            Answer {
                tick: ASKED + 1,
                entity: 2,
                position: rider,
                yaw: answers[1].yaw,
            },
        ]
    );
    assert!((walker_yaw + 90.0).abs() < 1e-3, "{walker_yaw}");
    assert!(walker.x > 0.1 && walker.x < placed[ASKED as usize + 1].0.x);
    assert!(rider.distance(Vec3::new(3.0, 0.5, -2.0)) < 1e-5, "{rider}");
    assert!((answers[1].yaw - 120.0).abs() < 1e-3, "{}", answers[1].yaw);
    assert_eq!(run().0, answers);
}
