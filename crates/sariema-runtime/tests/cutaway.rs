// SPDX-License-Identifier: Apache-2.0

//! A cutaway's line as the step moves it: at the near edge of the room the
//! hero stands in, eased to the next room without a jump, placed at once on
//! the first step and on a camera cut, and hashed only once placed.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_format::scenery::{CutawayIndex, Param, RoomIndex, Target};
use sariema_format::script::{Command, Controller, Ease, Kind, name_id, put_scene_value};
use sariema_format::{Area, CutLine};
use sariema_runtime::scenery::{RoomScene, UNPLACED};
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation};

use common::{entity, native, send};

const OFFSET: f32 = -0.1;

/// A back room and a front room, the camera's side being +Z.
fn house(follow: f32) -> RoomIndex {
    RoomIndex {
        params: vec![Param {
            name: "cutaway.walls.height".into(),
            value: 0.4,
            target: Target::CutawayHeight(0),
        }],
        cutaways: vec![CutawayIndex {
            name: "walls".into(),
            line: CutLine {
                areas: vec![
                    Area {
                        min: [-5.0, -6.0],
                        max: [5.0, 0.5],
                    },
                    Area {
                        min: [-5.0, 0.5],
                        max: [5.0, 4.0],
                    },
                ],
                offset: OFFSET,
                follow,
            },
        }],
        ..RoomIndex::default()
    }
}

type Script = Vec<(u64, Command<'static>)>;

/// A simulation over `room` whose game spawns a hero at the origin when
/// `hero` and sends what `script` holds for each step.
fn simulation(room: RoomIndex, hero: bool, script: Script) -> Simulation<impl Logic> {
    let script = Rc::new(RefCell::new(script));
    let logic = native(move |step, _, out| {
        if step == 0 && hero {
            let entity = entity(0);
            let kind = Kind::Actor;
            send(
                out,
                Command::Spawn {
                    entity,
                    kind,
                    asset: 0,
                },
            );
            send(out, Command::SetHero { entity, speed: 2.0 });
        }
        for (_, c) in script.borrow().iter().filter(|(at, _)| *at == step) {
            send(out, *c);
        }
    });
    let mut sim = Simulation::new(Settings::default(), Vec::new(), logic, 3);
    sim.set_rooms(vec![RoomScene::new("house", room)]);
    sim
}

/// The line after each of `steps` steps.
fn lines(sim: &mut Simulation<impl Logic>, steps: u64) -> Vec<f32> {
    (0..steps)
        .map(|_| {
            sim.step(&Default::default());
            sim.state().scenery.rooms()[0].lines[0]
        })
        .collect()
}

/// The game takes the hero from the player and walks it into the front room.
fn walk_forward() -> Script {
    let target = [0.0, 0.0, 2.0];
    let entity = entity(0);
    vec![
        (
            1,
            Command::Control {
                by: Controller::Script,
            },
        ),
        (
            1,
            Command::MoveTo {
                entity,
                target,
                speed: 1.5,
            },
        ),
    ]
}

#[test]
fn the_line_takes_the_near_edge_of_the_heros_room_and_eases_to_the_next() {
    let follow = 0.25;
    let mut sim = simulation(house(follow), true, walk_forward());
    let seen = lines(&mut sim, 300);
    let back = 0.5 + OFFSET;
    let front = 4.0 + OFFSET;
    assert_eq!(seen[0], back, "placed at once on its first step");
    let entered = seen.iter().position(|&l| l != back).unwrap();
    assert!(
        (15..30).contains(&entered),
        "the hero crosses at z 0.5: {entered}"
    );
    let most = (front - back) * (1.0 - (-1.0f32 / 60.0 / follow).exp()) + 1e-4;
    for w in seen[entered - 1..].windows(2) {
        assert!(w[1] >= w[0] && w[1] - w[0] <= most, "no jump: {w:?}");
    }
    assert!((seen[299] - front).abs() < 1e-3, "{}", seen[299]);
    let mut sim = simulation(house(0.0), true, walk_forward());
    let seen = lines(&mut sim, 60);
    let entered = seen.iter().position(|&l| l != back).unwrap();
    assert_eq!(seen[entered], front, "without follow it moves in the step");
}

#[test]
fn a_camera_cut_moves_the_line_at_once() {
    let rig = Command::RigFixed {
        rig: 1,
        position: [10.0, 5.0, 0.0],
        look_at: [0.0, 0.0, 0.0],
        fov: 40.0,
    };
    let cut = Command::CameraUse {
        rig: 1,
        blend: 0.0,
        ease: Ease::Linear,
    };
    let mut sim = simulation(house(1.0), true, vec![(30, rig), (30, cut)]);
    let seen = lines(&mut sim, 40);
    assert_eq!(seen[29], 0.5 + OFFSET);
    // Seen from +X, the near edge of the back room is its largest x.
    assert!((seen[30] - (5.0 + OFFSET)).abs() < 1e-5, "{}", seen[30]);
}

#[test]
fn the_line_waits_for_a_hero_and_only_a_placed_line_is_hashed() {
    let mut alone = simulation(house(0.25), false, Vec::new());
    assert!(lines(&mut alone, 10).iter().all(|&l| l == UNPLACED));
    let mut bare = simulation(
        RoomIndex {
            cutaways: Vec::new(),
            ..house(0.25)
        },
        false,
        Vec::new(),
    );
    for _ in 0..10 {
        bare.step(&Default::default());
    }
    assert_eq!(alone.state().world_hash(), bare.state().world_hash());
    let mut placed = simulation(house(0.25), true, Vec::new());
    let mut unseen = simulation(
        RoomIndex {
            cutaways: Vec::new(),
            ..house(0.25)
        },
        true,
        Vec::new(),
    );
    lines(&mut placed, 10);
    for _ in 0..10 {
        unseen.step(&Default::default());
    }
    assert_ne!(placed.state().world_hash(), unseen.state().world_hash());
}

#[test]
fn a_cutaways_height_tweens_like_any_scene_parameter() {
    let mut values = Vec::new();
    put_scene_value(&mut values, name_id("cutaway.walls.height"), 3.0);
    let raise = Command::SceneTween {
        tween: 2,
        values: Box::leak(values.into_boxed_slice()),
        duration: 1.0,
        ease: Ease::Linear,
    };
    let mut sim = simulation(house(0.25), true, vec![(1, raise)]);
    lines(&mut sim, 31);
    let height = sim.state().scenery.param(name_id("cutaway.walls.height"));
    assert!((height.unwrap() - 1.7).abs() < 1e-4, "{height:?}");
}
