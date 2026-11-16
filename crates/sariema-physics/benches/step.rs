// SPDX-License-Identifier: Apache-2.0

//! Cost of one fixed step for 1, 100 and 2,000 actors walking a 39 m × 24 m
//! room (partitioned, cluttered, on a bumpy triangle-mesh floor), and of a
//! 20 m path search. Times are this thread's
//! CPU time, on a performance core where the system allows. Also counts heap
//! allocations during the measured steps, which should be none.
//!
//! `cargo bench -p sariema-physics --bench step`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use glam::{Mat3, Vec2, Vec3};
use sariema_physics::{
    ActorCollider, ActorDesc, ActorId, ColliderDesc, Head, Intent, OwnedShape, Settings, World,
    yaw_matrix,
};

struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn pthread_set_qos_class_self_np(class: u32, relative_priority: i32) -> i32;
    fn clock_gettime_nsec_np(clock: u32) -> u64;
}

/// Asks to run on a performance core, as a game's main thread would.
fn prefer_fast_cores() {
    #[cfg(target_os = "macos")]
    unsafe {
        pthread_set_qos_class_self_np(0x21, 0);
    }
}

/// Microseconds this thread has run, so time other processes take is not counted.
fn thread_us() -> f64 {
    #[cfg(target_os = "macos")]
    {
        (unsafe { clock_gettime_nsec_np(16) }) as f64 / 1e3
    }
    #[cfg(not(target_os = "macos"))]
    {
        use std::sync::OnceLock;
        use std::time::Instant;
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1e6
    }
}

const WIDTH: f32 = 39.0;
const DEPTH: f32 = 24.0;
const FRAME_US: f64 = 1e6 / 60.0;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * ((self.next() >> 40) as f32 / (1u64 << 24) as f32)
    }
}

fn wall(at: Vec2, length: f32, yaw: f32) -> OwnedShape {
    let rotation = yaw_matrix(yaw);
    OwnedShape::Box {
        center: Vec3::new(at.x, 1.5, at.y),
        half_extents: Vec3::new(length * 0.5, 1.5, 0.1),
        rotation,
    }
}

/// The floor mesh at 0.5 m, bumps up to 8 cm; outer walls; two partitions
/// along x and three along z with 1 m doors; a closed 2 m pen; up to 160 props.
fn room() -> Vec<(String, OwnedShape)> {
    let mut shapes = Vec::new();
    let (nx, nz) = ((WIDTH * 2.0) as u32, (DEPTH * 2.0) as u32);
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for j in 0..=nz {
        for i in 0..=nx {
            let (x, z) = (i as f32 * 0.5, -(j as f32) * 0.5);
            let bump = 0.04 * (1.0 + ((i * 7 + j * 13) % 5) as f32 / 4.0 - 0.5);
            vertices.push(Vec3::new(x, bump, z));
        }
    }
    let v = |i: u32, j: u32| j * (nx + 1) + i;
    for j in 0..nz {
        for i in 0..nx {
            triangles.push([v(i, j), v(i + 1, j + 1), v(i, j + 1)]);
            triangles.push([v(i, j), v(i + 1, j), v(i + 1, j + 1)]);
        }
    }
    shapes.push(("floor".to_owned(), OwnedShape::Mesh(vertices, triangles)));
    let quarter = std::f32::consts::FRAC_PI_2;
    for (name, at, length, yaw) in [
        ("north", Vec2::new(WIDTH / 2.0, -DEPTH), WIDTH, 0.0),
        ("south", Vec2::new(WIDTH / 2.0, 0.0), WIDTH, 0.0),
        ("west", Vec2::new(0.0, -DEPTH / 2.0), DEPTH, quarter),
        ("east", Vec2::new(WIDTH, -DEPTH / 2.0), DEPTH, quarter),
    ] {
        shapes.push((name.to_owned(), wall(at, length, yaw)));
    }
    let mut k = 0;
    for z in [-8.0, -16.0] {
        let mut x = 0.0;
        for door in [6.0, 19.5, 33.0] {
            shapes.push((
                format!("partition{k}"),
                wall(Vec2::new((x + door - 0.5) / 2.0, z), door - 0.5 - x, 0.0),
            ));
            k += 1;
            x = door + 0.5;
        }
        shapes.push((
            format!("partition{k}"),
            wall(Vec2::new((x + WIDTH) / 2.0, z), WIDTH - x, 0.0),
        ));
        k += 1;
    }
    for x in [13.0, 26.0] {
        for (z0, z1) in [(0.0, -3.5), (-4.5, -8.0), (-16.0, -19.5), (-20.5, -24.0)] {
            shapes.push((
                format!("partition{k}"),
                wall(Vec2::new(x, (z0 + z1) / 2.0), z0 - z1, quarter),
            ));
            k += 1;
        }
    }
    for (i, (at, yaw)) in [
        (Vec2::new(35.5, -11.0), 0.0),
        (Vec2::new(35.5, -13.0), 0.0),
        (Vec2::new(34.5, -12.0), quarter),
        (Vec2::new(36.5, -12.0), quarter),
    ]
    .into_iter()
    .enumerate()
    {
        shapes.push((format!("pen{i}"), wall(at, 2.2, yaw)));
    }
    let mut rng = Rng(5);
    for i in 0..160 {
        let at = Vec3::new(
            rng.range(1.0, WIDTH - 1.0),
            0.0,
            rng.range(-DEPTH + 1.0, -1.0),
        );
        let shape = match i % 4 {
            0 => OwnedShape::Box {
                center: at + Vec3::Y * 0.4,
                half_extents: Vec3::new(
                    rng.range(0.2, 0.6),
                    rng.range(0.1, 0.5),
                    rng.range(0.2, 0.6),
                ),
                rotation: yaw_matrix(rng.range(0.0, 3.0)),
            },
            1 => OwnedShape::Hull(
                (0..12)
                    .map(|_| {
                        at + Vec3::new(
                            rng.range(-0.5, 0.5),
                            rng.range(0.0, 1.0),
                            rng.range(-0.5, 0.5),
                        )
                    })
                    .collect(),
            ),
            2 => OwnedShape::Capsule {
                a: at + Vec3::new(rng.range(-1.2, 1.2), -0.2, rng.range(-1.2, 1.2)),
                b: at
                    + Vec3::new(
                        rng.range(-1.2, 1.2),
                        rng.range(0.3, 1.2),
                        rng.range(-1.2, 1.2),
                    ),
                radius: rng.range(0.1, 0.3),
            },
            _ => OwnedShape::Box {
                center: at + Vec3::Y * 0.08,
                half_extents: Vec3::new(rng.range(0.4, 1.0), 0.08, rng.range(0.4, 1.0)),
                rotation: Mat3::IDENTITY,
            },
        };
        shapes.push((format!("prop{i}"), shape));
    }
    shapes
}

fn world(shapes: &[(String, OwnedShape)]) -> World {
    let descs: Vec<ColliderDesc> = shapes
        .iter()
        .map(|(name, shape)| ColliderDesc {
            name,
            shape: shape.shape(),
            enabled: true,
            platform: None,
        })
        .collect();
    World::new(Settings::default(), &descs).expect("the bench room is valid")
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

/// Places `count` actors on free spots of a jittered grid over the room.
fn populate(world: &mut World, count: usize, rng: &mut Rng) -> Vec<ActorId> {
    let collider = hero();
    let spacing = ((WIDTH - 2.0) * (DEPTH - 2.0) / (count as f32 * 1.6))
        .sqrt()
        .min(4.0);
    let mut ids = Vec::new();
    'fill: for pass in 0..8 {
        let mut z = -1.0 - spacing * (pass as f32 / 8.0);
        while z > -DEPTH + 1.0 {
            let mut x = 1.0 + spacing * (pass as f32 / 8.0);
            while x < WIDTH - 1.0 {
                let feet = Vec3::new(x + rng.range(-0.1, 0.1), 0.15, z + rng.range(-0.1, 0.1));
                if world.actor_overlap(&collider, feet, 0.0).is_none() {
                    ids.push(
                        world
                            .add_actor(ActorDesc {
                                position: feet,
                                collider,
                                ..ActorDesc::default()
                            })
                            .expect("a valid actor"),
                    );
                    if ids.len() == count {
                        break 'fill;
                    }
                }
                x += spacing;
            }
            z -= spacing;
        }
    }
    assert_eq!(ids.len(), count, "the room holds {count} actors");
    ids
}

fn steer(world: &mut World, ids: &[ActorId], rng: &mut Rng) {
    for &actor in ids {
        if rng.next().is_multiple_of(45) {
            let angle = rng.range(0.0, std::f32::consts::TAU);
            world.apply(Intent::Move {
                actor,
                stick: Vec2::new(angle.cos(), angle.sin()),
            });
        }
    }
}

fn steps(shapes: &[(String, OwnedShape)], count: usize, measured: u32) {
    let mut world = world(shapes);
    let mut rng = Rng(17);
    let ids = populate(&mut world, count, &mut rng);
    let mut events = Vec::with_capacity(4 * count);
    for _ in 0..120 {
        steer(&mut world, &ids, &mut rng);
        events.clear();
        world.step(&mut events);
    }
    let mut times = Vec::with_capacity(measured as usize);
    let mut allocations = 0;
    for _ in 0..measured {
        steer(&mut world, &ids, &mut rng);
        events.clear();
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let start = thread_us();
        world.step(&mut events);
        let us = thread_us() - start;
        allocations += ALLOCATIONS.load(Ordering::Relaxed) - before;
        times.push(us);
    }
    let mean = times.iter().sum::<f64>() / measured as f64;
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    let p99 = times[times.len() * 99 / 100];
    println!(
        "{count:>5} actors: median {median:>8.1} µs/step ({:>5.1}% of a 16.6 ms frame), mean {mean:>8.1}, p99 {p99:>8.1}, {:.2} µs per actor, {allocations} allocations in {measured} steps",
        100.0 * median / FRAME_US,
        median / count as f64
    );
}

fn paths(shapes: &[(String, OwnedShape)]) {
    let mut world = world(shapes);
    let id = world
        .add_actor(ActorDesc {
            position: Vec3::new(3.0, 0.15, -3.0),
            collider: hero(),
            ..ActorDesc::default()
        })
        .expect("a valid actor");
    let start = thread_us();
    world.prepare_paths(id);
    let build = (thread_us() - start) / 1e3;
    let cells = world.walkable_cells(id).count();
    println!("walkable grid: {build:.1} ms to build once at load, {cells} cells");
    let free: Vec<Vec3> = world.walkable_cells(id).collect();
    let nearest_free = |p: Vec3| {
        *free
            .iter()
            .min_by(|a, b| a.distance_squared(p).total_cmp(&b.distance_squared(p)))
            .expect("the room has walkable cells")
    };
    let mut waypoints = Vec::new();
    for (from, to) in [
        (Vec3::new(3.0, 0.0, -3.0), Vec3::new(21.0, 0.0, -12.0)),
        (Vec3::new(3.0, 0.0, -20.0), Vec3::new(23.0, 0.0, -21.0)),
        (Vec3::new(36.0, 0.0, -3.0), Vec3::new(30.0, 0.0, -22.0)),
        (Vec3::new(16.0, 0.0, -12.0), Vec3::new(35.5, 0.0, -12.0)),
    ] {
        let (from, to) = (nearest_free(from), nearest_free(to));
        let runs = 20;
        let start = thread_us();
        let mut found = false;
        for _ in 0..runs {
            found = world.find_path(id, from, to, &mut waypoints);
        }
        let us = (thread_us() - start) / runs as f64;
        let mut length = 0.0;
        let mut at = from;
        for &w in &waypoints {
            length += at.distance(w);
            at = w;
        }
        println!(
            "path {:.1} m apart: {us:>7.0} µs, {}",
            from.distance(to),
            if found {
                format!("{} waypoints, {length:.1} m long", waypoints.len())
            } else {
                "unreachable".to_owned()
            }
        );
    }
}

fn main() {
    prefer_fast_cores();
    let shapes = room();
    let triangles: usize = shapes
        .iter()
        .map(|(_, s)| match s {
            OwnedShape::Mesh(_, t) => t.len(),
            _ => 0,
        })
        .sum();
    println!(
        "room {WIDTH} m × {DEPTH} m: {} colliders, {triangles} floor triangles",
        shapes.len()
    );
    steps(&shapes, 1, 2000);
    steps(&shapes, 100, 600);
    steps(&shapes, 2000, 120);
    paths(&shapes);
}
