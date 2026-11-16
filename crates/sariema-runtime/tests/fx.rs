// SPDX-License-Identifier: Apache-2.0

//! Particles as the step keeps them: emitters started, stopped and burst by
//! the game, a fixed pool each, seeded so the same commands give the same
//! particles, and the sprites a host draws at a frame's `alpha`.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_format::script::{Command, Entity, Kind, PointList, name_id};
use sariema_runtime::fx::{Effect, Shape, Sprite};
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation};

use common::{entity, native, send};

fn sparkle() -> Effect {
    Effect {
        name: "sparkle".into(),
        sprite: Shape::Star,
        rate: 30.0,
        life: [0.5, 0.8],
        size: [0.08, 0.12],
        speed: [0.2, 0.4],
        spread: 40.0,
        radius: 0.1,
        gravity: [0.0, 0.5, 0.0],
        color: vec![
            [0.0, 1.0, 0.9, 0.6, 0.0],
            [0.2, 1.0, 0.9, 0.6, 1.0],
            [1.0, 1.0, 0.6, 0.3, 0.0],
        ],
        intensity: 2.0,
        ..Effect::default()
    }
}

fn dust() -> Effect {
    Effect {
        name: "dust".into(),
        sprite: Shape::Dot,
        pool: Some(8),
        life: [1.0, 1.0],
        size: [0.2, 0.2],
        grow: vec![[0.0, 0.5], [1.0, 1.0]],
        speed: [0.5, 0.5],
        drag: 2.0,
        additive: 0.0,
        ..Effect::default()
    }
}

fn settings(emitters: u32) -> Settings {
    let mut s = Settings::default();
    s.fx.emitters = emitters;
    s.fx.pool = 16;
    s.fx.effects = vec![sparkle(), dust()];
    s
}

type Script = Rc<RefCell<Vec<(u64, Command<'static>)>>>;

fn simulation(
    settings: Settings,
    script: &Script,
    seen: &Rc<RefCell<Vec<String>>>,
) -> Simulation<impl Logic + use<>> {
    let (script, seen) = (script.clone(), seen.clone());
    let logic = native(move |step, events, out| {
        for e in events {
            seen.borrow_mut().push(format!("{e:?}"));
        }
        for (_, c) in script.borrow().iter().filter(|(at, _)| *at == step) {
            send(out, *c);
        }
    });
    Simulation::new(settings, Vec::new(), logic, 7)
}

fn run(
    settings: Settings,
    commands: Vec<(u64, Command<'static>)>,
    steps: u64,
) -> (Vec<String>, Simulation<impl Logic>) {
    let script = Rc::new(RefCell::new(commands));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut sim = simulation(settings, &script, &seen);
    for _ in 0..steps {
        sim.step(&Default::default());
    }
    let seen = seen.borrow().clone();
    (seen, sim)
}

fn start(emitter: u32, effect: &str, position: [f32; 3], seed: u32) -> Command<'static> {
    Command::FxStart {
        emitter,
        effect: name_id(effect),
        entity: Entity::NONE,
        position,
        points: PointList::EMPTY,
        seed,
    }
}

fn sprites<L: Logic>(sim: &Simulation<L>, alpha: f32) -> Vec<Sprite> {
    let mut out = Vec::new();
    sim.state().fx.sprites(alpha, &mut out);
    out
}

fn rejected(seen: &[String], code: &str) -> bool {
    seen.iter()
        .any(|e| e.contains("Rejected") && e.contains(code))
}

#[test]
fn an_unknown_effect_is_refused() {
    let (seen, sim) = run(settings(4), vec![(0, start(1, "smoke", [0.0; 3], 1))], 3);
    assert!(rejected(&seen, "E_FX_UNKNOWN"), "{seen:?}");
    assert_eq!(sim.state().fx.live(), 0);
}

#[test]
fn an_emitter_spawns_at_its_rate_and_keeps_at_most_its_pool() {
    let (_, sim) = run(
        settings(4),
        vec![(0, start(1, "sparkle", [1.0, 0.5, -2.0], 9))],
        31,
    );
    // 30 a second over half a second, all alive (life at least 0.5 s).
    assert_eq!(sim.state().fx.live(), 15);
    let (_, sim) = run(
        settings(4),
        vec![(0, start(1, "sparkle", [1.0, 0.5, -2.0], 9))],
        120,
    );
    assert_eq!(sim.state().fx.live(), 16, "the settings' pool caps it");
    for s in sprites(&sim, 1.0) {
        assert!(s.position.distance(Vec3::new(1.0, 0.5, -2.0)) < 0.1 + 0.4 * 0.8 + 0.5 * 0.8 * 0.8);
        assert_eq!(s.shape, Shape::Star);
        assert!(s.size >= 0.08 && s.size <= 0.12);
        assert_eq!(s.additive, 1.0);
    }
}

#[test]
fn the_same_seed_gives_the_same_particles_and_another_seed_others() {
    let a = run(settings(4), vec![(0, start(1, "sparkle", [0.0; 3], 5))], 40).1;
    let b = run(settings(4), vec![(0, start(1, "sparkle", [0.0; 3], 5))], 40).1;
    let c = run(settings(4), vec![(0, start(1, "sparkle", [0.0; 3], 6))], 40).1;
    assert_eq!(a.state().fx.hash(), b.state().fx.hash());
    assert_eq!(sprites(&a, 0.5), sprites(&b, 0.5));
    assert_ne!(a.state().fx.hash(), c.state().fx.hash());
}

#[test]
fn the_particles_are_pinned_on_every_platform() {
    let (_, sim) = run(settings(4), vec![(0, start(1, "sparkle", [0.0; 3], 5))], 40);
    assert_eq!(sim.state().fx.hash(), 0x9b79_b0b6_f2d5_f1a0);
}

#[test]
fn a_burst_spawns_its_count_at_once_within_the_effects_pool() {
    let burst = |count| Command::FxBurst {
        effect: name_id("dust"),
        entity: Entity::NONE,
        position: [2.0, 0.0, 0.0],
        count,
        seed: 3,
    };
    let (_, sim) = run(settings(4), vec![(0, burst(5))], 1);
    assert_eq!(sim.state().fx.live(), 5);
    let (_, sim) = run(settings(4), vec![(0, burst(50))], 1);
    assert_eq!(sim.state().fx.live(), 8, "the effect's own pool");
    let (_, sim) = run(settings(4), vec![(0, burst(5))], 70);
    assert_eq!(sim.state().fx.live(), 0, "a one-second life");
    assert_eq!(sim.state().fx.emitters(), 0, "a burst frees its slot");
}

#[test]
fn dust_grows_slows_and_covers_what_is_behind() {
    let burst = Command::FxBurst {
        effect: name_id("dust"),
        entity: Entity::NONE,
        position: [0.0; 3],
        count: 1,
        seed: 3,
    };
    let (_, sim) = run(settings(4), vec![(0, burst)], 31);
    let s = sprites(&sim, 1.0)[0];
    assert!((s.size - 0.2 * 0.75).abs() < 0.01, "{}", s.size);
    assert_eq!(s.additive, 0.0);
    // 0.5 m/s losing 2 a second for half a second travels (1 − e⁻¹)/4.
    let travelled = s.position.length();
    assert!((travelled - 0.158).abs() < 0.01, "{travelled}");
}

#[test]
fn a_full_set_of_emitters_refuses_the_next() {
    let (seen, sim) = run(
        settings(2),
        vec![
            (0, start(1, "sparkle", [0.0; 3], 1)),
            (0, start(2, "sparkle", [0.0; 3], 2)),
            (0, start(3, "sparkle", [0.0; 3], 3)),
            (1, start(1, "dust", [0.0; 3], 4)),
        ],
        2,
    );
    assert!(rejected(&seen, "E_FX_FULL"), "{seen:?}");
    assert_eq!(
        sim.state().fx.emitters(),
        2,
        "the same id replaces its emitter"
    );
}

#[test]
fn a_stopped_emitter_fades_its_particles_out_and_frees_its_slot() {
    let (_, sim) = run(
        settings(4),
        vec![
            (0, start(1, "sparkle", [0.0; 3], 1)),
            (
                20,
                Command::FxStop {
                    emitter: 1,
                    fade: 0.25,
                },
            ),
        ],
        27,
    );
    let live = sim.state().fx.live();
    assert!(live > 0);
    let before: f32 = sprites(&sim, 1.0).iter().map(|s| s.color[3]).sum();
    let (_, later) = run(
        settings(4),
        vec![
            (0, start(1, "sparkle", [0.0; 3], 1)),
            (
                20,
                Command::FxStop {
                    emitter: 1,
                    fade: 0.25,
                },
            ),
        ],
        30,
    );
    let after: f32 = sprites(&later, 1.0).iter().map(|s| s.color[3]).sum();
    assert!(after < before, "{after} < {before}");
    assert!(
        later.state().fx.live() <= live,
        "no new particles once stopped"
    );
    let (_, done) = run(
        settings(4),
        vec![
            (0, start(1, "sparkle", [0.0; 3], 1)),
            (
                20,
                Command::FxStop {
                    emitter: 1,
                    fade: 0.25,
                },
            ),
            (
                40,
                Command::FxStop {
                    emitter: 9,
                    fade: 0.0,
                },
            ),
        ],
        41,
    );
    assert_eq!(done.state().fx.live(), 0);
    assert_eq!(done.state().fx.emitters(), 0);
}

#[test]
fn an_emitter_on_an_entity_spawns_where_the_entity_is() {
    let hero = entity(0);
    let mut commands = vec![
        (
            0,
            Command::Spawn {
                entity: hero,
                kind: Kind::Empty,
                asset: 0,
            },
        ),
        (
            0,
            Command::SetTransform {
                entity: hero,
                position: [5.0, 0.0, 0.0],
                yaw: 0.0,
            },
        ),
        (
            1,
            Command::FxStart {
                emitter: 1,
                effect: name_id("sparkle"),
                entity: hero,
                position: [0.0, 1.0, 0.0],
                points: PointList::EMPTY,
                seed: 1,
            },
        ),
    ];
    commands.push((
        10,
        Command::SetTransform {
            entity: hero,
            position: [-5.0, 0.0, 0.0],
            yaw: 0.0,
        },
    ));
    let (_, sim) = run(settings(4), commands.clone(), 10);
    assert!(
        sprites(&sim, 1.0)
            .iter()
            .all(|s| s.position.x > 4.0 && s.position.y > 0.5)
    );
    let (_, sim) = run(settings(4), commands, 16);
    assert!(sprites(&sim, 1.0).iter().any(|s| s.position.x < -4.0));
}

#[test]
fn an_emitter_on_points_spawns_along_them() {
    const PATH: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 0.0, -4.0]];
    let still = Effect {
        name: "still".into(),
        rate: 60.0,
        life: [10.0, 10.0],
        ..Effect::default()
    };
    let mut s = settings(4);
    s.fx.pool = 64;
    s.fx.effects.push(still);
    let (_, sim) = run(
        s,
        vec![(
            0,
            Command::FxStart {
                emitter: 1,
                effect: name_id("still"),
                entity: Entity::NONE,
                position: [0.0; 3],
                points: PointList::Slice(&PATH),
                seed: 2,
            },
        )],
        60,
    );
    let all = sprites(&sim, 1.0);
    assert_eq!(all.len(), 60);
    let on_path = |p: Vec3| {
        (p.z.abs() < 1e-4 && (0.0..=4.0).contains(&p.x))
            || ((p.x - 4.0).abs() < 1e-4 && (-4.0..=0.0).contains(&p.z))
    };
    assert!(all.iter().all(|s| on_path(s.position)));
    let second_leg = all.iter().filter(|s| s.position.z < -0.01).count();
    assert!(
        (20..=40).contains(&second_leg),
        "{second_leg} of 60 on the second half"
    );
}

#[test]
fn too_many_points_are_refused() {
    let mut s = settings(4);
    s.fx.path_points = 2;
    const PATH: [[f32; 3]; 3] = [[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
    let (seen, _) = run(
        s,
        vec![(
            0,
            Command::FxStart {
                emitter: 1,
                effect: name_id("sparkle"),
                entity: Entity::NONE,
                position: [0.0; 3],
                points: PointList::Slice(&PATH),
                seed: 2,
            },
        )],
        2,
    );
    assert!(rejected(&seen, "E_FX_POINTS"), "{seen:?}");
}

#[test]
fn sprites_move_between_steps_with_alpha() {
    let (_, sim) = run(settings(4), vec![(0, start(1, "sparkle", [0.0; 3], 5))], 20);
    let (a, b, mid) = (sprites(&sim, 0.0), sprites(&sim, 1.0), sprites(&sim, 0.5));
    for ((a, b), m) in a.iter().zip(&b).zip(&mid) {
        assert!(m.position.distance(a.position.lerp(b.position, 0.5)) < 1e-5);
    }
}

#[test]
fn the_colour_and_alpha_follow_the_effects_keys() {
    let one = Effect {
        name: "one".into(),
        puff: 1,
        life: [1.0, 1.0],
        color: vec![
            [0.0, 1.0, 0.0, 0.0, 0.0],
            [0.5, 0.0, 1.0, 0.0, 1.0],
            [1.0, 0.0, 0.0, 1.0, 0.0],
        ],
        intensity: 3.0,
        ..Effect::default()
    };
    let mut s = settings(4);
    s.fx.effects.push(one);
    let (_, sim) = run(s, vec![(0, start(1, "one", [0.0; 3], 1))], 16);
    let sprite = sprites(&sim, 1.0)[0];
    // A quarter of its life, halfway between the first two keys.
    assert!((sprite.color[0] - 1.5).abs() < 0.05, "{:?}", sprite.color);
    assert!((sprite.color[1] - 1.5).abs() < 0.05, "{:?}", sprite.color);
    assert!((sprite.color[3] - 0.5).abs() < 0.03, "{:?}", sprite.color);
}

#[test]
fn bad_effects_fail_to_load() {
    for bad in [
        r#"{"fx":{"effects":[{"name":"a","life":[0,1]}]}}"#,
        r#"{"fx":{"effects":[{"name":"a"},{"name":"a"}]}}"#,
        r#"{"fx":{"effects":[{"name":"a","color":[[0.5,1,1,1,1],[0.2,1,1,1,1]]}]}}"#,
        r#"{"fx":{"effects":[{"name":"a","size":[0.2,0.1]}]}}"#,
        r#"{"fx":{"emitters":0}}"#,
    ] {
        let e = sariema_runtime::game::parse_settings(bad).unwrap_err();
        assert_eq!(e.code, "E_SETTINGS", "{bad}: {}", e.message);
    }
    let ok = r#"{"fx":{"effects":[{"name":"a","sprite":"spark","life":[0.2,0.4]}]}}"#;
    assert!(sariema_runtime::game::parse_settings(ok).is_ok());
}
