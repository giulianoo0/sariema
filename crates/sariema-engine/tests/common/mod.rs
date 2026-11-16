// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sariema_engine::{Assets, Engine, EngineSettings};
use sariema_format::script::{Command, Entity, Event, Hud, records};
use sariema_import::{ActorSource, RoomSource, build_actor, build_collision};
use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};

/// Game logic written in Rust for tests: gets the step and its events,
/// writes commands.
pub struct Native<F> {
    step: u64,
    f: F,
}

pub fn native<F: FnMut(u64, &[Event<'_>], &mut Vec<u8>)>(f: F) -> Native<F> {
    Native { step: 0, f }
}

impl<F: FnMut(u64, &[Event<'_>], &mut Vec<u8>)> Logic for Native<F> {
    fn tick(&mut self, events: &[u8], _ends: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        commands.clear();
        let events: Vec<Event> = records::<Event>(events).map(|r| r.unwrap()).collect();
        (self.f)(self.step, &events, commands);
        self.step += 1;
        Ok(())
    }
}

pub fn send(out: &mut Vec<u8>, command: Command<'_>) {
    command.write(out);
}

pub fn entity(index: u32) -> Entity {
    Entity {
        index,
        generation: 0,
    }
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// The `climb` example's room collision, actor, sounds and HUD, built from
/// their sources once per test binary.
pub fn assets() -> Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS
        .get_or_init(|| {
            let root = example().join("collision");
            let room = std::fs::read_to_string(root.join("rooms/sill.json")).unwrap();
            let collision = build_collision(&RoomSource::from_json(&room).unwrap(), &root)
                .unwrap()
                .unwrap();
            let actor = std::fs::read_to_string(example().join("climb/actors/hero.json")).unwrap();
            let hero = build_actor(&ActorSource::from_json(&actor).unwrap(), &root).unwrap();
            let audio = example().join("audio/audio.json");
            let project = serde_json::from_str(&std::fs::read_to_string(&audio).unwrap()).unwrap();
            let bank = sariema_audio::build(&project, audio.parent().unwrap()).unwrap();
            let hud = example().join("climb/hud.json");
            let font = "../../crates/sariema-ui/fonts/Nunito.ttf";
            Assets {
                room: None,
                collision: Some(collision),
                actors: vec![("hero".into(), hero)],
                sounds: Some(bank),
                hud: Some(Hud {
                    description: std::fs::read_to_string(&hud).unwrap(),
                    files: vec![(
                        font.into(),
                        std::fs::read(hud.parent().unwrap().join(font)).unwrap(),
                    )],
                }),
            }
        })
        .clone()
}

/// Runtime settings for a game whose actors the collision module moves.
pub fn wired() -> Settings {
    let mut settings = Settings::default();
    settings.motion.builtin = false;
    settings
}

pub fn engine<L: Logic>(logic: L, settings: Settings, cutscenes: &[&str]) -> Engine<L> {
    let cutscenes = cutscenes
        .iter()
        .map(|c| compile_json(c).unwrap().0)
        .collect();
    let sim = Simulation::new(settings, cutscenes, logic, 1);
    Engine::new(sim, assets(), EngineSettings::default()).unwrap()
}

/// Spawns the hero, drawn with the `climb` actor, at `at` facing `yaw`.
pub fn spawn_hero(out: &mut Vec<u8>, at: [f32; 3], yaw: f32) {
    send(
        out,
        Command::Spawn {
            entity: entity(0),
            kind: sariema_format::script::Kind::Actor,
            asset: sariema_format::script::name_id("hero"),
        },
    );
    send(
        out,
        Command::SetTransform {
            entity: entity(0),
            position: at,
            yaw,
        },
    );
}
