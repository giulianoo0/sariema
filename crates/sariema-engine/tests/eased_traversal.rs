// SPDX-License-Identifier: Apache-2.0

//! A traversal times its distance along its points with its curve: eased,
//! it leaves slower and ends on the same step at the same point as a
//! linear one; linear, it moves as it always has.

mod common;

use glam::Vec3;
use sariema_engine::{Engine, EngineSettings, RoomAssets};
use sariema_format::collision::{Spot, Traversal};
use sariema_format::script::{Command, Ease, Event, name_id};
use sariema_runtime::input::Actions;
use sariema_runtime::sim::Simulation;

use common::{assets, block, collision, entity, native, send, spawn_hero, wired};

const END: [f32; 3] = [3.0, 0.0, 0.0];

/// A floor and a slide from the origin to `END`, timed with `ease`.
fn room(ease: Ease) -> RoomAssets {
    let mut pack = collision(
        "slope",
        vec![block("floor", [-6.0, -0.1, -4.0], [8.0, 0.0, 4.0])],
    );
    pack.spots = vec![Spot {
        name: "top".into(),
        actor: 0,
        position: [0.0; 3],
        yaw: 0.0,
    }];
    pack.traversals = vec![Traversal {
        name: "slide".into(),
        from: 0,
        points: vec![[1.5, 0.0, 0.0], END],
        yaw: 0.0,
        contacts: Vec::new(),
        hold_end: false,
        ease: ease as u8,
    }];
    RoomAssets::collision("slope", pack)
}

/// The hero's root each step, and the step it arrived on, for a slide
/// timed with `ease` sent at step 2.
fn slide(ease: Ease) -> (Vec<Vec3>, usize) {
    let arrived = std::rc::Rc::new(std::cell::Cell::new(None));
    let got = arrived.clone();
    let logic = native(move |n, events, out| {
        if events.iter().any(|e| matches!(e, Event::Arrived { .. })) && got.get().is_none() {
            got.set(Some(n as usize));
        }
        match n {
            0 => spawn_hero(out, [0.0; 3], 0.0),
            2 => send(
                out,
                Command::Traverse {
                    entity: entity(0),
                    traversal: name_id("slide"),
                    duration: 2.0,
                },
            ),
            _ => {}
        }
    });
    let mut assets = assets();
    assets.rooms = vec![room(ease)];
    let sim = Simulation::new(wired(), Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut roots = Vec::new();
    for _ in 0..200 {
        engine.step(&actions);
        roots.push(engine.state().world.node(entity(0)).unwrap().local.position);
    }
    (roots, arrived.get().expect("arrived"))
}

#[test]
fn an_eased_slide_leaves_slower_and_lands_on_the_same_step_as_a_linear_one() {
    let (linear, at) = slide(Ease::Linear);
    let (eased, eased_at) = slide(Ease::InOutCubic);
    assert_eq!(eased_at, at, "the same step");
    let landed = |r: &[Vec3]| r[at - 1];
    assert!(
        landed(&eased).distance(landed(&linear)) < 1e-5,
        "the same point"
    );
    let early = 2 + 20;
    assert!(
        eased[early].x < 0.5 * linear[early].x,
        "eased in: {} against {}",
        eased[early].x,
        linear[early].x
    );
    for w in eased[2..at].windows(2) {
        assert!(w[1].x >= w[0].x, "never back: {} then {}", w[0], w[1]);
    }
}
