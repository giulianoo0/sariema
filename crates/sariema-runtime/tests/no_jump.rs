// SPDX-License-Identifier: Apache-2.0

//! A visible actor never jumps, whatever command would move it at once.

mod common;

use glam::Vec3;
use sariema_core::STEP_SECONDS;
use sariema_format::script::{Command, Ease, Entity, Event, Kind, Property, records};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, REGION_TRANSFORMS, Simulation, Stop};

use common::{entity, send};

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
    /// Whether the screen was fully covered after each step.
    covered: Vec<bool>,
    /// The actor drawn halfway between the previous step and each step.
    drawn: Vec<Vec3>,
}

/// A game that runs `script[step]` each step and, after the tick, writes
/// the transform region `region(step)` gives: `(entity, x)` records.
struct Scripted<F, R> {
    step: u64,
    script: F,
    region: R,
    bytes: Vec<u8>,
    refused: Vec<(u64, String)>,
}

impl<F, R> Logic for Scripted<F, R>
where
    F: FnMut(u64, &mut Vec<u8>),
    R: FnMut(u64) -> Vec<(Entity, f32)>,
{
    fn tick(&mut self, events: &[u8], _: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        commands.clear();
        for e in records::<Event>(events) {
            if let Ok(Event::Rejected { code, .. }) = e {
                self.refused.push((self.step - 1, code.to_string()));
            }
        }
        if self.step == 0 {
            send(commands, spawn(Kind::Actor, ACTOR));
            send(commands, spawn(Kind::Empty, PROP));
        }
        (self.script)(self.step, commands);
        let records = (self.region)(self.step);
        self.bytes.clear();
        self.bytes
            .extend_from_slice(&(records.len() as u32).to_le_bytes());
        for (e, x) in records {
            for v in [e.index, e.generation] {
                self.bytes.extend_from_slice(&v.to_le_bytes());
            }
            for v in [x, 0.0, 0.0, 0.0] {
                self.bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        self.step += 1;
        Ok(())
    }

    fn regions(&self, each: &mut dyn FnMut(u32, &[u8])) {
        each(REGION_TRANSFORMS, &self.bytes);
    }
}

/// Runs the commands of `script[step]` at each step, the actor and the
/// prop spawned at step 0 at the origin, and the region records of
/// `region(step)`.
fn run_with(
    script: Vec<Vec<Command<'static>>>,
    region: impl FnMut(u64) -> Vec<(Entity, f32)>,
) -> Run {
    let steps = script.len() as u64 + 1;
    let logic = Scripted {
        step: 0,
        script: |step: u64, out: &mut Vec<u8>| {
            for c in script.get(step as usize).into_iter().flatten() {
                send(out, *c);
            }
        },
        region,
        bytes: Vec::new(),
        refused: Vec::new(),
    };
    let settings = Settings::default();
    let idle = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    let (mut at, mut covered, mut drawn) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..steps {
        sim.step(&idle);
        let world = &sim.state().world;
        at.push(world.current_pose(ACTOR).unwrap().position);
        drawn.push(world.interpolated(ACTOR, 0.5).unwrap().position);
        covered.push(sim.state().screen.covered());
    }
    sim.step(&idle);
    Run {
        at,
        refused: sim.logic().refused.clone(),
        covered,
        drawn,
    }
}

fn run(script: Vec<Vec<Command<'static>>>) -> Run {
    run_with(script, |_| Vec::new())
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

#[test]
fn a_tween_faster_than_a_visible_actor_may_move_is_a_jump() {
    let tween = |duration| Command::Tween {
        tween: 1,
        entity: ACTOR,
        property: Property::PositionX,
        var: 0,
        to: 50.0,
        duration,
        ease: Ease::Linear,
    };
    let r = run(vec![vec![], vec![tween(2.0 / 60.0)]]);
    assert_eq!(r.refused, refused_at(1), "25 m a step");
    assert!(r.at.iter().all(|p| p.x == 0.0));

    let r = run(vec![vec![], vec![tween(4.0)]]);
    assert!(r.refused.is_empty(), "12.5 m/s: {:?}", r.refused);
}

#[test]
fn a_spring_or_a_walk_faster_than_a_visible_actor_may_move_is_a_jump() {
    let spring = |frequency| Command::Spring {
        tween: 1,
        entity: ACTOR,
        property: Property::PositionX,
        var: 0,
        to: 5.0,
        frequency,
        damping: 1.0,
    };
    let r = run(vec![vec![], vec![spring(50.0)]]);
    assert_eq!(r.refused, refused_at(1));
    assert_eq!(r.at[2].x, 0.0);
    let r = run(vec![vec![], vec![spring(0.5)]]);
    assert!(r.refused.is_empty(), "{:?}", r.refused);

    let walk = |speed| Command::MoveTo {
        entity: ACTOR,
        target: [50.0, 0.0, 0.0],
        speed,
    };
    let r = run(vec![vec![], vec![walk(200.0)]]);
    assert_eq!(r.refused, refused_at(1));
    assert_eq!(r.at[2].x, 0.0);
    let r = run(vec![vec![], vec![walk(6.0)]]);
    assert!(r.refused.is_empty(), "a run: {:?}", r.refused);
}

#[test]
fn a_region_cannot_carry_a_visible_actor_away_at_once() {
    let parent = Command::SetParent {
        entity: ACTOR,
        parent: PROP,
    };
    let r = run_with(vec![vec![], vec![parent], vec![], vec![]], |step| {
        if step == 2 {
            vec![(PROP, 50.0)]
        } else {
            Vec::new()
        }
    });
    assert_eq!(r.refused, refused_at(2));
    assert!(r.at.iter().all(|p| p.x == 0.0), "{:?}", r.at);

    let follow = Command::Follow {
        entity: ACTOR,
        target: PROP,
        offset: [0.0; 3],
        lag: 0.0,
    };
    let r = run_with(vec![vec![], vec![follow], vec![], vec![]], |step| {
        if step == 2 {
            vec![(PROP, 50.0)]
        } else {
            Vec::new()
        }
    });
    assert_eq!(r.refused, refused_at(2), "a rigid follower is carried too");
    assert!(r.at.iter().all(|p| p.x == 0.0), "{:?}", r.at);

    let r = run_with(vec![vec![], vec![parent], vec![], vec![], vec![]], |step| {
        vec![(PROP, step as f32 * 0.1)]
    });
    assert!(r.refused.is_empty(), "a platform at 6 m/s: {:?}", r.refused);
    assert!(r.at[4].x > 0.0);
}

/// The farthest a visible actor may go between two frames.
const LIMIT: f32 = 20.0 * STEP_SECONDS * 1.001;

fn moved(r: &Run, k: usize) -> f32 {
    r.at[k].distance(r.at[k - 1])
}

fn tween_to(entity: Entity, property: Property, to: f32, steps: u32) -> Command<'static> {
    Command::Tween {
        tween: entity.index * 8 + property as u32,
        entity,
        property,
        var: 0,
        to,
        duration: steps as f32 * STEP_SECONDS,
        ease: Ease::Linear,
    }
}

#[test]
fn a_fast_tween_begun_under_a_fade_stops_once_the_fade_lifts() {
    let fade_in = Command::Fade {
        to: 0.0,
        duration: 0.5,
        color: [0.0; 3],
    };
    let tween = tween_to(ACTOR, Property::PositionX, 100.0, 60);
    let mut script = vec![vec![], vec![BLACK], vec![tween], vec![fade_in]];
    script.resize(12, vec![]);
    let r = run(script);
    assert_eq!(r.refused, refused_at(4), "judged once a frame shows it");
    for k in 1..r.at.len() {
        if !(r.covered[k] || r.covered[k - 1]) {
            assert!(moved(&r, k) <= LIMIT, "step {k}: {} m", moved(&r, k));
        }
    }
}

#[test]
fn a_fast_tween_begun_on_a_cut_stops_on_the_next_step() {
    let script = vec![
        vec![],
        vec![
            Command::RigFixed {
                rig: 1,
                position: [0.0, 5.0, 5.0],
                look_at: [0.0; 3],
                fov: 40.0,
            },
            Command::CameraUse {
                rig: 1,
                blend: 0.0,
                ease: Ease::Linear,
            },
            tween_to(ACTOR, Property::PositionX, 100.0, 30),
        ],
        vec![],
        vec![],
    ];
    let r = run(script);
    assert_eq!(r.refused, refused_at(2));
    assert!(r.at[1].x > 3.0, "the cut hides the first step");
    assert_eq!(r.at[2], r.at[1]);
    assert_eq!(r.at[3], r.at[1]);
}

#[test]
fn a_fast_walk_begun_while_hidden_stops_once_it_is_seen_moving() {
    let walk = Command::MoveTo {
        entity: ACTOR,
        target: [100.0, 0.0, 0.0],
        speed: 200.0,
    };
    let r = run(vec![
        vec![],
        vec![visible(ACTOR, false)],
        vec![walk],
        vec![],
        vec![visible(ACTOR, true)],
        vec![],
        vec![],
    ]);
    assert_eq!(r.refused, refused_at(5), "it appeared at 4, moved at 5");
    assert!(r.at[4].x > 5.0);
    assert_eq!(r.at[5], r.at[4]);
    assert_eq!(r.at[6], r.at[4]);
}

#[test]
fn a_carrier_that_turns_or_grows_cannot_sweep_a_visible_child_away() {
    let parent = Command::SetParent {
        entity: ACTOR,
        parent: PROP,
    };
    let setup = vec![
        vec![],
        vec![visible(ACTOR, false)],
        vec![set_transform(ACTOR, 50.0), parent, visible(ACTOR, true)],
    ];
    for motion in [
        tween_to(PROP, Property::Yaw, 320.0, 40),
        tween_to(PROP, Property::Scale, 3.0, 30),
    ] {
        let mut script = setup.clone();
        script.push(vec![motion]);
        script.resize(8, vec![]);
        let r = run(script);
        assert_eq!(r.refused, refused_at(3), "{motion:?}");
        for k in 3..r.at.len() {
            assert!(
                r.at[k].distance(r.at[2]) < 1e-3,
                "{motion:?} step {k}: {:?}",
                r.at[k]
            );
        }
    }
}

#[test]
fn moves_that_each_pass_but_sum_past_the_limit_are_a_jump() {
    let r = run(vec![
        vec![],
        vec![
            tween_to(ACTOR, Property::PositionX, 18.0, 60),
            tween_to(ACTOR, Property::PositionZ, 18.0, 60),
        ],
        vec![],
    ]);
    assert_eq!(
        r.refused,
        refused_at(1),
        "0.3 m and 0.3 m are 0.42 m a step"
    );
    assert_eq!(r.at[1], Vec3::ZERO);
    assert_eq!(r.at[2], Vec3::ZERO);

    let parent = Command::SetParent {
        entity: ACTOR,
        parent: PROP,
    };
    let r = run_with(vec![vec![], vec![parent], vec![], vec![]], |step| {
        if step == 2 {
            vec![(PROP, 0.2), (PROP, 0.4)]
        } else {
            Vec::new()
        }
    });
    assert_eq!(r.refused, refused_at(2), "two records, 0.4 m in one step");
    assert!(r.at.iter().all(|p| p.x == 0.0), "{:?}", r.at);
}

#[test]
fn an_ordinary_spring_is_not_refused() {
    let spring = Command::Spring {
        tween: 1,
        entity: ACTOR,
        property: Property::PositionX,
        var: 0,
        to: 5.0,
        frequency: 1.0,
        damping: 1.0,
    };
    let mut script = vec![vec![], vec![spring]];
    script.resize(120, vec![]);
    let r = run(script);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert!((r.at[119].x - 5.0).abs() < 0.05, "{}", r.at[119].x);
}

#[test]
fn a_forced_teleport_carries_a_visible_child_without_sliding_it() {
    let parent = Command::SetParent {
        entity: ACTOR,
        parent: PROP,
    };
    let force = Command::Teleport {
        entity: PROP,
        position: [50.0, 0.0, 0.0],
        yaw: 0.0,
        force: true,
    };
    let r = run(vec![vec![], vec![parent], vec![force]]);
    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert_eq!(r.at[2].x, 50.0);
    assert_eq!(r.drawn[2].x, 50.0, "drawn where it lands, not halfway");
}
