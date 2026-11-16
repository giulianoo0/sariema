// SPDX-License-Identifier: Apache-2.0

//! Switching rooms: the active room's collision moves the actors, which
//! cross into the next room's where they were, at their speed and on
//! their way; a world built off the step runs as one built in it; each
//! room's ambience gives way to the next one's.

mod common;

use std::path::Path;
use std::sync::Arc;

use glam::Vec3;
use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::script::{Command, Event, RoomTransition, name_id};
use sariema_import::{RoomSource, build_collision};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::{Logic, Simulation};

use common::{assets, entity, native, send, spawn_hero, wired};

/// `sill` (the climb's room, whose row of boxes at z −3 is closed at
/// x 3..4 by the sill) and `hall` (the same room with a gap there), each the
/// other's neighbour.
fn rooms() -> Vec<RoomAssets> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/collision");
    let room = |name: &str, neighbour: u32, ambience: &str| {
        let text = std::fs::read_to_string(root.join(format!("rooms/{name}.json"))).unwrap();
        let source = RoomSource::from_json(&text).unwrap();
        RoomAssets {
            name: name.into(),
            collision: Some(Arc::new(build_collision(&source, &root).unwrap().unwrap())),
            neighbours: vec![neighbour],
            ambience: Some(ambience.into()),
            ..RoomAssets::default()
        }
    };
    vec![room("sill", 1, "wind"), room("hall", 0, "theme")]
}

/// Actors reach their speed over half a second, so a world that lost
/// theirs would show.
fn two_rooms<L: Logic>(logic: L) -> Engine<L> {
    let mut assets = assets();
    assets.rooms = rooms();
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut settings = EngineSettings::default();
    settings.physics.controller.accel_steps = 30;
    Engine::new(sim, assets, settings).unwrap()
}

struct Walk {
    from: [f32; 3],
    to: [f32; 3],
    /// The step the game switches to the hall at, and how.
    switch: Option<(u64, RoomTransition)>,
    /// The host builds the hall's collision world off the step, this many
    /// grid columns at a time.
    take_jobs: Option<usize>,
}

/// The hero walking from `from` to `to`: its position at every step, the
/// engine's hash, and the step the hall became active at.
fn walk(w: Walk) -> (Vec<Vec3>, u64, Option<u64>) {
    let Walk {
        from,
        to,
        switch,
        take_jobs,
    } = w;
    let logic = native(move |n, events, out| {
        for e in events {
            if let Event::RoomSwitched { room } = e {
                assert_eq!(*room, name_id("hall"));
            }
        }
        if n == 0 {
            spawn_hero(out, from, -90.0);
        }
        if n == 3 {
            let (entity, target, speed) = (entity(0), to, 1.5);
            send(
                out,
                Command::MoveTo {
                    entity,
                    target,
                    speed,
                },
            );
        }
        if let Some((at, transition)) = switch
            && n == at
        {
            let room = name_id("hall");
            send(
                out,
                Command::RoomSwitch {
                    room,
                    transition,
                    duration: 0.3,
                },
            );
        }
    });
    let mut engine = two_rooms(logic);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut path = Vec::new();
    let mut switched = None;
    for n in 0..300 {
        if let Some(columns) = take_jobs
            && n == 1
        {
            let wanted = engine.wanted_rooms();
            for mut job in engine.world_jobs(&wanted) {
                let mut shares = 1;
                let built = loop {
                    if let Some(built) = job.advance(columns).unwrap() {
                        break built;
                    }
                    shares += 1;
                };
                assert!(
                    columns == usize::MAX || shares > 10,
                    "{shares} shares of {columns} columns"
                );
                engine.install_world(built);
            }
            assert!(
                engine.world_ready(1),
                "the neighbour's world is built off the step"
            );
        }
        engine.step(&actions);
        if engine.state().scenery.active() == 1 && switched.is_none() {
            switched = Some(n);
        }
        let hero = engine.state().world.current_pose(entity(0));
        path.push(hero.map_or(Vec3::ZERO, |p| p.position));
    }
    (path, engine.state_hash(), switched)
}

const ALONG: ([f32; 3], [f32; 3]) = ([0.6, 0.0, -5.0], [5.4, 0.0, -5.0]);
const SWITCH_AT: u64 = 40;

#[test]
fn the_hero_walks_on_into_the_next_room_at_its_speed() {
    let (path, _, switched) = walk(Walk {
        from: ALONG.0,
        to: ALONG.1,
        switch: Some((SWITCH_AT, RoomTransition::Continuous)),
        take_jobs: None,
    });
    assert_eq!(switched, Some(SWITCH_AT));
    let step = |k: usize| path[k + 1].distance(path[k]);
    let at = SWITCH_AT as usize;
    let before = step(at - 4);
    assert!(before > 0.02, "walking: {before} m a step");
    for k in at - 3..at + 4 {
        let ratio = step(k) / before;
        assert!(
            (0.9..1.1).contains(&ratio),
            "step {k} moves {} m against {before} m before the switch",
            step(k)
        );
    }
    let (stay, _, _) = walk(Walk {
        from: ALONG.0,
        to: ALONG.1,
        switch: None,
        take_jobs: None,
    });
    let (end, alone) = (*path.last().unwrap(), *stay.last().unwrap());
    assert!(
        end.distance(alone) < 1e-3,
        "it ends where it would have: {end} {alone}"
    );
    assert!(
        path.iter().all(|p| p.y.abs() < 0.05),
        "on the floor all the way"
    );
}

#[test]
fn the_next_rooms_collision_moves_the_hero() {
    let through = ([3.5, 0.0, -1.2], [3.5, 0.0, -5.0]);
    let run = |switch| {
        walk(Walk {
            from: through.0,
            to: through.1,
            switch,
            take_jobs: None,
        })
        .0
    };
    let crossed = |path: &[Vec3]| path.last().unwrap().z < -4.5;
    assert!(!crossed(&run(None)), "the sill closes the way in its room");
    assert!(
        crossed(&run(Some((1, RoomTransition::Cut)))),
        "the hall leaves it open"
    );
}

#[test]
fn a_world_built_off_the_step_whole_or_in_shares_runs_as_one_built_in_it() {
    let run = |take_jobs| {
        walk(Walk {
            from: ALONG.0,
            to: ALONG.1,
            switch: Some((SWITCH_AT, RoomTransition::Dissolve)),
            take_jobs,
        })
    };
    let (in_step, hash_in_step, _) = run(None);
    for columns in [usize::MAX, 500] {
        let (off_step, hash_off_step, _) = run(Some(columns));
        assert_eq!(in_step, off_step);
        assert_eq!(hash_in_step, hash_off_step);
    }
}

#[test]
fn a_switch_before_the_world_is_built_leaves_its_grids_for_after_the_step() {
    let logic = native(|n, _, out| {
        if n == 0 {
            spawn_hero(out, ALONG.0, -90.0);
        }
        if n == 5 {
            send(
                out,
                Command::RoomSwitch {
                    room: name_id("hall"),
                    transition: RoomTransition::Cut,
                    duration: 0.0,
                },
            );
        }
    });
    let mut engine = two_rooms(logic);
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..10 {
        engine.step(&actions);
    }
    assert_eq!(engine.state().scenery.active(), 1);
    let mut shares = 0;
    while engine.grow_grids(500) {
        shares += 1;
    }
    assert!(
        shares >= 5,
        "the hall's grid is built after the switch: {shares}"
    );
    assert!(!engine.grow_grids(500), "nothing left");
}

#[test]
fn the_rooms_wanted_are_the_active_one_then_its_neighbours() {
    let logic = native(|n, _, out| {
        if n == 1 {
            send(
                out,
                Command::RoomSwitch {
                    room: name_id("hall"),
                    transition: RoomTransition::Continuous,
                    duration: 0.5,
                },
            );
        }
    });
    let mut engine = two_rooms(logic);
    let actions = Actions::for_settings(&engine.state().settings.input);
    assert_eq!(engine.wanted_rooms(), [0, 1]);
    engine.step(&actions);
    engine.step(&actions);
    assert_eq!(
        engine.wanted_rooms(),
        [1, 0],
        "the hall, then the sill it is leaving"
    );
}

#[test]
fn each_rooms_ambience_fades_into_the_next() {
    let logic = native(|n, _, out| {
        if n == 30 {
            send(
                out,
                Command::RoomSwitch {
                    room: name_id("hall"),
                    transition: RoomTransition::Cut,
                    duration: 0.0,
                },
            );
        }
    });
    let mut engine = two_rooms(logic);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut playing = Vec::new();
    for _ in 0..40 {
        engine.step(&actions);
        engine.feed_audio();
        playing.push(engine.sound().unwrap().ambience_room());
    }
    assert_eq!(
        playing[0],
        Some((0, true)),
        "the sill's wind from the start"
    );
    assert_eq!(playing[29], Some((0, true)));
    assert_eq!(
        playing[31],
        Some((1, true)),
        "the hall's theme after the switch"
    );
}

#[test]
fn the_next_rooms_ambience_is_opened_before_the_switch_and_out_of_it() {
    let logic = native(|n, _, out| {
        if n == 30 {
            send(
                out,
                Command::RoomSwitch {
                    room: name_id("hall"),
                    transition: RoomTransition::Continuous,
                    duration: 0.5,
                },
            );
        }
    });
    let mut engine = two_rooms(logic);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut prepared = Vec::new();
    let mut switching = Vec::new();
    for _ in 0..80 {
        engine.step(&actions);
        engine.feed_audio();
        let sound = engine.sound().unwrap();
        prepared.push((sound.ambience_prepared(0), sound.ambience_prepared(1)));
        switching.push(engine.state().scenery.switch().is_some());
    }
    assert_eq!(
        prepared[0],
        (false, false),
        "the first feed plays the sill's wind and opens nothing else"
    );
    assert_eq!(
        prepared[1],
        (false, true),
        "then the hall's theme is opened"
    );
    assert_eq!(prepared[29], (false, true));
    let over = (30..80).find(|&n| !switching[n]).unwrap();
    assert!(over > 50, "the switch lasts: {over}");
    assert!(
        prepared[30..over].iter().all(|&p| p == (false, false)),
        "the switch plays the theme opened and opens nothing: {:?}",
        &prepared[30..over]
    );
    assert_eq!(
        prepared[over + 1],
        (true, false),
        "then the sill's wind, a neighbour now"
    );
}

#[test]
fn an_unknown_ambience_is_a_load_error() {
    let mut assets = assets();
    assets.rooms = rooms();
    assets.rooms[1].ambience = Some("rain".into());
    let sim = Simulation::new(wired(), Vec::new(), native(|_, _, _| {}), 1);
    let error = Engine::new(sim, assets, EngineSettings::default())
        .err()
        .unwrap();
    assert_eq!(error.code, "E_AUDIO_UNKNOWN_SOUND");
    assert!(error.message.contains("rain"), "{}", error.message);
}
