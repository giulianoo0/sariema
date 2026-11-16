// SPDX-License-Identifier: Apache-2.0

mod common;

use common::{Owned, Room, actor, actor_with, hold, settle, stick_at};
use glam::{Vec2, Vec3};
use sariema_physics::{ActorCollider, EventKind, Settings};

const SPEED: f32 = 2.1;
const DT: f32 = 1.0 / 60.0;

/// The speed the default settings keep along a wall for input at `approach`
/// degrees to it.
fn expected_slide(approach_deg: f32) -> f32 {
    let kept = approach_deg.to_radians().cos();
    let head_on = 25f32.to_radians().sin();
    let full = 45f32.to_radians().cos();
    if kept < head_on {
        return kept;
    }
    kept.max(0.85 * ((kept - head_on) / (full - head_on)).clamp(0.0, 1.0))
}

/// From the step that first ends touching a wall (closer than `touching`
/// along `normal` to the origin), or from the start when the actor only walks
/// alongside: the velocity along the wall never changes sign or stops, and the
/// actor never moves away from the wall.
fn steady_from_contact(positions: &[Vec3], along: Vec3, normal: Vec3, touching: f32) {
    let first = positions
        .iter()
        .position(|p| p.dot(normal) < touching)
        .unwrap_or(0);
    let steps: Vec<Vec3> = positions[first.saturating_sub(1)..]
        .windows(2)
        .map(|w| w[1] - w[0])
        .collect();
    let sign = steps[0].dot(along).signum();
    for (k, d) in steps.iter().enumerate() {
        let (tangent, away) = (d.dot(along), d.dot(normal));
        assert!(
            tangent != 0.0 && tangent.signum() == sign,
            "step {k} after contact: {tangent} along the wall"
        );
        assert!(
            away < 1e-4,
            "step {k} after contact: {away} away from the wall"
        );
    }
}

/// Feet closer than this to the plane of a wall's face touch it: the radius
/// and two skins, so a step that ends touching counts even if it bounced.
const TOUCHING: f32 = 0.3 + 0.021;

/// Steps along a wall through `along` (unit, horizontal) whose face is
/// `face` from the origin: checks the velocity from the first contact on, and
/// returns the steady speed along the wall.
fn slide_speed(positions: &[Vec3], along: Vec3, normal: Vec3, face: f32) -> f32 {
    steady_from_contact(positions, along, normal, face + TOUCHING);
    let steps: Vec<Vec3> = positions.windows(2).map(|w| w[1] - w[0]).collect();
    let tail = &steps[steps.len() / 2..];
    let tangent: Vec<f32> = tail.iter().map(|d| d.dot(along) / DT).collect();
    let sign = tangent[0].signum();
    assert!(
        tangent.iter().all(|t| t.signum() == sign),
        "tangent velocity flips: {tangent:?}"
    );
    let into: Vec<f32> = tail.iter().map(|d| d.dot(normal) / DT).collect();
    assert!(
        into.iter().all(|v| v.abs() < 1e-3),
        "normal velocity: {into:?}"
    );
    let mean = tangent.iter().sum::<f32>() / tangent.len() as f32;
    let jitter = tangent.iter().map(|t| (t - mean).abs()).fold(0.0, f32::max);
    assert!(jitter < 1e-3, "speed jitters by {jitter}: {tangent:?}");
    mean.abs()
}

#[test]
fn walls_at_0_30_45_89_degrees_slide_at_the_preserved_speed() {
    for wall_deg in [0.0f32, 30.0, 45.0, 89.0] {
        let yaw = wall_deg.to_radians();
        let mut room = Room::default();
        // The wall runs along `along` through the origin; the actor starts on its +normal side.
        room.floor(40.0)
            .wall("wall", Vec2::new(0.0, 0.0), Vec3::new(30.0, 2.0, 0.2), yaw);
        let along = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
        let normal = Vec3::new(yaw.sin(), 0.0, yaw.cos());
        let mut world = room.world(Settings::default());
        let id = actor(&mut world, normal * 0.6 - along * 8.0);
        settle(&mut world, id);
        // 45° into the wall.
        let push = (along - normal).normalize();
        let mut events = Vec::new();
        let positions = hold(&mut world, id, Vec2::new(push.x, push.z), 240, &mut events);
        let speed = slide_speed(&positions, along, normal, 0.1);
        assert!(
            (speed - SPEED * expected_slide(45.0)).abs() < 1e-3,
            "wall {wall_deg}°: {speed}"
        );
        let gap = (positions.last().unwrap().dot(normal)) - 0.1 - 0.3;
        assert!((0.0..0.02).contains(&gap), "wall {wall_deg}°: gap {gap}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e.kind, EventKind::HitWall { .. }))
        );
    }
}

#[test]
fn approach_angles_0_30_45_89_keep_the_expected_speed_along_a_turned_wall() {
    let yaw = 30f32.to_radians();
    let along = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    let normal = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    for approach in [0.0f32, 30.0, 45.0, 89.0] {
        let mut room = Room::default();
        room.floor(40.0)
            .wall("wall", Vec2::new(0.0, 0.0), Vec3::new(30.0, 2.0, 0.2), yaw);
        let mut world = room.world(Settings::default());
        let id = actor(&mut world, normal * 0.42 - along * 8.0);
        settle(&mut world, id);
        let a = approach.to_radians();
        let push = along * a.cos() - normal * a.sin();
        let mut events = Vec::new();
        let positions = hold(&mut world, id, Vec2::new(push.x, push.z), 180, &mut events);
        let speed = slide_speed(&positions, along, normal, 0.1);
        let want = SPEED * expected_slide(approach);
        assert!(
            (speed - want).abs() < 2e-3,
            "approach {approach}°: {speed} vs {want}"
        );
    }
}

#[test]
fn mesh_diagonal_wall_slides_without_jitter() {
    // A 45° wall made of 0.25 m quads, two triangles each.
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let along = Vec3::new(1.0, 0.0, -1.0).normalize();
    let cols = 80;
    for i in 0..=cols {
        for j in 0..=8 {
            vertices.push(along * (i as f32 * 0.25 - 10.0) + Vec3::Y * (j as f32 * 0.25));
        }
    }
    for i in 0..cols {
        for j in 0..8 {
            let v = |i: usize, j: usize| (i * 9 + j) as u32;
            triangles.push([v(i, j), v(i + 1, j), v(i + 1, j + 1)]);
            triangles.push([v(i, j), v(i + 1, j + 1), v(i, j + 1)]);
        }
    }
    let mut room = Room::default();
    room.floor(40.0)
        .add("diagonal", Owned::Mesh(vertices, triangles));
    let normal = Vec3::new(1.0, 0.0, 1.0).normalize();
    let mut world = room.world(Settings::default());
    let id = actor(&mut world, normal * 0.5 - along * 6.0);
    settle(&mut world, id);
    let push = (along * 2.0 - normal).normalize();
    let mut events = Vec::new();
    let positions = hold(&mut world, id, Vec2::new(push.x, push.z), 240, &mut events);
    steady_from_contact(&positions, along, normal, TOUCHING);
    let steps: Vec<Vec3> = positions.windows(2).map(|w| w[1] - w[0]).skip(60).collect();
    let first = steps[0];
    let jitter = steps
        .iter()
        .map(|d| (*d - first).length())
        .fold(0.0, f32::max);
    assert!(jitter < 1e-5, "per-step displacement varies by {jitter}");
    assert!(first.dot(along) > 0.0);
}

/// A wall along x at z = 0 with an opening `width` wide centred at x = 0.
fn doorway(width: f32) -> Room {
    let mut room = Room::default();
    let side = 4.0;
    room.floor(20.0)
        .wall(
            "left",
            Vec2::new(-width * 0.5 - side * 0.5, 0.0),
            Vec3::new(side, 2.5, 0.2),
            0.0,
        )
        .wall(
            "right",
            Vec2::new(width * 0.5 + side * 0.5, 0.0),
            Vec3::new(side, 2.5, 0.2),
            0.0,
        );
    room
}

/// Walks north (−z) from 2 m south of the wall at lateral offset `x`; returns
/// whether the actor got through.
fn passes(width: f32, x: f32) -> bool {
    let mut world = doorway(width).world(Settings::default());
    let id = actor(&mut world, Vec3::new(x, 0.0, 2.0));
    settle(&mut world, id);
    let mut events = Vec::new();
    let positions = hold(&mut world, id, Vec2::new(0.0, -1.0), 240, &mut events);
    for p in &positions {
        assert!(
            world
                .actor_overlap(&ActorCollider::default(), *p, 0.0)
                .is_none(),
            "inside at {p}"
        );
    }
    positions.last().unwrap().z < -1.0
}

#[test]
fn door_frame_of_0_8_m_approached_off_centre_passes() {
    for x in [0.0, 0.1, 0.18, -0.25, 0.3] {
        assert!(passes(0.8, x), "offset {x}");
    }
}

#[test]
fn gap_of_0_59_m_blocks_and_0_61_m_passes() {
    assert!(!passes(0.59, 0.0));
    assert!(!passes(0.59, 0.1));
    assert!(passes(0.61, 0.0));
    assert!(passes(0.61, 0.04));
}

#[test]
fn fast_actors_do_not_tunnel_through_a_3_cm_wall() {
    for speed in [10.0, 20.0, 30.0] {
        let mut room = Room::default();
        room.floor(200.0).wall(
            "thin",
            Vec2::new(0.0, 0.0),
            Vec3::new(100.0, 2.0, 0.03),
            0.0,
        );
        let mut world = room.world(Settings::default());
        let id = actor_with(
            &mut world,
            Vec3::new(0.0, 0.0, 3.0),
            ActorCollider::default(),
            speed,
        );
        settle(&mut world, id);
        let mut events = Vec::new();
        for angle in [-90.0f32, -60.0, -120.0] {
            let positions = hold(
                &mut world,
                id,
                stick_at(angle.to_radians()),
                60,
                &mut events,
            );
            for p in positions {
                assert!(
                    p.z > 0.015 + 0.3 - 1e-4,
                    "speed {speed}: through the wall at {p}"
                );
            }
        }
    }
}

/// The slowest step, as a share of the full step, of an actor holding north
/// from `past` metres beyond the end of a wall across its way, until it is
/// beyond the wall.
fn slowest_step_round_a_wall_end(past: f32, assist_angle_deg: f32) -> f32 {
    let mut room = Room::default();
    room.floor(20.0)
        .wall("wall", Vec2::new(-1.5, -1.1), Vec3::new(3.0, 2.0, 0.2), 0.0);
    let mut settings = Settings::default();
    settings.controller.corner_assist_angle_deg = assist_angle_deg;
    let mut world = room.world(settings);
    let id = actor(&mut world, Vec3::new(past, 0.0, 0.0));
    let path = hold(&mut world, id, Vec2::new(0.0, -1.0), 120, &mut Vec::new());
    let beyond = path
        .iter()
        .position(|p| p.z < -1.5)
        .expect("never got past the wall");
    path[..=beyond]
        .windows(2)
        .map(|w| (w[1] - w[0]).length() / (SPEED * DT))
        .fold(f32::INFINITY, f32::min)
}

#[test]
fn corner_assist_keeps_a_nearly_head_on_actor_moving_round_a_wall_end() {
    let default = Settings::default().controller.corner_assist_angle_deg;
    let assisted = slowest_step_round_a_wall_end(0.05, default);
    let plain = slowest_step_round_a_wall_end(0.05, 0.0);
    assert!(
        assisted > 0.4 && plain < 0.25,
        "assisted {assisted}, plain {plain}"
    );
}
