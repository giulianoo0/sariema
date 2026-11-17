// SPDX-License-Identifier: Apache-2.0

//! Replays and a long random walk. Every fixture value and input here is an
//! exact binary fraction or comes from `yaw_matrix`, so the inputs are the
//! same bits on every platform and the pinned hash checks the simulation
//! itself across platforms.

mod common;

use common::{Owned, Rng, Room};
use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{
    ActorCollider, ActorDesc, ActorId, ColliderId, Event, Head, Intent, Pose, Settings, Shape,
    World, yaw_matrix,
};

fn arena() -> Room {
    let mut room = Room::default();
    room.floor(16.0)
        .wall(
            "north",
            Vec2::new(0.0, -7.5),
            Vec3::new(15.0, 2.0, 0.25),
            0.0,
        )
        .wall(
            "south",
            Vec2::new(0.0, 7.5),
            Vec3::new(15.0, 2.0, 0.25),
            0.0,
        )
        .wall(
            "west",
            Vec2::new(-7.5, 0.0),
            Vec3::new(15.0, 2.0, 0.25),
            std::f32::consts::FRAC_PI_2,
        )
        .wall(
            "east",
            Vec2::new(7.5, 0.0),
            Vec3::new(15.0, 2.0, 0.25),
            std::f32::consts::FRAC_PI_2,
        );
    room
}

/// The arena with a seeded clutter of boxes, hulls, roots, steps and a mesh mound.
fn cluttered(seed: u64) -> Room {
    let mut room = arena();
    let mut rng = Rng(seed);
    for i in 0..24 {
        let at = Vec3::new(rng.range(-6.0, 6.0), 0.0, rng.range(-6.0, 6.0));
        let name = format!("clutter{i}");
        match i % 4 {
            0 => room.add(
                &name,
                Owned::Box {
                    center: at + Vec3::Y * 0.4,
                    half: Vec3::new(
                        rng.range(0.2, 0.8),
                        rng.range(0.05, 0.6),
                        rng.range(0.2, 0.8),
                    ),
                    rotation: yaw_matrix(rng.range(0.0, 3.0)),
                },
            ),
            1 => room.add(
                &name,
                Owned::Hull(
                    (0..10)
                        .map(|_| {
                            at + Vec3::new(
                                rng.range(-0.6, 0.6),
                                rng.range(0.0, 1.2),
                                rng.range(-0.6, 0.6),
                            )
                        })
                        .collect(),
                ),
            ),
            2 => room.add(
                &name,
                Owned::Capsule {
                    a: at + Vec3::new(rng.range(-1.5, 1.5), -0.2, rng.range(-1.5, 1.5)),
                    b: at
                        + Vec3::new(
                            rng.range(-1.5, 1.5),
                            rng.range(0.3, 1.4),
                            rng.range(-1.5, 1.5),
                        ),
                    radius: rng.range(0.1, 0.35),
                },
            ),
            _ => room.add(
                &name,
                Owned::Box {
                    center: at + Vec3::Y * 0.05,
                    half: Vec3::new(
                        rng.range(0.4, 1.2),
                        0.05 + 0.04 * (i % 3) as f32,
                        rng.range(0.4, 1.2),
                    ),
                    rotation: Mat3::IDENTITY,
                },
            ),
        };
    }
    let n = 8;
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let (x, z) = (
                i as f32 / n as f32 * 2.0 - 1.0,
                j as f32 / n as f32 * 2.0 - 1.0,
            );
            let h = (1.0 - x * x) * (1.0 - z * z) * 0.6;
            vertices.push(Vec3::new(x * 2.0 + 3.0, h, z * 2.0 - 3.0));
        }
    }
    for j in 0..n {
        for i in 0..n {
            let v = |i: usize, j: usize| (j * (n + 1) + i) as u32;
            triangles.push([v(i, j), v(i, j + 1), v(i + 1, j + 1)]);
            triangles.push([v(i, j), v(i + 1, j + 1), v(i + 1, j)]);
        }
    }
    room.add("mound", Owned::Mesh(vertices, triangles));
    room
}

fn hero() -> ActorCollider {
    ActorCollider {
        radius: 0.3,
        height: 1.0,
        head: Some(Head {
            radius: 0.34,
            height: 1.1,
            forward: 0.0,
        }),
    }
}

/// An intent log: what to apply before which step.
fn log(steps: u64) -> Vec<(u64, Intent)> {
    let mut rng = Rng(7);
    let a = ActorId(0);
    let b = ActorId(1);
    let mut out = Vec::new();
    let mut k = 0;
    while k < steps {
        let pick = rng.next() % 16;
        let exact = |rng: &mut Rng| (rng.next() % 17) as f32 / 8.0 - 1.0;
        let intent = match pick {
            0 => Intent::Teleport {
                actor: a,
                position: Vec3::new(exact(&mut rng) * 5.0, 0.5, exact(&mut rng) * 5.0),
                yaw: exact(&mut rng),
            },
            1 => Intent::Knockback {
                actor: b,
                push: Vec3::new(exact(&mut rng) * 2.0, 0.0, exact(&mut rng) * 2.0),
                steps: 22,
            },
            2 => Intent::MoveTo {
                actor: b,
                target: Vec3::new(exact(&mut rng) * 5.0, 0.0, exact(&mut rng) * 5.0),
                face: Some(exact(&mut rng)),
            },
            3 => Intent::SetSpeedScale {
                actor: a,
                scale: 1.0 + exact(&mut rng) * 0.5,
            },
            4 => Intent::SetColliderEnabled {
                collider: ColliderId(5),
                enabled: rng.next().is_multiple_of(2),
            },
            _ => Intent::Move {
                actor: if pick.is_multiple_of(2) { a } else { b },
                stick: Vec2::new(exact(&mut rng), exact(&mut rng)),
            },
        };
        out.push((k, intent));
        k += 1 + rng.next() % 40;
    }
    out
}

fn replay_world() -> World {
    let mut room = cluttered(3);
    room.platform(
        "lift",
        Owned::Box {
            center: Vec3::ZERO,
            half: Vec3::new(1.0, 0.1, 1.0),
            rotation: Mat3::IDENTITY,
        },
        Pose {
            translation: Vec3::new(-5.0, 0.5, 5.0),
            yaw: 0.0,
        },
    );
    let mut world = room.world(Settings::default());
    world
        .add_trigger(
            "middle",
            Shape::Box {
                center: Vec3::new(0.0, 1.0, 0.0),
                half_extents: Vec3::new(2.0, 1.0, 2.0),
                rotation: Mat3::IDENTITY,
            },
        )
        .unwrap();
    for p in [
        Vec3::new(-1.0, 0.5, 6.0),
        Vec3::new(1.0, 0.5, 6.0),
        Vec3::new(-5.0, 0.8, 5.0),
    ] {
        world
            .add_actor(ActorDesc {
                position: p,
                collider: hero(),
                ..ActorDesc::default()
            })
            .unwrap();
    }
    world
}

/// What a frame loop asks between steps: path searches, which refresh the
/// walkable grid after a collider changes, and read-only queries.
fn between_steps(world: &mut World, path: &mut Vec<Vec3>) {
    world.find_path(
        ActorId(0),
        Vec3::new(-4.0, 0.0, 4.0),
        Vec3::new(4.0, 0.0, -4.0),
        path,
    );
    assert!(world.walkable_cells(ActorId(1)).count() > 0);
    world.raycast(Vec3::new(0.5, 3.0, 0.5), Vec3::NEG_Y, 5.0);
    let a = world.actor(ActorId(2));
    world.actor_overlap(&hero(), a.position, a.yaw);
}

/// Runs `steps` steps, `batch` per frame, delivering each intent before its
/// step; batched runs also query the world after every step, as a frame loop
/// does. Returns every step's hash and the events. No actor is ever inside the
/// room beyond the skin.
fn replay(steps: u64, batch: u64) -> (Vec<u64>, Vec<Event>) {
    let mut world = replay_world();
    let lift = world.collider_by_name("lift").unwrap();
    let skin = world.settings().collision.skin;
    let intents = log(steps);
    let mut next = 0;
    let mut hashes = Vec::new();
    let mut events = Vec::new();
    let mut path = Vec::new();
    let mut k = 0;
    while k < steps {
        for _ in 0..batch.min(steps - k) {
            while next < intents.len() && intents[next].0 == k {
                world.apply(intents[next].1);
                next += 1;
            }
            let lift_y = (k % 240) as f32 / 120.0;
            world.set_platform_pose(
                lift,
                Pose {
                    translation: Vec3::new(
                        -5.0,
                        0.5 + if lift_y > 1.0 { 2.0 - lift_y } else { lift_y },
                        5.0,
                    ),
                    yaw: 0.0,
                },
            );
            world.step(&mut events);
            hashes.push(world.state_hash());
            for id in 0..world.actor_count() {
                let id = ActorId(id as u32);
                let a = world.actor(id);
                if let Some((c, separation)) =
                    world.actor_overlap(&world.actor_collider(id), a.position, a.yaw)
                {
                    assert!(
                        separation >= -skin,
                        "step {k}: actor {} {separation} m inside {}",
                        id.0,
                        world.collider_name(c)
                    );
                }
            }
            if batch > 1 {
                between_steps(&mut world, &mut path);
            }
            k += 1;
        }
    }
    (hashes, events)
}

#[test]
fn the_same_intent_log_gives_the_same_states_whatever_the_batching() {
    let (one, events) = replay(1200, 1);
    assert!(events.len() > 10, "{events:?}");
    assert_eq!(replay(1200, 1).0, one);
    for batch in [2, 5] {
        let (hashes, again) = replay(1200, batch);
        assert_eq!(hashes, one, "batching {batch}");
        assert_eq!(again, events);
    }
}

/// Pins the simulation's result on every platform: aarch64 macOS produced
/// this value. Update it only when a change to the simulation is meant.
#[test]
fn the_replay_hash_is_the_same_on_every_platform() {
    let (hashes, _) = replay(1200, 1);
    assert_eq!(
        *hashes.last().unwrap(),
        9_237_084_439_031_553_643,
        "final hash"
    );
}

#[test]
fn a_long_random_walk_never_enters_geometry_or_rises_without_ground() {
    let steps: u64 = 100_000;
    let mut world = cluttered(11).world(Settings::default());
    let collider = ActorCollider {
        head: Some(Head {
            radius: 0.34,
            height: 1.1,
            forward: 0.12,
        }),
        ..hero()
    };
    let id = world
        .add_actor(ActorDesc {
            position: Vec3::new(0.0, 2.0, 6.5),
            collider,
            ..ActorDesc::default()
        })
        .unwrap();
    let skin = world.settings().collision.skin;
    let mut rng = Rng(99);
    let mut events = Vec::new();
    let mut hold = 0;
    let mut was_grounded = false;
    for k in 0..steps {
        let mut teleported = false;
        if hold == 0 {
            hold = 20 + rng.next() % 70;
            let angle = rng.range(0.0, std::f32::consts::TAU);
            let tilt = rng.range(0.0, 1.0);
            world.apply(Intent::Move {
                actor: id,
                stick: Vec2::new(angle.cos(), angle.sin()) * tilt,
            });
            if rng.next().is_multiple_of(50) {
                world.apply(Intent::Teleport {
                    actor: id,
                    position: Vec3::new(
                        rng.range(-7.0, 7.0),
                        rng.range(0.0, 1.0),
                        rng.range(-7.0, 7.0),
                    ),
                    yaw: 0.0,
                });
                teleported = true;
            }
        }
        hold -= 1;
        events.clear();
        world.step(&mut events);
        let a = world.actor(id);
        if let Some((c, separation)) = world.actor_overlap(&collider, a.position, a.yaw) {
            assert!(
                separation >= -skin,
                "step {k}: {separation} inside {} at {}",
                world.collider_name(c),
                a.position
            );
        }
        if a.position.y > a.previous_position.y + 1e-4 && !teleported {
            assert!(
                a.grounded || was_grounded,
                "step {k}: rose without ground: {a:?}"
            );
        }
        was_grounded = a.grounded;
        assert!(
            a.position.x.abs() < 7.5 && a.position.z.abs() < 7.5 && a.position.y > -0.1,
            "step {k}: left the arena: {a:?}"
        );
    }
}
