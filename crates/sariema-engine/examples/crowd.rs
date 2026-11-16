// SPDX-License-Identifier: Apache-2.0

//! The cost of an engine step with a crowd walking under the collision
//! module, each actor the `climb` example's hero looping its clip:
//! `cargo run --release -p sariema-engine --example crowd -- 2000`. Prints
//! JSON with the median, 90th percentile and slowest step while they walk.

use std::path::Path;
use std::time::Instant;

use sariema_engine::{Assets, Engine, EngineSettings, RoomAssets};
use sariema_format::collision::{ActorBody, Collider, CollisionPack, HeadSphere, Shape};
use sariema_format::script::{ClipMode, Command, Entity, Kind, name_id};
use sariema_import::{ActorSource, build_actor};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};

/// Metres between neighbours in the crowd's grid, and how far each walks.
const SPACING: f32 = 1.0;
const WALK: f32 = 8.0;
const SPEED: f32 = 1.2;
/// Steps before timing, which include planning every path, and steps timed.
const SETTLE: u64 = 10;
const TIMED: u64 = 300;

struct Crowd {
    count: u32,
    columns: u32,
    step: u64,
}

impl Crowd {
    fn place(&self, i: u32) -> [f32; 3] {
        let (x, z) = (i % self.columns, i / self.columns);
        [x as f32 * SPACING, 0.0, -(z as f32) * SPACING]
    }
}

impl Logic for Crowd {
    fn tick(&mut self, _events: &[u8], _ends: &[u32], out: &mut Vec<u8>) -> Result<(), Stop> {
        out.clear();
        for i in 0..self.count {
            let entity = Entity {
                index: i,
                generation: 0,
            };
            let at = self.place(i);
            match self.step {
                0 => {
                    Command::Spawn {
                        entity,
                        kind: Kind::Actor,
                        asset: name_id("hero"),
                    }
                    .write(out);
                    Command::SetTransform {
                        entity,
                        position: at,
                        yaw: 0.0,
                    }
                    .write(out);
                }
                1 => {
                    Command::MoveTo {
                        entity,
                        target: [at[0] + WALK, 0.0, at[2]],
                        speed: SPEED,
                    }
                    .write(out);
                    Command::PlayClip {
                        entity,
                        clip: name_id("wave"),
                        mode: ClipMode::Loop,
                        blend: 0.0,
                        speed: 1.0,
                    }
                    .write(out);
                }
                _ => {}
            }
        }
        self.step += 1;
        Ok(())
    }
}

fn floor(columns: u32, rows: u32) -> CollisionPack {
    let (w, d) = (columns as f32 * SPACING + WALK, rows as f32 * SPACING);
    let margin = 2.0;
    CollisionPack {
        room: "crowd".into(),
        colliders: vec![Collider {
            name: "floor".into(),
            shape: Shape::Box {
                center: [w / 2.0 - SPACING, -0.05, -d / 2.0 + SPACING / 2.0],
                half_extents: [w / 2.0 + margin, 0.05, d / 2.0 + margin],
                rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            },
            enabled: true,
        }],
        triggers: Vec::new(),
        actors: vec![ActorBody {
            name: "hero".into(),
            radius: 0.3,
            height: 1.0,
            head: Some(HeadSphere {
                radius: 0.34,
                height: 1.1,
                forward: 0.0,
            }),
            visual_bounds: None,
        }],
        spots: Vec::new(),
        routes: Vec::new(),
        traversals: Vec::new(),
        nav_bounds: None,
        hinges: Vec::new(),
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let count: u32 = std::env::args()
        .nth(1)
        .map_or(100, |n| n.parse().expect("an actor count"));
    let columns = (count as f32).sqrt().ceil() as u32;
    let rows = count.div_ceil(columns);
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let source = std::fs::read_to_string(examples.join("climb/actors/hero.json")).unwrap();
    let hero = build_actor(
        &ActorSource::from_json(&source).unwrap(),
        &examples.join("collision"),
    )
    .unwrap();
    let assets = Assets {
        rooms: vec![RoomAssets::collision("floor", floor(columns, rows))],
        actors: vec![("hero".into(), hero)],
        sounds: None,
        hud: None,
    };
    let mut settings = Settings::default();
    settings.motion.builtin = false;
    let logic = Crowd {
        count,
        columns,
        step: 0,
    };
    let sim = Simulation::new(settings, Vec::new(), logic, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..SETTLE {
        engine.step(&actions);
    }
    let positions = |engine: &Engine<Crowd>| -> Vec<glam::Vec3> {
        (0..count)
            .map(|index| {
                let entity = Entity {
                    index,
                    generation: 0,
                };
                engine.state().world.node(entity).unwrap().local.position
            })
            .collect()
    };
    let start = positions(&engine);
    let mut steps = Vec::with_capacity(TIMED as usize);
    for _ in 0..TIMED {
        let t = Instant::now();
        engine.step(&actions);
        steps.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let walked: Vec<f32> = (start.iter().zip(positions(&engine)))
        .map(|(a, b)| (b - *a).length())
        .collect();
    steps.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({
            "actors": count,
            "steps_timed": TIMED,
            "median_step_ms": percentile(&steps, 0.5),
            "p90_step_ms": percentile(&steps, 0.9),
            "max_step_ms": steps[steps.len() - 1],
            "least_walked_m": walked.iter().copied().fold(f32::INFINITY, f32::min),
        })
    );
}
