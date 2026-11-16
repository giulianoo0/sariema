// SPDX-License-Identifier: Apache-2.0

//! The stick curve, acceleration, facing, knockback and speed scale.

mod common;

use common::{Room, actor, hold, settle};
use glam::{Vec2, Vec3};
use sariema_physics::{Intent, Settings, wrap_angle};

const STEP: f32 = 2.1 / 60.0;

fn open_floor(settings: Settings) -> (sariema_physics::World, sariema_physics::ActorId) {
    let mut room = Room::default();
    room.floor(60.0);
    let mut world = room.world(settings);
    let id = actor(&mut world, Vec3::ZERO);
    settle(&mut world, id);
    (world, id)
}

fn travel_per_step(stick: Vec2) -> f32 {
    let (mut world, id) = open_floor(Settings::default());
    let mut events = Vec::new();
    let p = hold(&mut world, id, stick, 3, &mut events);
    (p[2] - p[1]).length()
}

#[test]
fn stick_has_a_dead_zone_and_reaches_full_speed_at_0_7() {
    assert_eq!(travel_per_step(Vec2::new(0.15, 0.0)), 0.0);
    assert!((travel_per_step(Vec2::new(0.0, 0.425)) - STEP * 0.5).abs() < 1e-6);
    assert!((travel_per_step(Vec2::new(-0.7, 0.0)) - STEP).abs() < 1e-6);
    assert!((travel_per_step(Vec2::new(0.6, 0.8)) - STEP).abs() < 1e-6);
}

#[test]
fn full_speed_on_the_first_step_or_after_the_ramp() {
    let (mut world, id) = open_floor(Settings::default());
    let mut events = Vec::new();
    let p = hold(&mut world, id, Vec2::X, 1, &mut events);
    assert!(((p[0] - world.actor(id).previous_position).length() - STEP).abs() < 1e-6);

    let mut settings = Settings::default();
    settings.controller.accel_steps = 3;
    let (mut world, id) = open_floor(settings);
    let start = world.actor(id).position;
    let p = hold(&mut world, id, Vec2::X, 4, &mut events);
    let steps: Vec<f32> = [start, p[0], p[1], p[2], p[3]]
        .windows(2)
        .map(|w| (w[1] - w[0]).length() / STEP)
        .collect();
    for (got, want) in steps.iter().zip([1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0]) {
        assert!((got - want).abs() < 1e-5, "{steps:?}");
    }
}

#[test]
fn facing_eases_along_the_shortest_arc_and_ignores_small_changes() {
    let (mut world, id) = open_floor(Settings::default());
    let mut events = Vec::new();
    // Yaw 0 faces +z; turn to face −x, a quarter turn the short way.
    hold(&mut world, id, Vec2::new(-1.0, 0.0), 1, &mut events);
    let blend = 1.0 - (-14.0f32 / 60.0).exp();
    let want = -std::f32::consts::FRAC_PI_2;
    assert!(
        (world.actor(id).yaw - want * blend).abs() < 1e-4,
        "{}",
        world.actor(id).yaw
    );
    hold(&mut world, id, Vec2::new(-1.0, 0.0), 120, &mut events);
    assert!(wrap_angle(world.actor(id).yaw - want).abs() < 1e-4);
    let a = 0.08f32;
    hold(
        &mut world,
        id,
        Vec2::new(-a.cos(), -a.sin()),
        60,
        &mut events,
    );
    assert!(
        wrap_angle(world.actor(id).yaw - want).abs() < 1e-4,
        "a 0.08 rad change moved the facing"
    );
    let a = 0.3f32;
    hold(
        &mut world,
        id,
        Vec2::new(-a.cos(), -a.sin()),
        120,
        &mut events,
    );
    assert!(
        wrap_angle(world.actor(id).yaw - (want - a)).abs() < 1e-3,
        "{}",
        world.actor(id).yaw
    );
}

#[test]
fn knockback_covers_its_distance_and_speed_scale_scales() {
    let (mut world, id) = open_floor(Settings::default());
    let mut events = Vec::new();
    let start = world.actor(id).position;
    world.apply(Intent::Knockback {
        actor: id,
        push: Vec3::new(0.0, 0.0, 2.2),
        steps: 22,
    });
    let p = hold(&mut world, id, Vec2::X, 22, &mut events);
    let moved = p[21] - start;
    assert!(
        (moved - Vec3::new(0.0, 0.0, 2.2)).length() < 1e-4,
        "stick must not steer a knockback: {moved}"
    );
    let first = (p[0] - start).length();
    let last = (p[21] - p[20]).length();
    assert!(first > 10.0 * last, "fastest first: {first} then {last}");

    world.apply(Intent::SetSpeedScale {
        actor: id,
        scale: 0.5,
    });
    let p = hold(&mut world, id, Vec2::X, 2, &mut events);
    assert!(((p[1] - p[0]).length() - STEP * 0.5).abs() < 1e-6);
}
