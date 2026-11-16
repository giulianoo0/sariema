// SPDX-License-Identifier: Apache-2.0

use glam::Vec3;
use sariema_format::script::{Ease, Property};
use sariema_runtime::follow::Followers;
use sariema_runtime::store::Store;
use sariema_runtime::timer::Timers;
use sariema_runtime::tween::{Target, Tweens, read};
use sariema_runtime::world::{Entity, Kind, World};

const REST: f32 = 1e-3;

fn world_with(n: u32) -> (World, Vec<Entity>) {
    let mut w = World::new(64);
    let es: Vec<Entity> = (0..n)
        .map(|i| Entity {
            index: i,
            generation: 0,
        })
        .collect();
    for &e in &es {
        w.spawn(e, Kind::Empty, 0).unwrap();
    }
    (w, es)
}

fn x(entity: Entity) -> Target {
    Target {
        entity,
        property: Property::PositionX,
        var: 0,
    }
}

fn step(t: &mut Tweens, w: &mut World, s: &mut Store) -> Vec<u32> {
    let mut done = Vec::new();
    t.step(w, s, REST, |id| done.push(id));
    done
}

#[test]
fn a_tween_lands_exactly_on_its_end_value_on_its_last_step() {
    let (mut w, es) = world_with(1);
    let mut s = Store::default();
    let mut t = Tweens::default();
    t.tween(&w, &s, 1, x(es[0]), 3.0, 0.5, Ease::InOutCubic)
        .unwrap();
    for n in 1..30 {
        assert!(step(&mut t, &mut w, &mut s).is_empty(), "done early at {n}");
        assert!(read(&w, &s, &x(es[0])).unwrap() < 3.0);
    }
    assert_eq!(step(&mut t, &mut w, &mut s), [1]);
    assert_eq!(read(&w, &s, &x(es[0])).unwrap(), 3.0);
    assert!(t.is_empty());
    assert!(step(&mut t, &mut w, &mut s).is_empty());
}

#[test]
fn a_new_tween_on_the_same_property_continues_from_where_the_old_one_was() {
    let (mut w, es) = world_with(1);
    let mut s = Store::default();
    let mut t = Tweens::default();
    t.tween(&w, &s, 1, x(es[0]), 10.0, 1.0, Ease::Linear)
        .unwrap();
    let mut last = 0.0;
    for _ in 0..20 {
        step(&mut t, &mut w, &mut s);
        last = read(&w, &s, &x(es[0])).unwrap();
    }
    t.tween(&w, &s, 2, x(es[0]), 0.0, 1.0, Ease::InOutSine)
        .unwrap();
    assert_eq!(t.len(), 1, "the old tween is replaced");
    step(&mut t, &mut w, &mut s);
    let now = read(&w, &s, &x(es[0])).unwrap();
    assert!(
        (now - last).abs() < 10.0 / 60.0,
        "jumped from {last} to {now}"
    );
}

fn settle_steps(frequency: f32) -> usize {
    // Critically damped from rest: x = (1 + ωt)e^(−ωt), v = −ω²t·e^(−ωt).
    let omega = std::f32::consts::TAU * frequency;
    (1..10_000)
        .find(|&n| {
            let t = n as f32 / 60.0;
            let e = (-omega * t).exp();
            (1.0 + omega * t) * e <= REST && omega * omega * t * e <= REST
        })
        .unwrap()
}

#[test]
fn a_critical_spring_settles_within_the_analytic_bound_without_overshoot() {
    let (mut w, es) = world_with(1);
    let mut s = Store::default();
    let mut t = Tweens::default();
    w.node_mut(es[0]).unwrap().local.position.x = 1.0;
    t.spring(&w, &s, 5, x(es[0]), 0.0, 2.0, 1.0).unwrap();
    let bound = settle_steps(2.0);
    let mut settled = None;
    for n in 1..=bound + 2 {
        let done = step(&mut t, &mut w, &mut s);
        assert!(read(&w, &s, &x(es[0])).unwrap() >= 0.0, "overshot at {n}");
        if done == [5] {
            settled = Some(n);
            break;
        }
    }
    let settled = settled.expect("did not settle");
    assert!(settled <= bound + 1, "settled at {settled}, bound {bound}");
    assert!(
        settled + 6 >= bound,
        "settled at {settled}, too early for {bound}"
    );
    assert_eq!(read(&w, &s, &x(es[0])).unwrap(), 0.0);
}

#[test]
fn a_spring_retargeted_mid_flight_keeps_its_velocity() {
    let (mut w, es) = world_with(1);
    let mut s = Store::default();
    let mut t = Tweens::default();
    t.spring(&w, &s, 1, x(es[0]), 5.0, 1.5, 0.7).unwrap();
    let mut xs = vec![0.0];
    for _ in 0..10 {
        step(&mut t, &mut w, &mut s);
        xs.push(read(&w, &s, &x(es[0])).unwrap());
    }
    t.spring(&w, &s, 1, x(es[0]), -2.0, 1.5, 0.7).unwrap();
    step(&mut t, &mut w, &mut s);
    xs.push(read(&w, &s, &x(es[0])).unwrap());
    let n = xs.len();
    let before = xs[n - 2] - xs[n - 3];
    let after = xs[n - 1] - xs[n - 2];
    assert!(after > 0.0, "kept moving the old way for a step");
    assert!(
        (after - before).abs() < before.abs() * 0.5 + 1e-3,
        "{before} then {after}"
    );
}

#[test]
fn a_stiff_spring_stays_stable() {
    let (mut w, es) = world_with(1);
    let mut s = Store::default();
    let mut t = Tweens::default();
    t.spring(&w, &s, 1, x(es[0]), 1.0, 80.0, 0.05).unwrap();
    for _ in 0..600 {
        step(&mut t, &mut w, &mut s);
        let v = read(&w, &s, &x(es[0])).unwrap();
        assert!(v.is_finite() && v.abs() <= 2.0, "{v}");
    }
}

#[test]
fn tweens_drive_variables_and_drop_dead_entities_silently() {
    let (mut w, es) = world_with(1);
    let mut s = Store::default();
    let mut t = Tweens::default();
    let var = Target {
        entity: Entity::NONE,
        property: Property::Var,
        var: 9,
    };
    t.tween(&w, &s, 1, var, 2.0, 0.1, Ease::Linear).unwrap();
    t.tween(&w, &s, 2, x(es[0]), 2.0, 0.1, Ease::Linear)
        .unwrap();
    w.despawn(es[0]).unwrap();
    let mut done = Vec::new();
    for _ in 0..6 {
        done.extend(step(&mut t, &mut w, &mut s));
    }
    assert_eq!(done, [1]);
    assert_eq!(s.var(9), 2.0);
    assert!(t.is_empty());
}

#[test]
fn followers_are_exact_when_rigid_and_halve_the_gap_per_half_life() {
    let (mut w, es) = world_with(4);
    let mut f = Followers::default();
    w.node_mut(es[0]).unwrap().local.position = Vec3::new(4.0, 0.0, 0.0);
    f.follow(&w, es[1], es[0], Vec3::new(0.0, 1.0, 0.0), 0.0)
        .unwrap();
    f.follow(&w, es[2], es[1], Vec3::ZERO, 0.0).unwrap();
    f.step(&mut w);
    assert_eq!(
        w.current_pose(es[2]).unwrap().position,
        Vec3::new(4.0, 1.0, 0.0),
        "a chain settles in one step"
    );

    f.follow(&w, es[3], es[0], Vec3::ZERO, 0.5).unwrap();
    for _ in 0..30 {
        f.step(&mut w);
    }
    let gap = 4.0 - w.current_pose(es[3]).unwrap().position.x;
    assert!((gap - 2.0).abs() < 1e-3, "gap after one half-life is {gap}");

    f.follow(&w, es[3], es[1], Vec3::ZERO, 0.5).unwrap();
    let before = w.current_pose(es[3]).unwrap().position;
    f.step(&mut w);
    assert!(
        w.current_pose(es[3]).unwrap().position.distance(before) < 0.2,
        "retargeting jumped"
    );
}

#[test]
fn timers_fire_on_the_exact_step() {
    let mut timers = Timers::default();
    timers.set(1, 1.0, false);
    timers.set(2, 0.5, true);
    let mut fired = Vec::new();
    for step in 1..=120u32 {
        timers.step(|id| fired.push((step, id)));
    }
    assert_eq!(fired, [(30, 2), (60, 1), (60, 2), (90, 2), (120, 2)]);
    timers.cancel(2);
    assert!(timers.is_empty());
}
