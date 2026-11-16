// SPDX-License-Identifier: Apache-2.0

//! Two rooms of a game pack drawn across switches: the next room is loaded
//! a share at a time before the game asks for it, every switch shows no
//! black frame and creates no pipeline, a dissolve mixes the two rooms
//! and a continuous switch draws both whole.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::RgbaImage;
use sariema_engine::{Assets, Engine, EngineSettings, RoomAssets};
use sariema_format::scenery::RoomIndex;
use sariema_format::script::{Command, Entity, GameAssets, Kind, RoomTransition, name_id, records};
use sariema_import::{ActorSource, RoomSource, build_actor, build_collision, build_room};
use sariema_render::{Headless, Quality};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};
use sariema_view::{GameView, RoomsFrame, ViewSettings};

const W: u32 = 192;
const H: u32 = 108;

struct Script(Vec<(u64, Command<'static>)>, u64);

impl Logic for Script {
    fn tick(&mut self, _: &[u8], _: &[u32], out: &mut Vec<u8>) -> Result<(), Stop> {
        out.clear();
        for (_, c) in self.0.iter().filter(|(n, _)| *n == self.1) {
            c.write(out);
        }
        let _ = records::<sariema_format::script::Event>;
        self.1 += 1;
        Ok(())
    }
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// `sill` and `hall`, the same room with its exposure lowered in the hall
/// and, with `hall_red`, its materials reddened, so the two look apart; each
/// the other's neighbour.
fn rooms(hall_red: bool, beside: bool) -> Vec<RoomAssets> {
    let root = examples().join("collision");
    let room = |name: &str, neighbour: u32| {
        let text = std::fs::read_to_string(root.join(format!("rooms/{name}.json"))).unwrap();
        let source = RoomSource::from_json(&text).unwrap();
        let mut pack = build_room(&source, &root).unwrap();
        if name == "hall" && hall_red {
            for m in &mut pack.materials {
                m.base_color = [0.9, 0.15, 0.1, m.base_color[3]];
            }
        }
        if name == "hall" {
            pack.look.exposure = 0.4;
        }
        RoomAssets {
            name: name.into(),
            room: Some(Arc::from(sariema_format::encode(&pack))),
            collision: Some(Arc::new(build_collision(&source, &root).unwrap().unwrap())),
            index: RoomIndex::of(&pack),
            neighbours: vec![neighbour],
            beside: if beside { vec![neighbour] } else { Vec::new() },
            ambience: None,
        }
    };
    vec![room("sill", 1), room("hall", 0)]
}

fn hero(hall_red: bool, beside: bool) -> (GameAssets, Assets) {
    let text = std::fs::read_to_string(examples().join("climb/actors/hero.json")).unwrap();
    let actor = build_actor(
        &ActorSource::from_json(&text).unwrap(),
        &examples().join("collision"),
    )
    .unwrap();
    let drawn = GameAssets {
        actors: vec![("hero".into(), sariema_format::actor::encode(&actor))],
        ..GameAssets::default()
    };
    let assets = Assets {
        rooms: rooms(hall_red, beside),
        actors: vec![("hero".into(), actor)],
        ..Assets::default()
    };
    (drawn, assets)
}

fn switch(at: u64, room: &str, transition: RoomTransition) -> (u64, Command<'static>) {
    let room = name_id(room);
    let duration = 0.5;
    (
        at,
        Command::RoomSwitch {
            room,
            transition,
            duration,
        },
    )
}

const HERO: Entity = Entity {
    index: 0,
    generation: 0,
};

/// The hero walks while the game switches to the hall (dissolving), back
/// (continuous) and to the hall again (cut).
fn script() -> Script {
    Script(
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
                    position: [0.6, 0.0, -5.0],
                    yaw: -90.0,
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
            switch(90, "hall", RoomTransition::Dissolve),
            switch(180, "sill", RoomTransition::Continuous),
            switch(270, "hall", RoomTransition::Cut),
        ],
        0,
    )
}

struct Frame {
    image: RgbaImage,
    rooms: RoomsFrame,
    pipelines: u32,
    draws: u32,
    active: usize,
}

fn play(settings: ViewSettings, frames: u64, hall_red: bool) -> Vec<Frame> {
    play_with(settings, frames, hall_red, false)
}

fn play_with(settings: ViewSettings, frames: u64, hall_red: bool, beside: bool) -> Vec<Frame> {
    run(settings, frames, (hall_red, beside), script(), 1)
}

/// `frames` frames of `steps` steps each.
fn run(
    settings: ViewSettings,
    frames: u64,
    (hall_red, beside): (bool, bool),
    script: Script,
    steps: u64,
) -> Vec<Frame> {
    let (drawn, assets) = hero(hall_red, beside);
    let mut runtime = Settings::default();
    runtime.motion.builtin = false;
    let sim = Simulation::new(runtime, Vec::new(), script, 1);
    let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let mut h = Headless::new(W, H, Quality::high()).unwrap();
    let mut view = GameView::load_with(h.renderer(), &drawn, &engine, settings).unwrap();
    let actions = Actions::for_settings(&engine.state().settings.input);
    let aspect = W as f32 / H as f32;
    (0..frames)
        .map(|_| {
            for _ in 0..steps {
                engine.step(&actions);
            }
            h.paint(|r, target| view.world(r, &mut engine, 1.0, aspect, target))
                .unwrap();
            Frame {
                image: h.image(),
                rooms: view.rooms_frame(),
                pipelines: h.renderer().pipelines_created(),
                draws: h.renderer().last_frame_stats().draws,
                active: engine.state().scenery.active(),
            }
        })
        .collect()
}

fn luma(image: &RgbaImage) -> f32 {
    let sum: f32 = (image.pixels())
        .map(|p| 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32)
        .sum();
    sum / (image.width() * image.height()) as f32 / 255.0
}

fn spread() -> ViewSettings {
    ViewSettings {
        threads: false,
        decode_bytes_per_frame: 1024,
        upload_bytes_per_frame: 1024,
        world_ms_per_frame: 0.0,
        grid_columns_per_share: 500,
        ..ViewSettings::default()
    }
}

#[test]
fn the_neighbours_collision_world_is_built_a_share_a_frame_before_the_switch() {
    let frames = play(spread(), 90, true);
    let shares = frames.iter().filter(|f| f.rooms.world_share).count();
    assert!(
        shares > 10,
        "the hall's world is built over {shares} frames"
    );
    assert_eq!(
        frames.iter().map(|f| f.rooms.worlds_built).sum::<u32>(),
        1,
        "the hall's, before the switch"
    );
}

#[test]
fn switching_rooms_shows_no_black_frame_and_creates_no_pipeline() {
    let frames = play(spread(), 300, true);
    let darker = luma(&frames[0].image).min(luma(&frames[170].image));
    for (n, f) in frames.iter().enumerate() {
        let l = luma(&f.image);
        assert!(l > 0.8 * darker, "frame {n} has luma {l} against {darker}");
        assert!(!f.rooms.loaded_at_once, "frame {n} loaded a room at once");
        assert_eq!(
            f.pipelines, frames[0].pipelines,
            "frame {n} created a pipeline"
        );
    }
    assert_eq!(frames[89].active, 0);
    assert_eq!(frames[90].active, 1);
    assert_eq!(frames[180].active, 0);
    assert_eq!(frames[270].active, 1);
}

#[test]
fn the_neighbour_loads_a_share_a_frame_before_the_switch() {
    let frames = play(spread(), 90, true);
    let loading: Vec<&RoomsFrame> = (frames.iter().map(|f| &f.rooms))
        .filter(|r| r.bytes_decoded > 0 || r.bytes_uploaded > 0)
        .collect();
    assert!(
        loading.len() >= 4,
        "the hall loads over {} frames",
        loading.len()
    );
    assert_eq!(frames.iter().map(|f| f.rooms.rooms_loaded).sum::<u32>(), 1);
    assert_eq!(
        frames[89].rooms.rooms_resident, 2,
        "both rooms are in before the switch"
    );
    let hall = &rooms(true, false)[1];
    let pack = sariema_format::decode(hall.room.as_deref().unwrap()).unwrap();
    let textures = pack
        .textures
        .iter()
        .map(|t| t.mips.iter().map(Vec::len).sum::<usize>());
    let meshes = (pack.meshes.iter())
        .map(|m| m.vertices.len() * size_of::<sariema_format::Vertex>() + m.indices.len() * 5);
    let part = textures.chain(meshes).max().unwrap_or(0) as u64;
    let most = 1024 + part;
    for r in &loading {
        assert!(
            r.bytes_decoded <= most && r.bytes_uploaded <= most,
            "{r:?} past {most}"
        );
    }
}

#[test]
fn a_dissolve_fades_the_last_frame_of_the_room_left_over_the_next() {
    let frames = play(spread(), 240, true);
    let (sill, hall) = (&frames[85].image, &frames[150].image);
    let red = |p: &image::Rgba<u8>| p[0] as f32 - p[1] as f32;
    let room: Vec<usize> = (sill.pixels().zip(hall.pixels()).enumerate())
        .filter(|(_, (a, b))| red(b) - red(a) > 40.0)
        .map(|(i, _)| i)
        .collect();
    assert!(
        room.len() > 400,
        "the rooms differ in {} pixels",
        room.len()
    );
    let pixels = |image: &RgbaImage| -> Vec<image::Rgba<u8>> { image.pixels().copied().collect() };
    let (sill, hall) = (pixels(sill), pixels(hall));
    // How far each frame is from the sill to the hall, over the pixels that tell them apart.
    let towards_hall = |n: usize| {
        let frame = pixels(&frames[n].image);
        let along: f32 = (room.iter())
            .map(|&i| (red(&frame[i]) - red(&sill[i])) / (red(&hall[i]) - red(&sill[i])))
            .sum();
        along / room.len() as f32
    };
    let dissolve: Vec<f32> = (88..125).map(towards_hall).collect();
    assert!(
        dissolve[1].abs() < 0.1,
        "the sill whole until the switch: {dissolve:?}"
    );
    let middle = towards_hall(90 + 15);
    assert!(
        (0.3..0.7).contains(&middle),
        "halfway through the dissolve: {middle}"
    );
    assert!(
        dissolve.last().unwrap() > &0.9,
        "the hall whole after it: {dissolve:?}"
    );
    for pair in dissolve.windows(2) {
        assert!(
            pair[1] - pair[0] < 0.2 && pair[1] >= pair[0] - 0.02,
            "the dissolve moves smoothly: {dissolve:?}"
        );
    }
    let alone = frames[150].draws;
    assert!(
        frames[91..119].iter().all(|f| f.draws == alone + 1),
        "only the hall is drawn, and the frame held over it: {:?} against {alone}",
        frames[91..119].iter().map(|f| f.draws).collect::<Vec<_>>()
    );
}

#[test]
fn a_dissolve_with_no_duration_shows_on_a_host_drawing_every_other_step() {
    let mut script = script();
    script.0.retain(|(n, _)| *n < 90);
    script.0.push((
        90,
        Command::RoomSwitch {
            room: name_id("hall"),
            transition: RoomTransition::Dissolve,
            duration: 0.0,
        },
    ));
    let frames = run(spread(), 80, (true, false), script, 2);
    let alone = frames[70].draws;
    let draws: Vec<u32> = frames[44..62].iter().map(|f| f.draws).collect();
    assert!(
        frames[45].draws > alone && frames[46..59].iter().all(|f| f.draws == alone + 1),
        "the frame of steps 90 and 91 and the next ones show the sill held over the hall for \
         dissolve_s: {draws:?} against {alone}"
    );
    assert_eq!(frames[59].draws, alone, "over after 30 steps: {draws:?}");
    let apart = |a: &RgbaImage, b: &RgbaImage| -> f32 {
        let sum: f32 = (a.pixels().zip(b.pixels()))
            .map(|(p, q)| (p[0] as f32 - q[0] as f32).abs())
            .sum();
        sum / (a.width() * a.height()) as f32
    };
    let (before, switched, after) = (&frames[44].image, &frames[45].image, &frames[70].image);
    assert!(
        apart(switched, before) < apart(switched, after) / 4.0,
        "the switch's frame is the sill's: {} from it, {} from the hall",
        apart(switched, before),
        apart(switched, after)
    );
}

#[test]
fn with_one_room_resident_a_switch_loads_the_next_at_once() {
    let settings = ViewSettings {
        max_resident: 1,
        ..spread()
    };
    let frames = play(settings, 100, true);
    assert!(!frames[89].rooms.loaded_at_once);
    assert!(
        frames[90].rooms.loaded_at_once,
        "the hall was not loaded before"
    );
    let hall = luma(&play(spread(), 170, true)[169].image);
    assert!(
        luma(&frames[90].image) > 0.8 * hall,
        "the hall alone, but no black frame"
    );
}

#[test]
fn a_continuous_switch_draws_both_rooms_and_blends_their_light() {
    let frames = play(spread(), 240, false);
    let lumas: Vec<f32> = (175..215).map(|n| luma(&frames[n].image)).collect();
    let step = (lumas[0] - lumas[39]).abs();
    assert!(step > 0.02, "the two rooms' exposures differ: {lumas:?}");
    for pair in lumas.windows(2) {
        assert!(
            (pair[1] - pair[0]).abs() < 0.15 * step,
            "the light moves smoothly: {lumas:?}"
        );
    }
    let alone = frames[175].draws.max(frames[214].draws);
    assert!(
        (182..208).all(|n| frames[n].draws > alone),
        "both rooms are drawn while it lasts"
    );
}

#[test]
fn rooms_beside_the_active_one_are_drawn_with_it_all_along() {
    let alone = play(spread(), 240, false);
    let beside = play_with(spread(), 240, false, true);
    for n in [30, 60, 89, 100, 150, 200, 239] {
        assert!(
            beside[n].draws > alone[n].draws.min(alone[30].draws),
            "frame {n}: {} draws beside, {} alone",
            beside[n].draws,
            alone[n].draws
        );
    }
}

#[test]
fn a_room_beside_the_first_one_is_drawn_from_the_first_frame() {
    let alone = play(spread(), 3, false);
    let beside = play_with(spread(), 3, false, true);
    assert!(
        beside[0].draws > alone[0].draws,
        "{} draws beside, {} alone",
        beside[0].draws,
        alone[0].draws
    );
    assert!(
        beside.iter().all(|f| !f.rooms.loaded_at_once),
        "loaded with the first room, not by a frame"
    );
}
