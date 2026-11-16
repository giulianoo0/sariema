// SPDX-License-Identifier: Apache-2.0

//! Effects as a game drives them, from its commands to the frame: a
//! sparkle emitter, a trail with its wave running to the goal, and a
//! highlight pulsing on a dark and a light piece and on the hero, each a
//! strip of frames kept as a golden; no pipeline or buffer is made for
//! them after the first frame.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use image::RgbaImage;
use sariema_engine::{Assets, Engine, EngineSettings, RoomAssets};
use sariema_format::demo::demo_room;
use sariema_format::scenery::RoomIndex;
use sariema_format::script::{Command, Entity, GameAssets, Kind, PointList, name_id};
use sariema_format::{Aabb, Draw, Material, Node, RoomPack};
use sariema_import::{ActorSource, build_actor};
use sariema_render::{GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, Headless, Quality, compare_images};
use sariema_runtime::fx::{Effect, Shape};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};
use sariema_view::GameView;

const W: u32 = 256;
const H: u32 = 144;

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

fn golden(name: &str) -> String {
    format!(
        "{}/../../tests/golden/{name}.png",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn place(x: f32, z: f32) -> [f32; 16] {
    Mat4::from_scale_rotation_translation(Vec3::splat(0.8), Quat::IDENTITY, Vec3::new(x, 0.0, z))
        .to_cols_array()
}

/// The demo room's floor and lights, seen from 6.5 m, with a dark brown
/// box, brown by its vertex colours, and a cream one, each a node.
fn room() -> RoomPack {
    let mut room = demo_room();
    room.draws.truncate(1);
    room.camera.distance = 6.5;
    room.camera.pitch_deg = 40.0;
    room.camera.focus = [0.0, 0.4, 0.6];
    let mut brown = room.meshes[1].clone();
    for v in &mut brown.vertices {
        v.color = [40, 22, 10, 255];
    }
    room.meshes.push(brown);
    let white = room.materials.len() as u32;
    let base = room.materials[1].clone();
    room.materials.push(Material {
        name: "white".into(),
        base_color: [1.0; 4],
        ..base.clone()
    });
    room.materials.push(Material {
        name: "cream".into(),
        base_color: [0.85, 0.78, 0.62, 1.0],
        ..base
    });
    for (name, mesh, material, x) in [("dark", 2, white, -0.9), ("light", 1, white + 1, 0.9)] {
        let mut d = Draw {
            name: Some(name.into()),
            mesh,
            material,
            transform: place(x, 0.3),
            casts_shadow: true,
            ..room.draws[0].clone()
        };
        d.bounds = Aabb::around(&room.meshes[mesh as usize], &d.transform);
        room.draws.push(d);
        room.nodes.push(Node {
            name: name.into(),
            pivot: Mat4::IDENTITY.to_cols_array(),
            draws: vec![room.draws.len() as u32 - 1],
            visible: true,
            hinge: None,
        });
    }
    room
}

fn assets() -> (GameAssets, Assets) {
    let text = std::fs::read_to_string(examples().join("climb/actors/hero.json")).unwrap();
    let source = ActorSource::from_json(&text).unwrap();
    let actor = build_actor(&source, &examples().join("collision")).unwrap();
    let pack = room();
    let drawn = GameAssets {
        actors: vec![("hero".into(), sariema_format::actor::encode(&actor))],
        ..GameAssets::default()
    };
    let assets = Assets {
        rooms: vec![RoomAssets {
            name: "demo".into(),
            room: Some(Arc::from(sariema_format::encode(&pack))),
            collision: None,
            index: RoomIndex::of(&pack),
            neighbours: Vec::new(),
            beside: Vec::new(),
            ambience: None,
        }],
        actors: vec![("hero".into(), actor)],
        ..Assets::default()
    };
    (drawn, assets)
}

fn sparkle() -> Effect {
    Effect {
        name: "sparkle".into(),
        sprite: Shape::Star,
        rate: 14.0,
        life: [0.8, 1.1],
        size: [0.14, 0.2],
        grow: vec![[0.0, 0.4], [0.25, 1.0], [1.0, 0.6]],
        speed: [0.1, 0.25],
        spread: 60.0,
        radius: 0.3,
        gravity: [0.0, 0.12, 0.0],
        drag: 1.0,
        spin: [-60.0, 60.0],
        color: vec![
            [0.0, 1.0, 0.85, 0.55, 0.0],
            [0.2, 1.0, 0.85, 0.55, 1.0],
            [0.7, 1.0, 0.8, 0.5, 0.8],
            [1.0, 1.0, 0.7, 0.4, 0.0],
        ],
        intensity: 3.0,
        ..Effect::default()
    }
}

const HERO: Entity = Entity {
    index: 0,
    generation: 0,
};

/// The hero standing at the front left, facing right.
fn hero() -> Vec<(u64, Command<'static>)> {
    vec![
        (
            0,
            Command::Spawn {
                entity: HERO,
                kind: Kind::Actor,
                asset: name_id("hero"),
            },
        ),
        (
            0,
            Command::SetTransform {
                entity: HERO,
                position: [-2.2, 0.0, 1.3],
                yaw: 90.0,
            },
        ),
    ]
}

struct Strip {
    image: RgbaImage,
    pipelines: u32,
    buffers: u32,
    sprites: u32,
}

/// `frames` frames side by side, `every` steps apart, from step `first`.
fn strip(script: Vec<(u64, Command<'static>)>, first: u64, every: u64, frames: u32) -> Strip {
    let (drawn, assets) = assets();
    let mut settings = Settings::default();
    settings.motion.builtin = false;
    settings.fx.effects = vec![sparkle()];
    let sim = Simulation::new(settings, Vec::new(), Script(script, 0), 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let mut h = Headless::new(W, H, Quality::high()).unwrap();
    let mut view = GameView::load(h.renderer(), &drawn, &engine).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let aspect = W as f32 / H as f32;
    let mut image = RgbaImage::new(W * frames, H);
    let mut made = (0, 0);
    let mut sprites = 0;
    for frame in 0..frames {
        let steps = if frame == 0 { first } else { every };
        for _ in 0..steps {
            engine.step(&actions);
        }
        h.paint(|r, target| view.world(r, &mut engine, 1.0, aspect, target))
            .unwrap();
        image::imageops::replace(&mut image, &h.image(), (frame * W) as i64, 0);
        let r = h.renderer();
        if frame == 0 {
            made = (r.pipelines_created(), r.buffers_created());
        }
        sprites = sprites.max(r.last_frame_stats().sprites);
    }
    let r = h.renderer();
    Strip {
        image,
        pipelines: r.pipelines_created() - made.0,
        buffers: r.buffers_created() - made.1,
        sprites,
    }
}

fn sparkles() -> Strip {
    let mut script = hero();
    script.push((
        0,
        Command::FxStart {
            emitter: 1,
            effect: name_id("sparkle"),
            entity: Entity::NONE,
            position: [-0.9, 1.1, 0.3],
            points: PointList::EMPTY,
            seed: 7,
        },
    ));
    strip(script, 30, 12, 4)
}

const ROUTE: [[f32; 3]; 2] = [[-0.2, 0.0, 1.9], [2.4, 0.0, 1.1]];

fn trail() -> Strip {
    let mut script = hero();
    script.push((
        1,
        Command::Trail {
            trail: 1,
            from: HERO,
            points: PointList::Slice(&ROUTE),
            spacing: 1.0,
            size: 0.2,
            color: [1.0, 0.6, 0.2],
            intensity: 0.55,
            wave_speed: 2.5,
            fade_from: 0.8,
            fade_to: 0.8,
            fade: 0.3,
        },
    ));
    strip(script, 30, 15, 4)
}

fn glow() -> Strip {
    let mut script = hero();
    let highlight = |entity, node: &str| Command::Highlight {
        entity,
        node: name_id(node),
        fill: 0.25,
        rim: 0.6,
        pulse: 1.2,
        fade: 0.3,
    };
    script.push((1, highlight(Entity::NONE, "dark")));
    script.push((1, highlight(Entity::NONE, "light")));
    script.push((1, highlight(HERO, "")));
    strip(script, 37, 18, 4)
}

fn matches_golden(strip: &Strip, name: &str) {
    let expected = image::open(golden(name)).unwrap().to_rgba8();
    let d = compare_images(&strip.image, &expected).unwrap();
    if !(d.max_channel_delta <= GOLDEN_MAX_DELTA || d.differing_pixels_ratio <= GOLDEN_MAX_RATIO) {
        let actual = format!("{}/{name}.actual.png", env!("CARGO_TARGET_TMPDIR"));
        strip.image.save(&actual).unwrap();
        panic!("{name}: {d:?}; this render is in {actual}");
    }
}

#[test]
fn a_sparkle_emitter_matches_its_golden_strip() {
    let s = sparkles();
    assert!(s.sprites > 4, "{}", s.sprites);
    assert_eq!((s.pipelines, s.buffers), (0, 0));
    matches_golden(&s, "fx_sparkle");
}

#[test]
fn a_trail_with_its_wave_matches_its_golden_strip() {
    let s = trail();
    assert!(s.sprites >= 3, "{}", s.sprites);
    assert_eq!((s.pipelines, s.buffers), (0, 0));
    matches_golden(&s, "fx_trail");
}

#[test]
fn a_highlight_pulsing_on_a_dark_and_a_light_piece_matches_its_golden_strip() {
    let s = glow();
    assert_eq!((s.pipelines, s.buffers), (0, 0));
    matches_golden(&s, "fx_glow");
}

#[test]
fn the_same_seed_draws_the_same_frames() {
    let (a, b) = (sparkles(), sparkles());
    assert_eq!(
        compare_images(&a.image, &b.image)
            .unwrap()
            .max_channel_delta,
        0
    );
}

#[test]
#[ignore = "writes the goldens; run with --ignored to refresh them"]
fn write_fx_goldens() {
    sparkles().image.save(golden("fx_sparkle")).unwrap();
    trail().image.save(golden("fx_trail")).unwrap();
    glow().image.save(golden("fx_glow")).unwrap();
}
