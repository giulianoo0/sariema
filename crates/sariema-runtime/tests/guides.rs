// SPDX-License-Identifier: Apache-2.0

//! Trails of dots to a goal and highlights in an actor's or a node's own
//! colour, as the step keeps them: eased in and out without a jump, the
//! dots lying still while their start follows someone, the wave running
//! to the goal and the pulse breathing slowly.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use glam::Vec3;
use sariema_format::scenery::{NodeIndex, RoomIndex};
use sariema_format::script::{Command, Entity, Kind, PointList, name_id};
use sariema_runtime::fx::{Glow, GlowTarget, Shape, Sprite};
use sariema_runtime::scenery::RoomScene;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation};

use common::{entity, native, send};

type Script = Rc<RefCell<Vec<(u64, Command<'static>)>>>;

fn room() -> RoomIndex {
    let mut pivot = [0.0; 16];
    for i in [0, 5, 10, 15] {
        pivot[i] = 1.0;
    }
    RoomIndex {
        nodes: vec![NodeIndex {
            name: "desk".into(),
            pivot,
            visible: true,
            hinge: None,
        }],
        ..RoomIndex::default()
    }
}

fn run(
    settings: Settings,
    commands: Vec<(u64, Command<'static>)>,
    steps: u64,
) -> (Vec<String>, Simulation<impl Logic>) {
    let script: Script = Rc::new(RefCell::new(commands));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let (s, v) = (script.clone(), seen.clone());
    let logic = native(move |step, events, out| {
        for e in events {
            v.borrow_mut().push(format!("{e:?}"));
        }
        for (_, c) in s.borrow().iter().filter(|(at, _)| *at == step) {
            send(out, *c);
        }
    });
    let mut sim = Simulation::new(settings, Vec::new(), logic, 7);
    sim.set_rooms(vec![RoomScene::new("study", room())]);
    for _ in 0..steps {
        sim.step(&Default::default());
    }
    let seen = seen.borrow().clone();
    (seen, sim)
}

fn rejected(seen: &[String], code: &str) -> bool {
    seen.iter()
        .any(|e| e.contains("Rejected") && e.contains(code))
}

const LINE: [[f32; 3]; 2] = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0]];
const AMBER: [f32; 3] = [1.0, 0.6, 0.2];

fn trail(points: &'static [[f32; 3]], from: Entity, fade: f32) -> Command<'static> {
    Command::Trail {
        trail: 1,
        from,
        points: PointList::Slice(points),
        spacing: 1.0,
        size: 0.2,
        color: AMBER,
        intensity: 0.55,
        wave_speed: 0.0,
        fade_from: 0.0,
        fade_to: 0.0,
        fade,
    }
}

fn with(c: Command<'static>, f: impl FnOnce(&mut Command<'static>)) -> Command<'static> {
    let mut c = c;
    f(&mut c);
    c
}

fn dots<L: Logic>(sim: &Simulation<L>, alpha: f32) -> Vec<Sprite> {
    let mut out = Vec::new();
    sim.state().fx.sprites(alpha, &mut out);
    out
}

fn glows<L: Logic>(sim: &Simulation<L>, alpha: f32) -> Vec<Glow> {
    sim.state().fx.glows(alpha).collect()
}

fn spawn_at(hero: Entity, x: f32) -> [Command<'static>; 2] {
    [
        Command::Spawn {
            entity: hero,
            kind: Kind::Empty,
            asset: 0,
        },
        at(hero, x),
    ]
}

fn at(hero: Entity, x: f32) -> Command<'static> {
    Command::SetTransform {
        entity: hero,
        position: [x, 0.0, 0.0],
        yaw: 0.0,
    }
}

#[test]
fn a_trail_lays_soft_flat_dots_back_from_its_goal_at_its_spacing() {
    let (seen, sim) = run(
        Settings::default(),
        vec![(0, trail(&LINE, Entity::NONE, 0.0))],
        2,
    );
    assert!(seen.iter().all(|e| !e.contains("Rejected")), "{seen:?}");
    let dots = dots(&sim, 1.0);
    let xs: Vec<f32> = dots.iter().map(|d| d.position.x).collect();
    assert_eq!(xs, [4.0, 3.0, 2.0, 1.0, 0.0]);
    let lift = Settings::default().fx.trail.lift;
    for d in &dots {
        assert_eq!(d.position.y, lift);
        assert!(d.flat);
        assert_eq!(d.shape, Shape::Disc);
        assert_eq!(d.additive, 1.0);
        assert_eq!(d.color[3], 1.0);
    }
    // The still wave's crest sits on the goal: twice as bright and a quarter
    // larger there, e⁻² of that a metre away.
    assert!((dots[0].color[0] - 2.0 * 0.55).abs() < 1e-5);
    assert!((dots[0].size - 0.25).abs() < 1e-5);
    let one_metre = 0.55 * (1.0 + (-2.0f32).exp());
    assert!(
        (dots[1].color[0] - one_metre).abs() < 1e-4,
        "{}",
        dots[1].color[0]
    );
    assert!(
        (dots[1].color[1] / dots[1].color[0] - 0.6).abs() < 1e-5,
        "amber stays amber"
    );
}

#[test]
fn the_dots_fade_out_near_the_start_and_near_the_goal() {
    let soft = with(trail(&LINE, Entity::NONE, 0.0), |c| {
        if let Command::Trail {
            spacing,
            fade_from,
            fade_to,
            ..
        } = c
        {
            (*spacing, *fade_from, *fade_to) = (0.5, 1.0, 1.0);
        }
    });
    let (_, sim) = run(Settings::default(), vec![(0, soft)], 2);
    let alpha: Vec<f32> = dots(&sim, 1.0).iter().map(|d| d.color[3]).collect();
    assert_eq!(alpha.len(), 9);
    assert_eq!(alpha[0], 0.0, "on the goal");
    assert!((alpha[1] - 0.5).abs() < 1e-5, "half a metre from it");
    assert_eq!(&alpha[2..7], &[1.0; 5]);
    assert!((alpha[7] - 0.5).abs() < 1e-5);
    assert_eq!(alpha[8], 0.0, "on the start");
}

#[test]
fn the_wave_runs_towards_the_goal_smoothly_between_steps() {
    const LONG: [[f32; 3]; 2] = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]];
    let moving = with(trail(&LONG, Entity::NONE, 0.0), |c| {
        if let Command::Trail {
            spacing,
            wave_speed,
            ..
        } = c
        {
            (*spacing, *wave_speed) = (0.25, 2.0);
        }
    });
    let crest = |sim: &Simulation<_>, alpha| {
        let dots = dots(sim, alpha);
        let top = dots
            .iter()
            .filter(|d| d.position.x < 9.0)
            .max_by(|a, b| a.color[0].total_cmp(&b.color[0]))
            .unwrap();
        10.0 - top.position.x
    };
    // At 2 m/s a crest that left the goal's far side 8 m back is 7 m from
    // the goal after half a second and 6.5 m after three quarters.
    let (_, sim) = run(Settings::default(), vec![(0, moving)], 30);
    assert!(
        (crest(&sim, 1.0) - 7.0).abs() <= 0.125,
        "{}",
        crest(&sim, 1.0)
    );
    let (_, later) = run(Settings::default(), vec![(0, moving)], 45);
    assert!(
        (crest(&later, 1.0) - 6.5).abs() <= 0.125,
        "{}",
        crest(&later, 1.0)
    );
    let (a, b) = (dots(&later, 0.0), dots(&later, 1.0));
    let mid = dots(&later, 0.5);
    for ((a, b), m) in a.iter().zip(&b).zip(&mid) {
        assert_eq!(a.position, m.position, "the dots never move");
        let between = (a.color[0] + b.color[0]) / 2.0;
        assert!(
            (m.color[0] - between).abs() < 0.02,
            "the glow eases in between"
        );
    }
}

#[test]
fn the_dots_stay_put_while_the_start_follows_someone_walking() {
    const GOAL: [[f32; 3]; 1] = [[4.0, 0.0, 0.0]];
    let hero = entity(0);
    let [spawn, place] = spawn_at(hero, 0.0);
    let script = vec![
        (0, spawn),
        (0, place),
        (1, trail(&GOAL, hero, 0.0)),
        (10, at(hero, 1.5)),
    ];
    let (_, before) = run(Settings::default(), script.clone(), 10);
    let xs =
        |sim: &Simulation<_>| -> Vec<f32> { dots(sim, 1.0).iter().map(|d| d.position.x).collect() };
    assert_eq!(xs(&before), [4.0, 3.0, 2.0, 1.0, 0.0]);
    let (_, after) = run(Settings::default(), script, 12);
    assert_eq!(
        xs(&after),
        [4.0, 3.0, 2.0],
        "the dots behind the hero are gone, the rest unmoved"
    );
}

#[test]
fn a_trail_fades_in_and_out_without_a_jump_and_is_freed() {
    let script = vec![
        (0, trail(&LINE, Entity::NONE, 0.5)),
        (
            40,
            Command::TrailOff {
                trail: 1,
                fade: 1.0,
            },
        ),
        (55, trail(&LINE, Entity::NONE, 0.5)),
        (
            100,
            Command::TrailOff {
                trail: 1,
                fade: 0.5,
            },
        ),
    ];
    let shown = |steps| {
        let (_, sim) = run(Settings::default(), script.clone(), steps);
        let d = dots(&sim, 1.0);
        (
            d.first().map_or(0.0, |d| d.color[3]),
            sim.state().fx.trails(),
        )
    };
    assert!((shown(15).0 - 0.5).abs() < 0.04, "{}", shown(15).0);
    assert_eq!(shown(35).0, 1.0);
    let (fading, _) = shown(55);
    assert!((fading - 0.75).abs() < 0.04, "{fading}");
    let (back, _) = shown(56);
    assert!(
        (back - fading).abs() < 0.04,
        "taken back from {fading}, not from 0: {back}"
    );
    assert_eq!(shown(100).0, 1.0);
    assert_eq!(shown(131), (0.0, 0), "faded out and freed");
}

#[test]
fn a_trail_with_too_many_points_or_one_too_many_is_refused() {
    let mut few = Settings::default();
    few.fx.trail_points = 1;
    let (seen, sim) = run(few, vec![(0, trail(&LINE, Entity::NONE, 0.0))], 2);
    assert!(rejected(&seen, "E_FX_POINTS"), "{seen:?}");
    assert_eq!(sim.state().fx.trails(), 0);
    let mut one = Settings::default();
    one.fx.trails = 1;
    let other = with(trail(&LINE, Entity::NONE, 0.0), |c| {
        if let Command::Trail { trail, .. } = c {
            *trail = 2;
        }
    });
    let script = vec![
        (0, trail(&LINE, Entity::NONE, 0.0)),
        (0, trail(&LINE, Entity::NONE, 0.0)),
        (1, other),
    ];
    let (seen, sim) = run(one, script, 3);
    assert!(rejected(&seen, "E_FX_FULL"), "{seen:?}");
    assert_eq!(
        sim.state().fx.trails(),
        1,
        "the same id takes the new route"
    );
}

#[test]
fn a_trail_draws_at_most_its_dots() {
    let mut few = Settings::default();
    few.fx.trail_dots = 3;
    let (_, sim) = run(few, vec![(0, trail(&LINE, Entity::NONE, 0.0))], 2);
    let xs: Vec<f32> = dots(&sim, 1.0).iter().map(|d| d.position.x).collect();
    assert_eq!(xs, [4.0, 3.0, 2.0], "the ones nearest the goal");
}

fn highlight(
    entity: Entity,
    node: &str,
    (fill, rim): (f32, f32),
    pulse: f32,
    fade: f32,
) -> Command<'static> {
    Command::Highlight {
        entity,
        node: name_id(node),
        fill,
        rim,
        pulse,
        fade,
    }
}

#[test]
fn a_highlight_eases_in_holds_and_eases_out_without_a_jump() {
    let hero = entity(0);
    let [spawn, place] = spawn_at(hero, 0.0);
    let script = vec![
        (0, spawn),
        (0, place),
        (1, highlight(hero, "", (0.5, 1.0), 0.0, 0.5)),
        (60, highlight(hero, "", (0.0, 0.0), 0.0, 0.5)),
        (75, highlight(hero, "", (0.5, 1.0), 0.0, 0.5)),
        (120, highlight(hero, "", (0.0, 0.0), 0.0, 0.25)),
    ];
    let lit = |steps| {
        let (_, sim) = run(Settings::default(), script.clone(), steps);
        (glows(&sim, 1.0), sim.state().fx.highlights())
    };
    let (half, _) = lit(16);
    assert_eq!(half[0].target, GlowTarget::Entity(hero));
    assert!(
        (half[0].fill - 0.25).abs() < 0.02 && (half[0].rim - 0.5).abs() < 0.04,
        "{half:?}"
    );
    assert_eq!(lit(40).0[0].fill, 0.5);
    let (going, _) = lit(75);
    assert!((going[0].fill - 0.25).abs() < 0.02, "{going:?}");
    let (back, _) = lit(76);
    assert!(
        (back[0].fill - going[0].fill).abs() < 0.02,
        "no jump: {going:?} then {back:?}"
    );
    assert_eq!(lit(140), (Vec::new(), 0), "faded out and freed");
}

#[test]
fn a_pulse_breathes_slowly_between_full_and_its_depth() {
    let hero = entity(0);
    let [spawn, place] = spawn_at(hero, 0.0);
    let script = vec![
        (0, spawn),
        (0, place),
        (1, highlight(hero, "", (1.0, 1.0), 2.0, 0.0)),
    ];
    let (_, sim) = run(Settings::default(), script.clone(), 2);
    let mut sim = sim;
    let (mut low, mut high, mut last) = (f32::MAX, f32::MIN, None::<f32>);
    for _ in 0..120 {
        sim.step(&Default::default());
        for alpha in [0.25, 0.5, 0.75, 1.0] {
            let fill = glows(&sim, alpha)[0].fill;
            low = low.min(fill);
            high = high.max(fill);
            if let Some(last) = last {
                assert!((fill - last).abs() < 0.01, "smooth: {last} then {fill}");
            }
            last = Some(fill);
        }
    }
    let depth = Settings::default().fx.pulse_depth;
    assert!((high - 1.0).abs() < 1e-3, "{high}");
    assert!((low - (1.0 - depth)).abs() < 1e-3, "{low}");
}

#[test]
fn a_room_node_glows_by_its_name_and_an_unknown_one_is_refused() {
    let script = vec![
        (0, highlight(Entity::NONE, "desk", (0.3, 0.8), 0.0, 0.0)),
        (0, highlight(Entity::NONE, "chair", (0.3, 0.8), 0.0, 0.0)),
    ];
    let (seen, sim) = run(Settings::default(), script, 2);
    assert!(rejected(&seen, "E_SCENE_UNKNOWN"), "{seen:?}");
    let lit = glows(&sim, 1.0);
    assert_eq!(lit.len(), 1);
    assert_eq!(lit[0].target, GlowTarget::Node { room: 0, node: 0 });
    assert_eq!((lit[0].fill, lit[0].rim), (0.3, 0.8));
}

#[test]
fn a_dead_entity_or_one_too_many_highlights_is_refused() {
    let (seen, _) = run(
        Settings::default(),
        vec![(0, highlight(entity(3), "", (0.3, 0.8), 0.0, 0.0))],
        2,
    );
    assert!(rejected(&seen, "E_ENTITY_STALE"), "{seen:?}");
    let mut one = Settings::default();
    one.fx.highlights = 1;
    let hero = entity(0);
    let [spawn, place] = spawn_at(hero, 0.0);
    let script = vec![
        (0, spawn),
        (0, place),
        (1, highlight(hero, "", (0.3, 0.8), 0.0, 0.0)),
        (1, highlight(Entity::NONE, "desk", (0.3, 0.8), 0.0, 0.0)),
        (1, highlight(Entity::NONE, "desk", (0.0, 0.0), 0.0, 0.0)),
    ];
    let (seen, sim) = run(one, script, 3);
    assert!(rejected(&seen, "E_FX_FULL"), "{seen:?}");
    assert_eq!(
        sim.state().fx.highlights(),
        1,
        "turning off one never lit is no error"
    );
}

#[test]
fn trails_and_highlights_change_the_effects_hash_and_never_the_engines() {
    let hero = entity(0);
    let [spawn, place] = spawn_at(hero, 0.0);
    let plain = vec![(0, spawn), (0, place)];
    let mut guided = plain.clone();
    guided.push((1, trail(&LINE, hero, 0.5)));
    guided.push((1, highlight(hero, "", (0.3, 0.8), 2.0, 0.5)));
    let (_, a) = run(Settings::default(), plain, 30);
    let (_, b) = run(Settings::default(), guided.clone(), 30);
    let (_, c) = run(Settings::default(), guided, 30);
    assert_eq!(a.state_hash(), b.state_hash());
    assert_ne!(a.state().fx.hash(), b.state().fx.hash());
    assert_eq!(b.state().fx.hash(), c.state().fx.hash());
    let ends = dots(&b, 1.0);
    assert!(
        ends.iter()
            .all(|d| d.position.distance(Vec3::new(d.position.x, 0.02, 0.0)) < 1e-6)
    );
}

#[test]
fn a_room_node_entity_glows_as_its_node_and_a_glow_ends_with_its_entity() {
    let (desk, hero) = (entity(1), entity(0));
    let [spawn, place] = spawn_at(hero, 0.0);
    let script = vec![
        (0, spawn),
        (0, place),
        (
            0,
            Command::Spawn {
                entity: desk,
                kind: Kind::RoomNode,
                asset: name_id("desk"),
            },
        ),
        (1, highlight(desk, "", (0.3, 0.8), 0.0, 0.0)),
        (1, highlight(hero, "", (0.3, 0.8), 0.0, 0.0)),
        (5, Command::Despawn { entity: hero }),
    ];
    let (_, sim) = run(Settings::default(), script.clone(), 3);
    let targets: Vec<GlowTarget> = glows(&sim, 1.0).iter().map(|g| g.target).collect();
    assert_eq!(
        targets,
        [
            GlowTarget::Node { room: 0, node: 0 },
            GlowTarget::Entity(hero)
        ]
    );
    let (_, sim) = run(Settings::default(), script, 7);
    let targets: Vec<GlowTarget> = glows(&sim, 1.0).iter().map(|g| g.target).collect();
    assert_eq!(targets, [GlowTarget::Node { room: 0, node: 0 }]);
}
