// SPDX-License-Identifier: Apache-2.0

//! A cutscene's walk the game interrupts (a `stop`, a teleport, a
//! `move_intent`) ends, played or skipped, whether the runtime's own mover
//! or the collision module moves the actor, so the cutscene never waits
//! forever.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_engine::{Engine, EngineSettings};
use sariema_format::script::{Command, Event, name_id};
use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send, spawn_hero, wired};

const START: [f32; 3] = [0.75, 0.0, -0.4];

const WALK: &str = r#"{ "name": "walk", "roles": { "hero": { "start": { "position": [0.75, 0, -0.4], "yaw": 0 } } },
    "steps": [ { "walk": { "role": "hero", "to": [4.5, 0, -1.0], "speed": 1.2 } }, { "call": "after" } ] }"#;

#[derive(Clone, Copy, Debug)]
enum Interrupt {
    Stop,
    Teleport,
    MoveIntent,
    /// Into the sill, which the collision module refuses.
    RefusedTeleport,
}

#[derive(Clone, Copy, Debug)]
enum Mover {
    Builtin,
    Collision,
}

const INTERRUPTED_AT: u64 = 40;

fn interrupt(out: &mut Vec<u8>, how: Interrupt) {
    let hero = entity(0);
    match how {
        Interrupt::Stop => send(out, Command::Stop { entity: hero }),
        Interrupt::Teleport => send(
            out,
            Command::Teleport {
                entity: hero,
                position: [1.5, 0.0, -0.5],
                yaw: 0.0,
                force: true,
            },
        ),
        Interrupt::MoveIntent => send(
            out,
            Command::MoveIntent {
                entity: hero,
                velocity: [0.0, 0.0, 0.5],
            },
        ),
        Interrupt::RefusedTeleport => send(
            out,
            Command::Teleport {
                entity: hero,
                position: [3.5, 0.3, -3.0],
                yaw: 0.0,
                force: true,
            },
        ),
    }
}

/// The step the cutscene ended at, and whether it was skipped, when the
/// game interrupts the walk `how` and skips the cutscene at step `skip`.
fn ended(how: Option<Interrupt>, skip: Option<u64>, mover: Mover) -> Option<(u64, bool)> {
    let seen = Rc::new(RefCell::new(None));
    let log = seen.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::CutsceneEnded { skipped, .. } = e {
                log.borrow_mut().get_or_insert((n, *skipped));
            }
        }
        if n == INTERRUPTED_AT
            && let Some(how) = how
        {
            interrupt(out, how);
        }
        if Some(n) == skip {
            send(out, Command::CutsceneSkip {});
        }
        match n {
            0 => spawn_hero(out, START, 0.0),
            1 => {
                send(
                    out,
                    Command::CutsceneBind {
                        role: name_id("hero"),
                        entity: entity(0),
                    },
                );
                send(
                    out,
                    Command::CutscenePlay {
                        cutscene: name_id("walk"),
                    },
                );
            }
            _ => {}
        }
    });
    let cutscenes = vec![compile_json(WALK).unwrap().0];
    let mut assets = common::assets();
    let settings = match mover {
        Mover::Builtin => {
            assets.collision = None;
            Settings::default()
        }
        Mover::Collision => wired(),
    };
    let sim = Simulation::new(settings, cutscenes, logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..600 {
        engine.step(&actions);
    }
    let playing = engine.simulation().playing_cutscene();
    drop(engine);
    let ended = *seen.borrow();
    assert_eq!(playing.is_none(), ended.is_some());
    ended
}

#[test]
fn an_interrupted_walk_ends_its_cutscene() {
    let skips = [None, Some(INTERRUPTED_AT), Some(INTERRUPTED_AT - 5)];
    let mut wrong = Vec::new();
    for mover in [Mover::Builtin, Mover::Collision] {
        for how in [Interrupt::Stop, Interrupt::Teleport, Interrupt::MoveIntent] {
            for skip in skips {
                let at = ended(Some(how), skip, mover);
                let ends = at.is_some_and(|(step, skipped)| {
                    skipped == skip.is_some() && step < INTERRUPTED_AT + 60
                });
                if !ends {
                    wrong.push(format!("{how:?}, {mover:?}, skipped at {skip:?}: {at:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn a_walk_left_alone_ends_on_arrival() {
    for mover in [Mover::Builtin, Mover::Collision] {
        let (step, skipped) = ended(None, None, mover).expect("the walk ends");
        assert!(!skipped);
        assert!(
            step > INTERRUPTED_AT + 60,
            "{mover:?}: the walk takes its time: {step}"
        );
    }
}

#[test]
fn a_teleport_the_collision_module_refuses_leaves_the_walk_going() {
    let alone = ended(None, None, Mover::Collision).expect("the walk ends");
    let refused = ended(Some(Interrupt::RefusedTeleport), None, Mover::Collision);
    assert_eq!(refused, Some(alone));
}

/// Reachable, clear of the sill.
const SPOT: [f32; 3] = [3.5, 0.0, -2.0];

const AT_SPOT: &str = r#"{ "name": "spot", "roles": { "hero": { "start": { "position": [3.5, 0, -2.0], "yaw": 0 } } },
    "steps": [ { "call": "placed" } ] }"#;

/// Whether `at` stands inside the sill, which spans x 3 to 4, z −3.5 to
/// −2.5 and rises 0.9.
fn in_sill(at: glam::Vec3) -> bool {
    (3.0..=4.0).contains(&at.x) && (-3.5..=-2.5).contains(&at.z) && at.y < 0.9
}

/// A game that teleports a role and plays its cutscene in the same tick has
/// the role judged from where the collision module put it, not from where
/// it was sent: here the teleport lands inside the sill and is refused, so
/// the role walks to its start from where it still stands instead of
/// easing there from inside the sill.
#[test]
fn a_role_teleported_as_its_cutscene_starts_is_placed_from_where_it_landed() {
    let logic = native(move |n, _, out| match n {
        0 => spawn_hero(out, START, 0.0),
        3 => {
            send(
                out,
                Command::Teleport {
                    entity: entity(0),
                    position: [3.5, 0.3, -3.0],
                    yaw: 0.0,
                    force: true,
                },
            );
            send(
                out,
                Command::CutsceneBind {
                    role: name_id("hero"),
                    entity: entity(0),
                },
            );
            send(
                out,
                Command::CutscenePlay {
                    cutscene: name_id("spot"),
                },
            );
        }
        _ => {}
    });
    let mut settings = wired();
    settings.cutscene.start_blend_max = 1.1;
    let sim = Simulation::new(settings, vec![compile_json(AT_SPOT).unwrap().0], logic, 1);
    let mut engine = Engine::new(sim, common::assets(), EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let hero = |engine: &Engine<_>| engine.state().world.node(entity(0)).unwrap().local.position;
    for _ in 0..5 {
        engine.step(&actions);
    }
    assert!(engine.simulation().playing_cutscene().is_some());
    while engine.simulation().playing_cutscene().is_some() && engine.steps() < 600 {
        engine.step(&actions);
        let at = hero(&engine);
        assert!(
            !in_sill(at),
            "step {}: the hero stands in the sill at {at}",
            engine.steps()
        );
    }
    let placed = hero(&engine);
    let spot = glam::Vec3::from_array(SPOT);
    assert!(
        placed.distance(spot) < 0.05,
        "placed at {placed}, not {spot}"
    );
}
