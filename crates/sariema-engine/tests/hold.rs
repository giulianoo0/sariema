// SPDX-License-Identifier: Apache-2.0

//! Something held in an actor's hand: taken where it stands, it keeps its
//! place on the socket while the clip swings the arm; snapped, it sits at
//! its offset from the socket in metres of the world, whatever the actor's
//! scale; let go, it stays where it was.

mod common;

use glam::{Quat, Vec3};
use sariema_format::script::{ClipMode, Command, Entity, Event, Kind, name_id};
use sariema_runtime::input::Actions;

use common::{engine, entity, native, send, spawn_hero, wired};

const ITEM: u32 = 3;
const AT: [f32; 3] = [1.9, 0.8, -0.9];
const SCALE: f32 = 2.25;

fn hold(actor: Entity, socket: &str, offset: [f32; 3], keep: bool) -> Command<'static> {
    Command::Hold {
        entity: entity(ITEM),
        actor,
        socket: name_id(socket),
        offset,
        keep,
    }
}

struct Step {
    item: Vec3,
    item_rotation: Quat,
    /// The socket in the world, `None` while the hero has no animator.
    socket: Option<(Vec3, Quat)>,
}

struct Run {
    steps: Vec<Step>,
    events: Vec<(u64, String)>,
}

/// The hero, scaled and waving, and an item at `AT` turned 30°; `script`
/// at the steps it names.
fn run(script: fn(u64, &mut Vec<u8>), steps: u64) -> Run {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let log = events.clone();
    let logic = native(move |n, seen, out| {
        log.borrow_mut()
            .extend(seen.iter().map(|e| (n, format!("{e:?}"))));
        if seen.iter().any(|e| matches!(e, Event::Started { .. })) {
            spawn_hero(out, [1.5, 0.0, -0.6], 20.0);
            send(
                out,
                Command::SetScale {
                    entity: entity(0),
                    scale: SCALE,
                },
            );
            send(
                out,
                Command::Spawn {
                    entity: entity(ITEM),
                    kind: Kind::Empty,
                    asset: 0,
                },
            );
            send(
                out,
                Command::SetTransform {
                    entity: entity(ITEM),
                    position: AT,
                    yaw: 30.0,
                },
            );
        }
        if n == 2 {
            send(
                out,
                Command::PlayClip {
                    entity: entity(0),
                    clip: name_id("wave"),
                    mode: ClipMode::Loop,
                    blend: 0.0,
                    speed: 1.0,
                },
            );
        }
        script(n, out);
    });
    let mut engine = engine(logic, wired(), &[]);
    let actions = Actions::for_settings(&engine.state().settings.input);
    let mut trace = Vec::new();
    for _ in 0..steps {
        engine.step(&actions);
        let world = &engine.state().world;
        let item = world.pose(entity(ITEM)).unwrap();
        let socket = (engine.animation().socket(entity(0), name_id("tip")))
            .zip(world.pose(entity(0)))
            .map(|(m, hero)| {
                let (_, r, p) = m.to_scale_rotation_translation();
                (
                    hero.position + hero.rotation * (p * hero.scale),
                    (hero.rotation * r).normalize(),
                )
            });
        trace.push(Step {
            item: item.position,
            item_rotation: item.rotation,
            socket,
        });
    }
    let events = events.take();
    Run {
        steps: trace,
        events,
    }
}

fn in_socket(s: &Step) -> (Vec3, Quat) {
    let (at, turn) = s.socket.unwrap();
    (
        turn.inverse() * (s.item - at),
        turn.inverse() * s.item_rotation,
    )
}

#[test]
fn taken_where_it_stands_it_keeps_its_place_on_the_socket_and_stays_where_let_go() {
    let r = run(
        |n, out| match n {
            20 => send(out, hold(entity(0), "tip", [0.0; 3], true)),
            100 => send(out, hold(Entity::NONE, "", [0.0; 3], false)),
            _ => {}
        },
        140,
    );
    let s = &r.steps;
    assert!(
        s[20].item.distance(Vec3::from(AT)) < 1e-5,
        "taken where it stands: {}",
        s[20].item
    );
    let (offset, turn) = in_socket(&s[20]);
    for (k, step) in s.iter().enumerate().take(100).skip(21) {
        let (o, t) = in_socket(step);
        assert!(
            o.distance(offset) < 1e-4 && t.angle_between(turn) < 1e-3,
            "step {k}: {o} {t} on the socket, against {offset} {turn}"
        );
    }
    let swing = (s[21..100].iter())
        .map(|x| x.item.distance(s[20].item))
        .fold(0.0, f32::max);
    assert!(swing > 0.05, "the wave carries it: {swing} m");
    let let_go = s[99].item;
    assert!(
        s[100..].iter().all(|x| x.item.distance(let_go) < 1e-5),
        "let go, it stays where it was"
    );
}

#[test]
fn snapped_it_sits_at_its_offset_in_metres_of_the_world() {
    let r = run(
        |n, out| {
            if n == 20 {
                send(out, hold(entity(0), "tip", [0.0, -0.04, 0.0], false))
            }
        },
        80,
    );
    for (k, step) in r.steps.iter().enumerate().skip(20) {
        let (o, t) = in_socket(step);
        assert!(
            o.distance(Vec3::new(0.0, -0.04, 0.0)) < 1e-4 && t.angle_between(Quat::IDENTITY) < 1e-3,
            "step {k}: {o} {t}, not 4 cm under the socket at scale {SCALE}"
        );
    }
}

#[test]
fn an_unknown_socket_or_an_actor_to_hold_is_refused_and_a_despawned_holder_lets_go() {
    let r = run(
        |n, out| match n {
            10 => send(out, hold(entity(0), "tail", [0.0; 3], true)),
            12 => send(
                out,
                Command::Hold {
                    entity: entity(0),
                    actor: entity(0),
                    socket: name_id("tip"),
                    offset: [0.0; 3],
                    keep: true,
                },
            ),
            20 => send(out, hold(entity(0), "tip", [0.0; 3], true)),
            60 => send(out, Command::Despawn { entity: entity(0) }),
            _ => {}
        },
        90,
    );
    let at = |code: &str| -> Vec<u64> {
        (r.events.iter())
            .filter(|(_, e)| e.contains(code))
            .map(|(n, _)| *n)
            .collect()
    };
    assert_eq!(at("E_SOCKET_UNKNOWN"), [11]);
    assert_eq!(at("E_HOLD_ACTOR"), [13]);
    assert!(
        r.steps[10..20]
            .iter()
            .all(|x| x.item.distance(Vec3::from(AT)) < 1e-5),
        "an unknown socket leaves it where it stands"
    );
    let dropped = r.steps[59].item;
    assert!(
        r.steps[60..]
            .iter()
            .all(|x| x.item.distance(dropped) < 1e-5),
        "its holder gone, it stays where it was"
    );
}
