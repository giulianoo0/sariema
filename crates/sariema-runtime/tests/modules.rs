// SPDX-License-Identifier: Apache-2.0

//! What other modules get from a step: the world after the game, the
//! sequencer and the hero's control, before the no-jump measurement judges
//! what they did; and the camera's anchor points.

mod common;

use glam::Vec3;
use sariema_format::script::{Anchor, Command, Event, Kind, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::outbox::{PhysicsIntent, UiOp};
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Modules, Simulation};
use sariema_runtime::state::State;

use common::{entity, native, send};

/// Moves the actor by `step` metres along +X each step, as a collision
/// module would, and records the intents it was given.
struct Mover {
    step: f32,
    intents: Vec<PhysicsIntent>,
}

impl Modules for Mover {
    fn step(&mut self, s: &mut State) {
        self.intents.append(&mut s.outbox.physics);
        if let Ok(node) = s.world.node_mut(entity(0)) {
            node.local.position.x += self.step;
        }
    }
}

fn spawned(out: &mut Vec<u8>) {
    send(
        out,
        Command::Spawn {
            entity: entity(0),
            kind: Kind::Actor,
            asset: 0,
        },
    );
}

fn no_builtin() -> Settings {
    let mut settings = Settings::default();
    settings.motion.builtin = false;
    settings
}

fn rejected(events: &[(u64, String)]) -> Vec<u64> {
    events
        .iter()
        .filter(|(_, e)| e.contains("E_TELEPORT_VISIBLE"))
        .map(|(s, _)| *s)
        .collect()
}

fn run(step: f32, steps: u64) -> (Vec<(u64, String)>, Vec3) {
    let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = heard.clone();
    let logic = native(move |n, events, out| {
        log.borrow_mut()
            .extend(events.iter().map(|e| (n, format!("{e:?}"))));
        if n == 0 {
            spawned(out);
        }
    });
    let settings = no_builtin();
    let actions = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    let mut mover = Mover {
        step,
        intents: Vec::new(),
    };
    for _ in 0..steps {
        sim.step_with(&actions, &mut mover);
    }
    let at = sim.state().world.current_pose(entity(0)).unwrap().position;
    drop(sim);
    (heard.take(), at)
}

#[test]
fn the_measurement_judges_what_a_module_did_in_the_same_step() {
    let (events, at) = run(0.1, 30);
    assert!(rejected(&events).is_empty(), "{events:?}");
    assert!((at.x - 3.0).abs() < 1e-4, "walked to {at}");

    let (events, at) = run(5.0, 3);
    assert!(!rejected(&events).is_empty(), "a 5 m step went unjudged");
    assert_eq!(at.x, 5.0, "held where the step it appeared on left it");
}

#[test]
fn a_rig_aims_at_the_point_the_animation_module_gives_for_its_anchor() {
    struct Hand;
    impl Modules for Hand {
        fn step(&mut self, s: &mut State) {
            let anchors: Vec<_> = s.camera.anchors().collect();
            for (e, anchor, name) in anchors {
                assert_eq!((anchor, name), (Anchor::Socket, name_id("hand")));
                s.camera.set_anchor_point(e, Vec3::new(0.0, 1.5, 0.0));
            }
        }
    }
    let logic = native(|n, _, out| {
        if n == 0 {
            spawned(out);
            send(
                out,
                Command::RigFollow {
                    rig: 1,
                    subject: entity(0),
                    distance: 4.0,
                    pitch: 0.0,
                    yaw: 0.0,
                    height: 0.0,
                    fov: 40.0,
                },
            );
            send(
                out,
                Command::RigAnchor {
                    rig: 1,
                    subject: entity(0),
                    anchor: Anchor::Socket,
                    name: name_id("hand"),
                },
            );
            send(
                out,
                Command::CameraUse {
                    rig: 1,
                    blend: 0.0,
                    ease: sariema_format::script::Ease::Linear,
                },
            );
        }
    });
    let settings = Settings::default();
    let actions = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    for _ in 0..5 {
        sim.step_with(&actions, &mut Hand);
    }
    let pose = sim.state().camera.pose().unwrap();
    assert!(
        (pose.position.y - 1.5).abs() < 1e-3,
        "the camera aims at the hand, 1.5 m up: {}",
        pose.position
    );
}

#[test]
fn a_traversal_without_a_collision_module_ends_at_once_with_blocked() {
    let blocked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = blocked.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::Blocked { entity } = e {
                seen.borrow_mut().push((n, *entity));
            }
        }
        match n {
            0 => spawned(out),
            1 => send(
                out,
                Command::Traverse {
                    entity: entity(0),
                    traversal: name_id("window"),
                    duration: 1.0,
                },
            ),
            _ => {}
        }
    });
    let settings = Settings::default();
    let actions = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    for _ in 0..4 {
        sim.step(&actions);
    }
    drop(sim);
    assert_eq!(*blocked.borrow(), vec![(2, entity(0))]);
}

#[test]
fn a_collision_module_is_asked_to_traverse_face_and_play_a_hud_animation() {
    let logic = native(|n, _, out| match n {
        0 => spawned(out),
        1 => {
            send(
                out,
                Command::Traverse {
                    entity: entity(0),
                    traversal: name_id("window"),
                    duration: 1.0,
                },
            );
            send(
                out,
                Command::Face {
                    entity: entity(0),
                    yaw: 90.0,
                },
            );
            send(
                out,
                Command::UiPlay {
                    animation: name_id("objective_in"),
                },
            );
        }
        _ => {}
    });
    let settings = no_builtin();
    let actions = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    let mut mover = Mover {
        step: 0.0,
        intents: Vec::new(),
    };
    sim.step_with(&actions, &mut mover);
    sim.step_with(&actions, &mut mover);
    assert_eq!(
        sim.state().outbox.ui,
        vec![UiOp::Play {
            animation: name_id("objective_in")
        }]
    );
    assert!(mover.intents.contains(&PhysicsIntent::Traverse {
        entity: entity(0),
        traversal: name_id("window"),
        duration: 1.0,
    }));
    assert!(mover.intents.contains(&PhysicsIntent::Face {
        entity: entity(0),
        yaw: 90.0,
    }));
}
