// SPDX-License-Identifier: Apache-2.0

//! After a walk arrives, nothing drives the actor: a `face` some steps
//! later only turns it, whoever has the hero and with the stick at rest.

mod common;

use glam::{Vec2, Vec3};
use sariema_format::script::{Command, Controller, Event, Kind, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, entity, native, send, wired};
use sariema_engine::{Engine, EngineSettings};

const TARGET: [f32; 3] = [5.4, 0.0, -5.0];
const SPEED: f32 = 1.9;
const WATCH: usize = 120;

/// Who has the hero before and after the walk.
#[derive(Clone, Copy, Debug)]
enum Hands {
    Script,
    /// Handed to the player the step `face` is sent, the stick at rest.
    Player,
    /// Walked by the player along the walk's way until the script takes it
    /// for the walk, the stick let go as it does.
    PlayerFirst,
}

/// The step the script takes the hero and sends the walk.
const WALK: u64 = 30;

/// The hero's place from the step the walk arrives, for `WATCH` steps
/// after `face` is sent `after` steps later.
fn turned(after: u64, hands: Hands) -> Vec<Vec3> {
    let arrived = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = arrived.clone();
    let logic = native(move |n, events, out| {
        assert!(
            !events.iter().any(|e| matches!(e, Event::Rejected { .. })),
            "{events:?}"
        );
        if events.iter().any(|e| matches!(e, Event::Arrived { .. })) {
            seen.set(Some(n));
        }
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
                    Command::SetHero {
                        entity: entity(0),
                        speed: 1.0,
                    },
                );
                let by = match hands {
                    Hands::PlayerFirst => Controller::Player,
                    _ => Controller::Script,
                };
                send(out, Command::Control { by });
            }
            WALK => {
                send(
                    out,
                    Command::Control {
                        by: Controller::Script,
                    },
                );
                send(
                    out,
                    Command::MoveTo {
                        entity: entity(0),
                        target: TARGET,
                        speed: SPEED,
                    },
                );
            }
            _ if seen.get().is_some_and(|a| n == a + after) => {
                send(
                    out,
                    Command::Face {
                        entity: entity(0),
                        yaw: 0.0,
                    },
                );
                if matches!(hands, Hands::Player) {
                    send(
                        out,
                        Command::Control {
                            by: Controller::Player,
                        },
                    );
                }
            }
            _ => {}
        }
    });
    let mut settings = wired();
    settings.physics.controller.accel_steps = 6;
    let sim = Simulation::new(settings, Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets(), EngineSettings::default()).unwrap();
    let mut actions = Actions::for_settings(&engine.state().settings.input);
    let mut trace = Vec::new();
    for n in 0..400 {
        let walking = matches!(hands, Hands::PlayerFirst) && n < WALK;
        actions.axes[0] = if walking { Vec2::X } else { Vec2::ZERO };
        engine.step(&actions);
        trace.push(engine.state().world.node(entity(0)).unwrap().local.position);
    }
    let at = arrived.get().expect("the hero arrives") as usize;
    let end = (at + after as usize + WATCH).min(trace.len());
    trace[at - 1..end].to_vec()
}

#[test]
fn a_face_some_steps_after_arriving_only_turns_the_actor() {
    let mut failed = Vec::new();
    for hands in [Hands::Script, Hands::Player, Hands::PlayerFirst] {
        for after in [1, 2, 5] {
            let trace = turned(after, hands);
            let mark = trace[0];
            let drift = (trace.iter())
                .map(|&p| Vec3::new(p.x - mark.x, 0.0, p.z - mark.z).length())
                .fold(0.0, f32::max);
            if drift > 1e-3 {
                failed.push(format!(
                    "{hands:?}, face {after} steps after arriving: {drift} m off the mark"
                ));
            }
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn a_hero_the_script_takes_from_a_walking_player_comes_to_rest() {
    let logic = native(move |n, _, out| match n {
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
                Command::SetHero {
                    entity: entity(0),
                    speed: 1.0,
                },
            );
            send(
                out,
                Command::Control {
                    by: Controller::Player,
                },
            );
        }
        WALK => send(
            out,
            Command::Control {
                by: Controller::Script,
            },
        ),
        _ => {}
    });
    let mut settings = wired();
    settings.physics.controller.accel_steps = 6;
    let sim = Simulation::new(settings, Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets(), EngineSettings::default()).unwrap();
    let mut actions = Actions::for_settings(&engine.state().settings.input);
    let mut trace = Vec::new();
    for n in 0..WALK + 60 {
        actions.axes[0] = if n < WALK { Vec2::X } else { Vec2::ZERO };
        engine.step(&actions);
        trace.push(engine.state().world.node(entity(0)).unwrap().local.position);
    }
    let walked = trace[WALK as usize - 2].x - trace[0].x;
    assert!(walked > 0.3, "the player walks the hero first: {walked} m");
    let (a, b) = (trace[trace.len() - 2], trace[trace.len() - 1]);
    let still = (b - a).length();
    assert!(
        still < 1e-6,
        "still walking {still} m a step under the script"
    );
}
