// SPDX-License-Identifier: Apache-2.0

//! The room's light and look as the game tweens them: drawn at the value
//! between the last two steps that a frame falls on, like every other
//! moving thing, and exactly the room's own while nothing changes them.

use std::path::Path;
use std::sync::Arc;

use sariema_engine::{Assets, Engine, EngineSettings, RoomAssets};
use sariema_format::scenery::RoomIndex;
use sariema_format::script::{Command, Ease, GameAssets, name_id, put_scene_value};
use sariema_import::{RoomSource, build_collision, build_room};
use sariema_render::{Headless, Quality};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};
use sariema_view::{GameView, ViewSettings};

const W: u32 = 96;
const H: u32 = 54;

/// Sends `values` as one `scene_tween` over a second at step 1.
struct Script(Vec<u8>, u64);

impl Logic for Script {
    fn tick(&mut self, _: &[u8], _: &[u32], out: &mut Vec<u8>) -> Result<(), Stop> {
        out.clear();
        if self.1 == 1 && !self.0.is_empty() {
            Command::SceneTween {
                tween: 1,
                values: &self.0,
                duration: 1.0,
                ease: Ease::Linear,
            }
            .write(out);
        }
        self.1 += 1;
        Ok(())
    }
}

fn sill() -> (sariema_format::RoomPack, RoomAssets) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/collision");
    let text = std::fs::read_to_string(root.join("rooms/sill.json")).unwrap();
    let source = RoomSource::from_json(&text).unwrap();
    let pack = build_room(&source, &root).unwrap();
    let assets = RoomAssets {
        name: "sill".into(),
        room: Some(Arc::from(sariema_format::encode(&pack))),
        collision: Some(Arc::new(build_collision(&source, &root).unwrap().unwrap())),
        index: RoomIndex::of(&pack),
        neighbours: Vec::new(),
        beside: Vec::new(),
        ambience: None,
    };
    (pack, assets)
}

struct Played {
    engine: Engine<Script>,
    view: GameView,
    headless: Headless,
}

fn played(values: &[(&str, f32)]) -> Played {
    let mut bytes = Vec::new();
    for (name, value) in values {
        put_scene_value(&mut bytes, name_id(name), *value);
    }
    let (_, room) = sill();
    let mut runtime = Settings::default();
    runtime.motion.builtin = false;
    let sim = Simulation::new(runtime, Vec::new(), Script(bytes, 0), 1);
    let assets = Assets {
        rooms: vec![room],
        ..Assets::default()
    };
    let engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let mut headless = Headless::new(W, H, Quality::high()).unwrap();
    let settings = ViewSettings {
        threads: false,
        ..ViewSettings::default()
    };
    let view = GameView::load_with(
        headless.renderer(),
        &GameAssets::default(),
        &engine,
        settings,
    )
    .unwrap();
    Played {
        engine,
        view,
        headless,
    }
}

impl Played {
    fn step(&mut self, n: u32) {
        let actions = Actions::for_settings(&self.engine.state().settings.input);
        for _ in 0..n {
            self.engine.step(&actions);
        }
    }

    fn draw(&mut self, alpha: f32) {
        let (view, engine) = (&mut self.view, &mut self.engine);
        self.headless
            .paint(|r, target| view.world(r, engine, alpha, W as f32 / H as f32, target))
            .unwrap();
    }

    fn param(&self, name: &str) -> (f32, f32) {
        let room = &self.engine.state().scenery.rooms()[0];
        let at = room.param(name_id(name)).unwrap();
        (room.previous[at], room.params[at])
    }
}

#[test]
fn a_frame_between_two_steps_draws_the_value_between_them() {
    let (pack, _) = sill();
    let to = pack.look.exposure * 3.0;
    let mut p = played(&[
        ("look.exposure", to),
        ("fog.density", 0.2),
        ("sun.intensity", 0.0),
    ]);
    p.step(20);
    let (before, after) = p.param("look.exposure");
    assert!(before < after && after < to, "{before} {after}");
    let (fog_before, fog_after) = p.param("fog.density");
    let (sun_before, sun_after) = p.param("sun.intensity");
    for alpha in [0.0, 0.25, 0.5, 1.0] {
        p.draw(alpha);
        let renderer = p.headless.renderer();
        let look = renderer.look().unwrap();
        let expected = before + (after - before) * alpha;
        assert!(
            (look.exposure - expected).abs() < 1e-5,
            "at {alpha}: {} against {expected}",
            look.exposure
        );
        let lighting = renderer.lighting().unwrap();
        let fog = fog_before + (fog_after - fog_before) * alpha;
        assert!(
            (lighting.fog.density - fog).abs() < 1e-6,
            "{:?}",
            lighting.fog
        );
        let sun = sun_before + (sun_after - sun_before) * alpha;
        let drawn = lighting.sun.as_ref().unwrap().intensity;
        assert!((drawn - sun).abs() < 1e-4, "sun {drawn} against {sun}");
    }
}

#[test]
fn while_nothing_changes_the_room_is_drawn_exactly_as_its_pack_has_it() {
    let (pack, _) = sill();
    let mut p = played(&[]);
    p.step(5);
    for alpha in [0.0, 0.37, 1.0] {
        p.draw(alpha);
        let renderer = p.headless.renderer();
        assert_eq!(renderer.look(), Some(&pack.look));
        let lighting = renderer.lighting().unwrap();
        assert_eq!(lighting.ambient, pack.ambient.sh9);
        assert_eq!(lighting.sun, pack.sun);
        assert_eq!(lighting.fog, pack.fog);
    }
}

#[test]
fn the_hemisphere_turns_the_ambient_light_it_reads() {
    let mut p = played(&[
        ("hemisphere.sky.r", 2.0),
        ("hemisphere.sky.g", 0.0),
        ("hemisphere.sky.b", 0.0),
        ("hemisphere.ground.r", 0.0),
        ("hemisphere.ground.g", 0.0),
        ("hemisphere.ground.b", 1.0),
    ]);
    p.step(70);
    p.draw(1.0);
    let lighting = p.headless.renderer().lighting().unwrap();
    let up = sariema_format::eval_sh9(&lighting.ambient, [0.0, 1.0, 0.0]);
    let down = sariema_format::eval_sh9(&lighting.ambient, [0.0, -1.0, 0.0]);
    let (pack, _) = sill();
    let mut band2 = [[0.0; 3]; 9];
    band2[4..].copy_from_slice(&pack.ambient.sh9[4..]);
    let rest_up = sariema_format::eval_sh9(&band2, [0.0, 1.0, 0.0]);
    let rest_down = sariema_format::eval_sh9(&band2, [0.0, -1.0, 0.0]);
    for k in 0..3 {
        let sky = [2.0, 0.0, 0.0][k] + rest_up[k];
        let ground = [0.0, 0.0, 1.0][k] + rest_down[k];
        assert!((up[k] - sky).abs() < 1e-4, "up {up:?}");
        assert!((down[k] - ground).abs() < 1e-4, "down {down:?}");
    }
}
