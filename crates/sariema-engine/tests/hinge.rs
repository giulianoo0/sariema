// SPDX-License-Identifier: Apache-2.0

//! A door leaf the game turns by tweening `hinge.<node>`: the collider turns
//! with the leaf drawn, step for step, and stops short of an actor in its
//! swing instead of pushing it, telling the game once.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::collision::{Collider, CollisionPack, Hinge, Shape};
use sariema_format::scenery::{NodeIndex, Param, RoomIndex, Target};
use sariema_format::script::{Command, Ease, Entity, Event, Property, name_id};
use sariema_physics::ColliderId;
use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, entity, native, send, spawn_hero, wired};

const LEAF: &str = "doorway.door_leaf";
const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

fn wall(name: &str, x: f32) -> Collider {
    Collider {
        name: name.into(),
        shape: Shape::Box {
            center: [x, 1.1, 0.0],
            half_extents: [3.0, 1.1, 0.1],
            rotation: IDENTITY,
        },
        enabled: true,
    }
}

/// A wall along x at z = 0 with a doorway from x −0.5 to 0.5, its leaf
/// hinged on the west jamb and open 90° to the north (−z).
fn doorway() -> RoomAssets {
    let hero = assets().rooms[0].collision.as_ref().unwrap().actors.clone();
    let floor = Collider {
        name: "floor".into(),
        shape: Shape::Box {
            center: [0.0, -0.5, 0.0],
            half_extents: [10.0, 0.5, 10.0],
            rotation: IDENTITY,
        },
        enabled: true,
    };
    let leaf = Collider {
        name: LEAF.into(),
        shape: Shape::Box {
            center: [0.0, 1.0, 0.0],
            half_extents: [0.47, 1.0, 0.025],
            rotation: IDENTITY,
        },
        enabled: true,
    };
    let pack = CollisionPack {
        room: "doorway".into(),
        colliders: vec![floor, wall("west", -3.5), wall("east", 3.5), leaf],
        triggers: Vec::new(),
        actors: hero,
        spots: Vec::new(),
        routes: Vec::new(),
        traversals: Vec::new(),
        nav_bounds: None,
        hinges: vec![Hinge {
            collider: 3,
            pivot: [-0.49, 0.0, 0.0],
            axis: [0.0, 1.0, 0.0],
            angle_deg: 90.0,
        }],
    };
    let hinge = sariema_format::Hinge {
        axis: [0.0, 1.0, 0.0],
        angle_deg: 90.0,
    };
    let pivot = glam::Mat4::from_translation(glam::Vec3::new(-0.49, 0.0, 0.0));
    RoomAssets {
        index: RoomIndex {
            nodes: vec![NodeIndex {
                name: LEAF.into(),
                pivot: pivot.to_cols_array(),
                visible: true,
                hinge: Some(hinge),
            }],
            params: vec![Param {
                name: format!("hinge.{LEAF}"),
                value: 90.0,
                target: Target::Hinge(0),
            }],
            ..RoomIndex::default()
        },
        ..RoomAssets::collision("doorway", pack)
    }
}

fn close(seconds: f32) -> Command<'static> {
    Command::Tween {
        tween: 7,
        entity: Entity::NONE,
        property: Property::Scene,
        var: name_id(&format!("hinge.{LEAF}")),
        to: 0.0,
        duration: seconds,
        ease: Ease::Linear,
    }
}

struct Run {
    /// Every hinge_blocked, with its step.
    blocked: Vec<(u64, u32, Entity)>,
    /// The step the tween ended at.
    done: Option<u64>,
    /// Each step: the leaf's angle drawn, its collider's, and the hero.
    steps: Vec<(f32, f32, glam::Vec3)>,
}

/// The hero spawned at `from`, the leaf closed over `seconds` from step 5,
/// and `then` sent at the steps it names.
fn run(
    from: [f32; 3],
    seconds: f32,
    then: impl Fn(u64, &mut Vec<u8>) + 'static,
    steps: u64,
) -> Run {
    let seen = Rc::new(RefCell::new((Vec::new(), None)));
    let log = seen.clone();
    let logic = native(move |n, events, out| {
        let mut log = log.borrow_mut();
        for e in events {
            match e {
                Event::HingeBlocked { hinge, entity } => log.0.push((n, *hinge, *entity)),
                Event::TweenDone { tween: 7 } => log.1 = Some(n),
                _ => {}
            }
        }
        if n == 0 {
            spawn_hero(out, from, 0.0);
        }
        if n == 5 {
            send(out, close(seconds));
        }
        then(n, out);
    });
    let mut assets = assets();
    assets.rooms = vec![doorway()];
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let param = name_id(&format!("hinge.{LEAF}"));
    let mut out = Vec::new();
    for _ in 0..steps {
        engine.step(&actions);
        let drawn = engine.state().scenery.param(param).unwrap();
        let pose = (engine.physics().unwrap().world)
            .platform_pose(ColliderId(3))
            .unwrap();
        let hero = engine.state().world.node(entity(0)).unwrap().local.position;
        out.push((drawn, pose.yaw.to_degrees(), hero));
    }
    let (blocked, done) = seen.take();
    Run {
        blocked,
        done,
        steps: out,
    }
}

fn drawn_as_it_collides(run: &Run) {
    for (k, &(drawn, collides, _)) in run.steps.iter().enumerate() {
        assert!(
            (drawn - collides).abs() < 1e-3,
            "step {k}: drawn at {drawn}°, colliding at {collides}°"
        );
    }
}

#[test]
fn a_leaf_closing_on_the_hero_in_its_swing_stops_tells_the_game_once_and_goes_on_after() {
    let away = |n: u64, out: &mut Vec<u8>| {
        if n == 150 {
            send(
                out,
                Command::Teleport {
                    entity: entity(0),
                    position: [0.0, 0.0, -2.5],
                    yaw: 0.0,
                    force: true,
                    hold: false,
                },
            );
        }
    };
    let run = run([0.0, 0.0, -0.45], 1.0, away, 240);
    drawn_as_it_collides(&run);
    let hinge = name_id(&format!("hinge.{LEAF}"));
    assert_eq!(run.blocked.len(), 1, "{:?}", run.blocked);
    let (at, which, who) = run.blocked[0];
    assert_eq!((which, who), (hinge, entity(0)));
    let standing = run.steps[at as usize - 1].2;
    for &(drawn, _, hero) in &run.steps[at as usize..150] {
        assert!(drawn > 10.0 && drawn < 80.0, "stopped at {drawn}°");
        assert!((hero - standing).length() < 1e-4, "pushed to {hero}");
    }
    let done = run.done.expect("the leaf closed once the hero was away");
    assert!(done > 150, "done at step {done}");
    assert_eq!(run.steps.last().unwrap().0, 0.0);
}

#[test]
fn a_leaf_closing_while_the_hero_walks_through_waits_and_never_pushes_it() {
    let walk = |n: u64, out: &mut Vec<u8>| {
        if n == 5 {
            send(
                out,
                Command::MoveTo {
                    entity: entity(0),
                    target: [0.0, 0.0, 2.0],
                    speed: 2.0,
                },
            );
        }
    };
    let run = run([0.0, 0.0, -0.8], 0.67, walk, 240);
    drawn_as_it_collides(&run);
    assert!(!run.blocked.is_empty(), "the leaf never met the hero");
    let mut last = run.steps[5].2;
    for &(_, _, hero) in &run.steps[5..] {
        assert!(hero.z >= last.z - 1e-4, "pushed back from {last} to {hero}");
        last = hero;
    }
    assert!(last.z > 1.5, "the hero did not get through: {last}");
    assert!(run.done.is_some(), "the leaf did not finish closing");
    assert_eq!(run.steps.last().unwrap().0, 0.0);
}

const OPEN_ON_CUE: &str = r#"{
  "name": "shut",
  "skippable": false,
  "roles": { "hero": { "start": { "position": [0.0, 0.0, 1.5], "yaw": 0.0 } } },
  "steps": [
    { "parallel": [
      [ { "clip": { "role": "hero", "clip": "wave", "mode": "once", "blend": 0.0 } } ],
      [ { "hinge": { "node": "doorway.door_leaf", "role": "hero", "clip": "wave",
                     "span": [0.2, 0.7], "to": 0.0, "ease": "linear" } } ]
    ] }
  ]
}"#;

#[test]
fn a_cutscene_turns_a_leaf_with_the_time_of_an_actors_clip() {
    let cutscene = compile_json(OPEN_ON_CUE).unwrap().0;
    let ended = Rc::new(RefCell::new(None));
    let log = ended.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::CutsceneEnded { .. } = e {
                *log.borrow_mut() = Some(n);
            }
        }
        if n == 0 {
            spawn_hero(out, [0.0, 0.0, 1.5], 0.0);
        }
        if n == 2 {
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
                    cutscene: name_id("shut"),
                },
            );
        }
    });
    let mut assets = assets();
    assets.rooms = vec![doorway()];
    let sim = Simulation::new(wired(), vec![cutscene], logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let param = name_id(&format!("hinge.{LEAF}"));
    let mut compared = 0;
    for _ in 0..90 {
        engine.step(&actions);
        let drawn = engine.state().scenery.param(param).unwrap();
        let pose = (engine.physics().unwrap().world)
            .platform_pose(ColliderId(3))
            .unwrap();
        assert!((drawn - pose.yaw.to_degrees()).abs() < 1e-3);
        let Some((clip, t)) = engine.state().clip_time(entity(0)) else {
            assert_eq!(drawn, 90.0);
            continue;
        };
        assert_eq!(clip, name_id("wave"));
        let f = ((t - 0.2) / 0.5).clamp(0.0, 1.0);
        let want = 90.0 * (1.0 - f);
        assert!(
            (drawn - want).abs() < 1e-3,
            "clip at {t} s: leaf at {drawn}°, wanted {want}°"
        );
        compared += 1;
    }
    assert!(compared > 45, "{compared}");
    assert!(ended.borrow().is_some(), "the cutscene ended with the span");
}

/// Plays a cutscene of one hinge step (and a clip when `clip`), and gives
/// the steps it ended at and every rejected code, with the leaf's angle at
/// the end.
fn hinge_alone(node: &str, clip: bool, steps: u64) -> (Option<u64>, Vec<(u64, String)>, f32) {
    let wave = if clip {
        r#"{ "clip": { "role": "hero", "clip": "wave", "mode": "once", "blend": 0.0 } },"#
    } else {
        ""
    };
    let text = format!(
        r#"{{ "name": "shut", "skippable": false,
  "roles": {{ "hero": {{ "start": {{ "position": [0.0, 0.0, 1.5], "yaw": 0.0 }} }} }},
  "steps": [ {wave}
    {{ "hinge": {{ "node": "{node}", "role": "hero", "clip": "wave", "span": [0.2, 0.7], "to": 0.0 }} }} ] }}"#
    );
    let cutscene = compile_json(&text).unwrap().0;
    let seen = Rc::new(RefCell::new((None, Vec::new())));
    let log = seen.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            match e {
                Event::CutsceneEnded { .. } => log.borrow_mut().0 = Some(n),
                Event::Rejected { code, .. } => log.borrow_mut().1.push((n, code.to_string())),
                _ => {}
            }
        }
        if n == 0 {
            spawn_hero(out, [0.0, 0.0, 1.5], 0.0);
        }
        if n == 2 {
            let (role, entity) = (name_id("hero"), entity(0));
            send(out, Command::CutsceneBind { role, entity });
            let cutscene = name_id("shut");
            send(out, Command::CutscenePlay { cutscene });
        }
    });
    let mut assets = assets();
    assets.rooms = vec![doorway()];
    let sim = Simulation::new(wired(), vec![cutscene], logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..steps {
        engine.step(&actions);
    }
    let leaf = engine
        .state()
        .scenery
        .param(name_id(&format!("hinge.{LEAF}")))
        .unwrap();
    let (ended, rejected) = seen.take();
    (ended, rejected, leaf)
}

#[test]
fn a_hinge_step_on_a_node_the_room_lacks_is_rejected_and_ends() {
    let (ended, rejected, leaf) = hinge_alone("doorway.door_lef", true, 30);
    assert!(
        rejected.iter().any(|(_, code)| code == "E_SCENE_UNKNOWN"),
        "{rejected:?}"
    );
    assert!(ended.is_some(), "the cutscene goes on");
    assert_eq!(leaf, 90.0);
}

#[test]
fn a_hinge_step_whose_role_never_plays_its_clip_gives_up_after_its_setting() {
    let limit = wired().cutscene.hinge_clip_wait;
    assert!(limit > 0.0 && limit <= 5.0, "{limit}");
    let (ended, rejected, leaf) = hinge_alone(LEAF, false, 60 * 6);
    let gave_up: Vec<u64> = (rejected.iter())
        .filter(|(_, code)| code == "E_CUTSCENE_HINGE_WAIT")
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(gave_up.len(), 1, "{rejected:?}");
    let waited = (gave_up[0] as f32 - 3.0) / 60.0;
    assert!((waited - limit).abs() < 0.1, "gave up after {waited} s");
    assert!(ended.is_some(), "the cutscene goes on");
    assert_eq!(leaf, 90.0, "the leaf does not pop to its end");
}
