// SPDX-License-Identifier: Apache-2.0

//! A visible actor never jumps, whatever command would move it at once.

mod common;

use glam::Vec3;
use sariema_format::script::{Command, Ease, Entity, Event, Kind, Property};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

const ACTOR: Entity = Entity {
    index: 0,
    generation: 0,
};
const PROP: Entity = Entity {
    index: 1,
    generation: 0,
};

fn spawn(kind: Kind, e: Entity) -> Command<'static> {
    Command::Spawn {
        entity: e,
        kind,
        asset: 0,
    }
}

fn visible(e: Entity, visible: bool) -> Command<'static> {
    Command::SetVisible { entity: e, visible }
}

fn teleport(e: Entity, x: f32) -> Command<'static> {
    Command::Teleport {
        entity: e,
        position: [x, 0.0, 0.0],
        yaw: 0.0,
        force: false,
    }
}

fn set_transform(e: Entity, x: f32) -> Command<'static> {
    Command::SetTransform {
        entity: e,
        position: [x, 0.0, 0.0],
        yaw: 0.0,
    }
}

const BLACK: Command<'static> = Command::Fade {
    to: 1.0,
    duration: 0.0,
    color: [0.0; 3],
};

struct Run {
    /// The actor's world position after each step.
    at: Vec<Vec3>,
    /// `(step, code)` of every refusal.
    refused: Vec<(u64, String)>,
}

/// Runs the commands of `script[step]` at each step, the actor and the
/// prop spawned at step 0 at the origin.
fn run(script: Vec<Vec<Command<'static>>>) -> Run {
    let steps = script.len() as u64 + 1;
    let mut refused = Vec::new();
    let logic = native(|step, events, out| {
        for e in events {
            if let Event::Rejected { code, .. } = e {
                refused.push((step - 1, code.to_string()));
            }
        }
        if step == 0 {
            send(out, spawn(Kind::Actor, ACTOR));
            send(out, spawn(Kind::Empty, PROP));
        }
        for c in script.get(step as usize).into_iter().flatten() {
            send(out, *c);
        }
    });
    let settings = Settings::default();
    let idle = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    let mut at = Vec::new();
    for _ in 0..steps {
        sim.step(&idle);
        at.push(sim.state().world.current_pose(ACTOR).unwrap().position);
    }
    drop(sim);
    Run { at, refused }
}

fn refused_at(step: u64) -> Vec<(u64, String)> {
    vec![(step, "E_TELEPORT_VISIBLE".to_string())]
}

#[test]
fn hiding_and_showing_in_the_same_step_does_not_hide_a_teleport() {
    let r = run(vec![
        vec![],
        vec![
            visible(ACTOR, false),
            teleport(ACTOR, 5.0),
            visible(ACTOR, true),
        ],
    ]);
    assert_eq!(r.refused, refused_at(1));
    assert_eq!(r.at[1].x, 0.0);

    let r = run(vec![
        vec![],
        vec![visible(ACTOR, false)],
        vec![teleport(ACTOR, 5.0), visible(ACTOR, true)],
    ]);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert_eq!(r.at[2].x, 5.0, "hidden for a whole step, it may move");
}

#[test]
fn a_one_step_tween_of_a_visible_actor_is_a_jump() {
    let tween = |duration| Command::Tween {
        tween: 1,
        entity: ACTOR,
        property: Property::PositionX,
        var: 0,
        to: 5.0,
        duration,
        ease: Ease::Linear,
    };
    let r = run(vec![vec![], vec![tween(0.0)]]);
    assert_eq!(r.refused, refused_at(1));
    assert_eq!(r.at[2].x, 0.0);

    let r = run(vec![vec![], vec![tween(0.5)]]);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert!(r.at[2].x > 0.0 && r.at[2].x < 1.0, "eased: {}", r.at[2].x);

    let r = run(vec![vec![], vec![BLACK], vec![tween(0.0)]]);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert_eq!(r.at[2].x, 5.0, "allowed under a full fade");
}

#[test]
fn a_rigid_follow_that_would_move_a_visible_actor_is_a_jump() {
    let follow = |lag| Command::Follow {
        entity: ACTOR,
        target: PROP,
        offset: [0.0; 3],
        lag,
    };
    let r = run(vec![
        vec![],
        vec![set_transform(PROP, 5.0)],
        vec![follow(0.0)],
    ]);
    assert_eq!(r.refused, refused_at(2));
    assert_eq!(r.at[3].x, 0.0);

    let r = run(vec![
        vec![],
        vec![set_transform(PROP, 5.0)],
        vec![follow(0.5)],
    ]);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert!(
        r.at[3].x > 0.0 && r.at[3].x < 1.0,
        "closes the gap: {}",
        r.at[3].x
    );

    let r = run(vec![
        vec![],
        vec![follow(0.0)],
        vec![set_transform(PROP, 5.0)],
    ]);
    assert_eq!(
        r.refused,
        refused_at(2),
        "placing what a visible actor rigidly follows moves it at once"
    );
    assert_eq!(r.at[3].x, 0.0);
}

#[test]
fn reparenting_that_would_move_a_visible_actor_is_a_jump() {
    let parent = Command::SetParent {
        entity: ACTOR,
        parent: PROP,
    };
    let r = run(vec![vec![], vec![set_transform(PROP, 5.0)], vec![parent]]);
    assert_eq!(r.refused, refused_at(2));
    assert_eq!(r.at[3].x, 0.0);
    assert!(r.at.iter().all(|p| p.x == 0.0));

    let r = run(vec![vec![], vec![parent]]);
    assert!(r.refused.is_empty(), "a parent at the origin moves nothing");

    let r = run(vec![
        vec![],
        vec![set_transform(PROP, 5.0)],
        vec![BLACK],
        vec![parent],
    ]);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert_eq!(r.at[3].x, 5.0);
}

#[test]
fn placing_a_parent_that_carries_a_visible_actor_is_a_jump() {
    let parent = Command::SetParent {
        entity: ACTOR,
        parent: PROP,
    };
    let r = run(vec![vec![], vec![parent], vec![set_transform(PROP, 5.0)]]);
    assert_eq!(r.refused, refused_at(2));
    assert_eq!(r.at[3].x, 0.0);

    let r = run(vec![
        vec![],
        vec![parent, visible(ACTOR, false)],
        vec![set_transform(PROP, 5.0)],
    ]);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert_eq!(r.at[3].x, 5.0, "a hidden child may be carried");

    let deep = entity(2);
    let r = run(vec![
        vec![spawn(Kind::Empty, deep)],
        vec![
            Command::SetParent {
                entity: PROP,
                parent: deep,
            },
            parent,
        ],
        vec![set_transform(deep, 5.0)],
    ]);
    assert_eq!(r.refused, refused_at(2), "a grandchild is carried too");
}
