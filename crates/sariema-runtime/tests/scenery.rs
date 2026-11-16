// SPDX-License-Identifier: Apache-2.0

//! Rooms, groups, piece states and scene parameters as the step keeps them.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_format::scenery::{LightIndex, NodeIndex, Param, PieceIndex, RoomIndex, Target};
use sariema_format::script::{
    Command, Ease, Entity, Event, Kind, Property, RoomTransition, name_id, put_scene_value,
};
use sariema_runtime::scenery::{Binding, RoomRequest, RoomScene};
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn house() -> RoomIndex {
    let mut pivot = IDENTITY;
    pivot[12..15].copy_from_slice(&[2.0, 0.5, -3.0]);
    RoomIndex {
        groups: vec![("kitchen".into(), true), ("hall".into(), false)],
        nodes: vec![NodeIndex {
            name: "kitchen.door".into(),
            pivot,
            visible: false,
            hinge: None,
        }],
        pieces: vec![PieceIndex {
            name: "kitchen.tv".into(),
            states: vec!["".into(), "on".into(), "static".into()],
            state: 0,
        }],
        lights: vec![LightIndex {
            name: "kitchen.lamp".into(),
            position: [1.0, 2.5, -1.0],
        }],
        params: vec![
            Param {
                name: "look.exposure".into(),
                value: 1.0,
                target: Target::Exposure,
            },
            Param {
                name: "hinge.kitchen.door".into(),
                value: 0.0,
                target: Target::Hinge(0),
            },
        ],
    }
}

fn garden() -> RoomIndex {
    RoomIndex {
        groups: vec![("lawn".into(), true)],
        params: vec![Param {
            name: "look.exposure".into(),
            value: 0.6,
            target: Target::Exposure,
        }],
        ..RoomIndex::default()
    }
}

type Script = Rc<RefCell<Vec<(u64, Command<'static>)>>>;

/// A simulation over the house and the garden whose game sends, at each
/// step, the commands `script` holds for it, and records every event.
fn simulation(
    script: &Script,
    seen: &Rc<RefCell<Vec<(u64, String)>>>,
) -> Simulation<impl sariema_runtime::sim::Logic + use<>> {
    let (script, seen) = (script.clone(), seen.clone());
    let logic = native(move |step, events, out| {
        for e in events {
            seen.borrow_mut().push((step, format!("{e:?}")));
        }
        for (_, c) in script.borrow().iter().filter(|(at, _)| *at == step) {
            send(out, *c);
        }
    });
    let mut sim = Simulation::new(Settings::default(), Vec::new(), logic, 7);
    sim.set_rooms(vec![
        RoomScene::new("house", house()),
        RoomScene::new("garden", garden()),
    ]);
    sim
}

fn run(
    commands: Vec<(u64, Command<'static>)>,
    steps: u64,
) -> (
    Vec<(u64, String)>,
    Simulation<impl sariema_runtime::sim::Logic>,
) {
    let script = Rc::new(RefCell::new(commands));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut sim = simulation(&script, &seen);
    for _ in 0..steps {
        sim.step(&Default::default());
    }
    let seen = seen.borrow().clone();
    (seen, sim)
}

fn rejected(seen: &[(u64, String)], code: &str) -> bool {
    seen.iter()
        .any(|(_, e)| e.contains("Rejected") && e.contains(code))
}

#[test]
fn a_group_switches_in_the_active_room_and_an_unknown_one_is_refused() {
    let (seen, sim) = run(
        vec![
            (
                0,
                Command::RoomGroup {
                    group: name_id("hall"),
                    on: true,
                },
            ),
            (
                0,
                Command::RoomGroup {
                    group: name_id("kitchen"),
                    on: false,
                },
            ),
            (
                1,
                Command::RoomGroup {
                    group: name_id("cellar"),
                    on: true,
                },
            ),
        ],
        3,
    );
    let house = &sim.state().scenery.rooms()[0];
    assert_eq!(house.groups, [false, true]);
    assert!(rejected(&seen, "E_SCENE_UNKNOWN"));
}

#[test]
fn a_piece_shows_a_state_by_name_and_state_zero_is_its_start_look() {
    let piece = name_id("kitchen.tv");
    let (seen, sim) = run(
        vec![
            (
                0,
                Command::PieceState {
                    piece,
                    state: name_id("static"),
                },
            ),
            (
                1,
                Command::PieceState {
                    piece,
                    state: name_id("broken"),
                },
            ),
        ],
        3,
    );
    assert_eq!(sim.state().scenery.rooms()[0].pieces, [2]);
    assert!(rejected(&seen, "E_SCENE_UNKNOWN"));
    let (_, sim) = run(
        vec![
            (
                0,
                Command::PieceState {
                    piece,
                    state: name_id("on"),
                },
            ),
            (1, Command::PieceState { piece, state: 0 }),
        ],
        2,
    );
    assert_eq!(sim.state().scenery.rooms()[0].pieces, [0]);
}

fn exposure_tween(to: f32, duration: f32) -> Command<'static> {
    Command::Tween {
        tween: 1,
        entity: Entity::NONE,
        property: Property::Scene,
        var: name_id("look.exposure"),
        to,
        duration,
        ease: Ease::Linear,
    }
}

#[test]
fn a_scene_parameter_tweens_without_a_jump_and_lands_on_its_value() {
    // One second, 60 steps, from 1.0 to 3.0, the first in the step that
    // gave it: 30 steps in is the middle.
    let (_, sim) = run(vec![(0, exposure_tween(3.0, 1.0))], 30);
    assert_eq!(sim.state().scenery.param(name_id("look.exposure")), Ok(2.0));
    let (seen, sim) = run(vec![(0, exposure_tween(3.0, 1.0))], 62);
    assert_eq!(sim.state().scenery.param(name_id("look.exposure")), Ok(3.0));
    assert!(seen.iter().any(|(_, e)| e.contains("TweenDone")));
    let mut last = 1.0;
    let script = Rc::new(RefCell::new(vec![(0, exposure_tween(3.0, 1.0))]));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut sim = simulation(&script, &seen);
    for _ in 0..62 {
        sim.step(&Default::default());
        let now = sim.state().scenery.param(name_id("look.exposure")).unwrap();
        assert!((now - last).abs() <= 2.0 / 60.0 + 1e-5, "{last} to {now}");
        last = now;
    }
}

#[test]
fn an_unknown_scene_parameter_is_refused() {
    let mut tween = exposure_tween(2.0, 1.0);
    if let Command::Tween { var, .. } = &mut tween {
        *var = name_id("look.sparkle");
    }
    let (seen, _) = run(vec![(0, tween)], 2);
    assert!(rejected(&seen, "E_SCENE_UNKNOWN"));
}

/// `scene_tween` driving each named parameter to its value under `tween`.
fn bundle(tween: u32, values: &[(&str, f32)], duration: f32, ease: Ease) -> Command<'static> {
    let mut bytes = Vec::new();
    for (name, value) in values {
        put_scene_value(&mut bytes, name_id(name), *value);
    }
    Command::SceneTween {
        tween,
        values: Box::leak(bytes.into_boxed_slice()),
        duration,
        ease,
    }
}

fn done(seen: &[(u64, String)], tween: u32) -> Vec<u64> {
    let mark = format!("TweenDone {{ tween: {tween} }}");
    (seen.iter())
        .filter(|(_, e)| e.contains(&mark))
        .map(|(step, _)| *step)
        .collect()
}

#[test]
fn a_bundle_tweens_its_parameters_together_and_reports_done_once() {
    let door = [("look.exposure", 3.0), ("hinge.kitchen.door", 90.0)];
    let (_, sim) = run(vec![(0, bundle(5, &door, 1.0, Ease::Linear))], 30);
    let scenery = &sim.state().scenery;
    assert_eq!(scenery.param(name_id("look.exposure")), Ok(2.0));
    assert_eq!(scenery.param(name_id("hinge.kitchen.door")), Ok(45.0));
    let (_, sim) = run(vec![(0, bundle(5, &door, 1.0, Ease::InOutSine))], 30);
    let middle = sim.state().scenery.param(name_id("look.exposure")).unwrap();
    assert!((middle - 2.0).abs() < 1e-5, "{middle}");
    let (seen, sim) = run(vec![(0, bundle(5, &door, 1.0, Ease::Linear))], 90);
    let scenery = &sim.state().scenery;
    assert_eq!(scenery.param(name_id("look.exposure")), Ok(3.0));
    assert_eq!(scenery.param(name_id("hinge.kitchen.door")), Ok(90.0));
    assert_eq!(done(&seen, 5).len(), 1, "{seen:?}");
}

#[test]
fn a_bundle_takes_over_a_parameter_without_a_jump_and_leaves_the_others() {
    // The exposure tween is halfway (2.0) when the bundle takes it to 1.0
    // over the next second; the hinge, in no bundle, keeps its own tween.
    let hinge = Command::Tween {
        tween: 9,
        entity: Entity::NONE,
        property: Property::Scene,
        var: name_id("hinge.kitchen.door"),
        to: 60.0,
        duration: 1.0,
        ease: Ease::Linear,
    };
    let script = Rc::new(RefCell::new(vec![
        (0, exposure_tween(3.0, 1.0)),
        (0, hinge),
        (30, bundle(5, &[("look.exposure", 1.0)], 1.0, Ease::Linear)),
    ]));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut sim = simulation(&script, &seen);
    let mut last = 1.0;
    for _ in 0..100 {
        sim.step(&Default::default());
        let now = sim.state().scenery.param(name_id("look.exposure")).unwrap();
        assert!((now - last).abs() <= 2.0 / 60.0 + 1e-5, "{last} to {now}");
        last = now;
    }
    let scenery = &sim.state().scenery;
    assert_eq!(scenery.param(name_id("look.exposure")), Ok(1.0));
    assert_eq!(scenery.param(name_id("hinge.kitchen.door")), Ok(60.0));
    let seen = seen.borrow();
    assert_eq!(
        done(&seen, 1),
        Vec::<u64>::new(),
        "the exposure tween was replaced"
    );
    assert_eq!(done(&seen, 9).len(), 1);
    assert_eq!(done(&seen, 5).len(), 1);
}

#[test]
fn a_bundle_with_an_unknown_parameter_or_a_broken_value_changes_nothing() {
    let unknown = bundle(
        5,
        &[("look.exposure", 3.0), ("look.sparkle", 1.0)],
        1.0,
        Ease::Linear,
    );
    let (seen, sim) = run(vec![(0, unknown)], 30);
    assert!(rejected(&seen, "E_SCENE_UNKNOWN"));
    assert_eq!(sim.state().scenery.param(name_id("look.exposure")), Ok(1.0));
    let mut bytes = Vec::new();
    put_scene_value(&mut bytes, name_id("look.exposure"), 3.0);
    bytes.pop();
    let torn = Command::SceneTween {
        tween: 5,
        values: Box::leak(bytes.into_boxed_slice()),
        duration: 1.0,
        ease: Ease::Linear,
    };
    let not_a_number = bundle(6, &[("look.exposure", f32::NAN)], 1.0, Ease::Linear);
    let (seen, sim) = run(vec![(0, torn), (1, not_a_number)], 30);
    let refused = (seen.iter())
        .filter(|(_, e)| e.contains("Rejected") && e.contains("E_SCENE_VALUES"))
        .count();
    assert_eq!(refused, 2, "{seen:?}");
    assert_eq!(sim.state().scenery.param(name_id("look.exposure")), Ok(1.0));
}

#[test]
fn the_values_of_the_step_before_stay_for_the_frames_between_steps() {
    let (_, mut sim) = run(vec![(0, exposure_tween(3.0, 1.0))], 30);
    let room = &sim.state().scenery.rooms()[0];
    let at = room.param(name_id("look.exposure")).unwrap();
    assert!((room.previous[at] - (1.0 + 2.0 * 29.0 / 60.0)).abs() < 1e-6);
    assert_eq!(room.params[at], 2.0);
    sim.step(&Default::default());
    let room = &sim.state().scenery.rooms()[0];
    assert_eq!(room.previous[at], 2.0);
}

#[test]
fn a_bundle_changes_the_world_hash_only_once_sent() {
    let quiet = run(Vec::new(), 40).1.state().world_hash();
    let still = run(
        vec![(50, bundle(5, &[("look.exposure", 3.0)], 1.0, Ease::Linear))],
        40,
    );
    assert_eq!(still.1.state().world_hash(), quiet);
    let sent = run(
        vec![(10, bundle(5, &[("look.exposure", 3.0)], 1.0, Ease::Linear))],
        40,
    );
    assert_ne!(sent.1.state().world_hash(), quiet);
    let again = run(
        vec![(10, bundle(5, &[("look.exposure", 3.0)], 1.0, Ease::Linear))],
        40,
    );
    assert_eq!(again.1.state().world_hash(), sent.1.state().world_hash());
}

fn switch(room: &str, transition: RoomTransition, duration: f32) -> Command<'static> {
    Command::RoomSwitch {
        room: name_id(room),
        transition,
        duration,
    }
}

#[test]
fn a_switch_makes_another_room_active_and_shows_for_its_duration() {
    let script: Script = Rc::new(RefCell::new(vec![(
        2,
        switch("garden", RoomTransition::Dissolve, 0.25),
    )]));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut sim = simulation(&script, &seen);
    for _ in 0..3 {
        sim.step(&Default::default());
    }
    let scenery = &sim.state().scenery;
    assert_eq!(scenery.active(), 1);
    assert_eq!(scenery.requests, [RoomRequest::Switch(1)]);
    let shown = scenery.switch().unwrap();
    assert_eq!(
        (shown.from, shown.kind, shown.steps),
        (0, RoomTransition::Dissolve, 15)
    );
    assert_eq!(scenery.param(name_id("look.exposure")), Ok(0.6));
    for _ in 0..15 {
        assert!(sim.state().scenery.switch().is_some());
        sim.step(&Default::default());
    }
    assert_eq!(sim.state().scenery.switch(), None);
    assert!(
        seen.borrow()
            .iter()
            .any(|(at, e)| *at == 3 && e.contains("RoomSwitched"))
    );
}

#[test]
fn a_cut_has_no_transition_and_an_unknown_room_is_refused() {
    let (seen, sim) = run(
        vec![
            (0, switch("garden", RoomTransition::Cut, 0.5)),
            (1, switch("attic", RoomTransition::Cut, 0.0)),
            (
                1,
                Command::RoomPreload {
                    room: name_id("cellar"),
                },
            ),
        ],
        3,
    );
    assert_eq!(sim.state().scenery.active(), 1);
    assert_eq!(sim.state().scenery.switch(), None);
    assert!(rejected(&seen, "E_ROOM_UNKNOWN"));
}

#[test]
fn a_room_keeps_what_the_game_changed_when_it_is_left_and_entered_again() {
    let (_, sim) = run(
        vec![
            (
                0,
                Command::RoomGroup {
                    group: name_id("hall"),
                    on: true,
                },
            ),
            (1, switch("garden", RoomTransition::Cut, 0.0)),
            (
                2,
                Command::RoomGroup {
                    group: name_id("lawn"),
                    on: false,
                },
            ),
            (3, switch("house", RoomTransition::Cut, 0.0)),
        ],
        4,
    );
    let scenery = &sim.state().scenery;
    assert_eq!(scenery.active(), 0);
    assert_eq!(scenery.rooms()[0].groups, [true, true]);
    assert_eq!(scenery.rooms()[1].groups, [false]);
}

#[test]
fn a_room_node_or_light_entity_starts_where_the_room_has_it() {
    let (_, sim) = run(
        vec![
            (
                0,
                Command::Spawn {
                    entity: entity(1),
                    kind: Kind::RoomNode,
                    asset: name_id("kitchen.door"),
                },
            ),
            (
                0,
                Command::Spawn {
                    entity: entity(2),
                    kind: Kind::Light,
                    asset: name_id("kitchen.lamp"),
                },
            ),
            (
                0,
                Command::Spawn {
                    entity: entity(3),
                    kind: Kind::RoomNode,
                    asset: name_id("nothing"),
                },
            ),
        ],
        1,
    );
    let s = sim.state();
    let door = s.world.node(entity(1)).unwrap();
    assert_eq!(door.local.position.to_array(), [2.0, 0.5, -3.0]);
    assert!(
        !door.visible,
        "the door node starts hidden, as the room has it"
    );
    assert!(matches!(
        s.scenery.binding(&s.world, entity(1)),
        Some(Binding::Node {
            room: 0,
            node: 0,
            ..
        })
    ));
    let lamp = s.world.node(entity(2)).unwrap();
    assert_eq!(lamp.local.position.to_array(), [1.0, 2.5, -1.0]);
    assert_eq!(
        s.scenery.binding(&s.world, entity(2)),
        Some(Binding::Light { room: 0, light: 0 })
    );
    assert_eq!(s.scenery.binding(&s.world, entity(3)), None);
}

#[test]
fn rooms_left_as_they_load_leave_the_world_hash_alone() {
    let script = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut with_rooms = simulation(&script, &seen);
    let logic = native(|_, _: &[Event<'_>], _| {});
    let mut without = Simulation::new(Settings::default(), Vec::new(), logic, 7);
    with_rooms.step(&Default::default());
    without.step(&Default::default());
    assert_eq!(
        with_rooms.state().world_hash(),
        without.state().world_hash()
    );
    let (_, changed) = run(
        vec![(
            0,
            Command::RoomGroup {
                group: name_id("hall"),
                on: true,
            },
        )],
        1,
    );
    assert_ne!(changed.state().world_hash(), without.state().world_hash());
}
