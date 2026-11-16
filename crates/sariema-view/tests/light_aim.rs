// SPDX-License-Identifier: Apache-2.0

//! A room's spot light carried by the hero: aimed in the hero's frame, the
//! frame's spot points ahead of the hero as it walks; hidden, the light
//! leaves the frame's lights, and shown again it comes back.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec3;
use sariema_engine::{Assets, Engine, EngineSettings, RoomAssets};
use sariema_format::LightKind;
use sariema_format::scenery::RoomIndex;
use sariema_format::script::{Command, Entity, GameAssets, Kind, name_id};
use sariema_import::{ActorSource, RoomSource, build_actor, build_collision, build_room};
use sariema_render::{Headless, Quality};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};
use sariema_view::{GameView, ViewSettings};

const W: u32 = 96;
const H: u32 = 54;
const HERO: Entity = Entity {
    index: 0,
    generation: 0,
};
const LAMP: Entity = Entity {
    index: 1,
    generation: 0,
};
const AHEAD: [f32; 3] = [0.0, 1.0, -3.0];

struct Script(Vec<(u64, Command<'static>)>, u64);

impl Logic for Script {
    fn tick(&mut self, _: &[u8], _: &[u32], out: &mut Vec<u8>) -> Result<(), Stop> {
        out.clear();
        for (_, c) in self.0.iter().filter(|(n, _)| *n == self.1) {
            c.write(out);
        }
        self.1 += 1;
        Ok(())
    }
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn assets() -> (GameAssets, Assets) {
    let root = examples().join("collision");
    let text = std::fs::read_to_string(root.join("rooms/sill.json")).unwrap();
    let source = RoomSource::from_json(&text).unwrap();
    let pack = build_room(&source, &root).unwrap();
    let room = RoomAssets {
        name: "sill".into(),
        room: Some(Arc::from(sariema_format::encode(&pack))),
        collision: Some(Arc::new(build_collision(&source, &root).unwrap().unwrap())),
        index: RoomIndex::of(&pack),
        neighbours: Vec::new(),
        beside: Vec::new(),
        ambience: None,
    };
    let text = std::fs::read_to_string(examples().join("climb/actors/hero.json")).unwrap();
    let actor = build_actor(&ActorSource::from_json(&text).unwrap(), &root).unwrap();
    let drawn = GameAssets {
        actors: vec![("hero".into(), sariema_format::actor::encode(&actor))],
        ..GameAssets::default()
    };
    let assets = Assets {
        rooms: vec![room],
        actors: vec![("hero".into(), actor)],
        ..Assets::default()
    };
    (drawn, assets)
}

fn script() -> Script {
    let spawn = |entity, kind, asset| Command::Spawn {
        entity,
        kind,
        asset,
    };
    Script(
        vec![
            (0, spawn(HERO, Kind::Actor, name_id("hero"))),
            (
                0,
                Command::SetTransform {
                    entity: HERO,
                    position: [0.6, 0.0, -5.0],
                    yaw: -90.0,
                },
            ),
            (0, spawn(LAMP, Kind::Light, name_id("a.foco"))),
            (
                1,
                Command::LightAim {
                    entity: LAMP,
                    at: HERO,
                    target: AHEAD,
                },
            ),
            (
                2,
                Command::MoveTo {
                    entity: HERO,
                    target: [5.4, 0.0, -5.0],
                    speed: 1.0,
                },
            ),
            (
                40,
                Command::SetVisible {
                    entity: LAMP,
                    visible: false,
                },
            ),
            (
                60,
                Command::SetVisible {
                    entity: LAMP,
                    visible: true,
                },
            ),
        ],
        0,
    )
}

struct Frame {
    /// The frame's spot light's target, if it shone.
    spot: Option<Vec3>,
    lights: usize,
    ahead: Vec3,
}

#[test]
fn a_carried_spot_points_ahead_of_its_carrier_and_hidden_takes_no_place() {
    let (drawn, assets) = assets();
    let mut runtime = Settings::default();
    runtime.motion.builtin = false;
    let sim = Simulation::new(runtime, Vec::new(), script(), 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let mut h = Headless::new(W, H, Quality::high()).unwrap();
    let settings = ViewSettings {
        threads: false,
        ..ViewSettings::default()
    };
    let mut view = GameView::load_with(h.renderer(), &drawn, &engine, settings).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let aspect = W as f32 / H as f32;
    let frames: Vec<Frame> = (0..80)
        .map(|_| {
            engine.step(&actions);
            h.paint(|r, target| view.world(r, &mut engine, 1.0, aspect, target))
                .unwrap();
            let lights = h.renderer().frame_lights();
            let spot = lights.iter().find_map(|l| match l.kind {
                LightKind::Spot { target, .. } => Some(Vec3::from(target)),
                LightKind::Point => None,
            });
            let hero = engine.state().world.pose(HERO).unwrap();
            Frame {
                spot,
                lights: lights.len(),
                ahead: hero.position + hero.rotation * Vec3::from(AHEAD),
            }
        })
        .collect();
    let all = frames[5].lights;
    for (n, f) in frames.iter().enumerate().skip(5) {
        if (40..60).contains(&n) {
            assert_eq!(
                (f.lights, f.spot),
                (all - 1, None),
                "frame {n}: hidden, the spot takes no place"
            );
            continue;
        }
        assert_eq!(f.lights, all, "frame {n}");
        let spot = f.spot.expect("the spot shines");
        assert!(
            spot.distance(f.ahead) < 1e-4,
            "frame {n}: the spot points at {spot}, not 3 m ahead of the hero at {}",
            f.ahead
        );
    }
    let walked = frames[79].ahead.distance(frames[5].ahead);
    assert!(walked > 0.5, "the hero carried the aim {walked} m");
}
