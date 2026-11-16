// SPDX-License-Identifier: Apache-2.0

//! A room's cutaway as the view draws it: the line between the last two
//! steps that a frame falls on, the height the game set, the camera's side
//! and the hero, with every pipeline made before play.

use std::path::Path;
use std::sync::Arc;

use sariema_engine::{Assets, Engine, EngineSettings, RoomAssets};
use sariema_format::scenery::RoomIndex;
use sariema_format::script::{Command, Controller, Entity, GameAssets, Kind, name_id};
use sariema_format::{CutLine, CutStyle, Cutaway};
use sariema_import::{ActorSource, RoomSource, build_actor, build_room};
use sariema_render::{Headless, Quality};
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};
use sariema_view::{GameView, ViewSettings};

const W: u32 = 96;
const H: u32 = 54;
const HEIGHT: f32 = 0.4;

const HERO: Entity = Entity {
    index: 0,
    generation: 0,
};

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

/// The hero walks towards the camera's side of the room, the line easing
/// after it.
fn script() -> Script {
    let at = |step, c| (step, c);
    Script(
        vec![
            at(
                0,
                Command::Spawn {
                    entity: HERO,
                    kind: Kind::Actor,
                    asset: name_id("hero"),
                },
            ),
            at(
                0,
                Command::SetHero {
                    entity: HERO,
                    speed: 2.0,
                },
            ),
            at(
                0,
                Command::Control {
                    by: Controller::Script,
                },
            ),
            at(
                0,
                Command::SetTransform {
                    entity: HERO,
                    position: [0.6, 0.0, -5.0],
                    yaw: 0.0,
                },
            ),
            at(
                2,
                Command::MoveTo {
                    entity: HERO,
                    target: [0.6, 0.0, -2.0],
                    speed: 1.5,
                },
            ),
        ],
        0,
    )
}

struct Played {
    engine: Engine<Script>,
    view: GameView,
    headless: Headless,
}

/// The `sill` room, without its collision so the hero walks straight, with
/// its first draw in a cutaway whose line sits by the hero and follows it
/// over half a second.
fn played() -> Played {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let root = examples.join("collision");
    let text = std::fs::read_to_string(root.join("rooms/sill.json")).unwrap();
    let source = RoomSource::from_json(&text).unwrap();
    let mut pack = build_room(&source, &root).unwrap();
    pack.cutaways.push(Cutaway {
        name: "walls".into(),
        draws: vec![0],
        height: HEIGHT,
        line: CutLine {
            areas: Vec::new(),
            offset: 0.0,
            follow: 0.5,
        },
        radius: 0.0,
        soft: 0.0,
        cap: None,
        cut_shadows: true,
        anchors: vec![None],
        style: CutStyle::Cut,
    });
    let room = RoomAssets {
        name: "sill".into(),
        room: Some(Arc::from(sariema_format::encode(&pack))),
        collision: None,
        index: RoomIndex::of(&pack),
        neighbours: Vec::new(),
        beside: Vec::new(),
        ambience: None,
    };
    let text = std::fs::read_to_string(examples.join("climb/actors/hero.json")).unwrap();
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
    let sim = Simulation::new(Settings::default(), Vec::new(), script(), 1);
    let engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
    let mut headless = Headless::new(W, H, Quality::high()).unwrap();
    let settings = ViewSettings {
        threads: false,
        ..ViewSettings::default()
    };
    let view = GameView::load_with(headless.renderer(), &drawn, &engine, settings).unwrap();
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
}

#[test]
fn a_frame_between_two_steps_cuts_at_the_line_between_them() {
    let mut p = played();
    let made = p.headless.renderer().pipelines_created();
    p.step(40);
    let room = &p.engine.state().scenery.rooms()[0];
    let (before, now) = (room.lines_before[0], room.lines[0]);
    assert!(before < now, "the line follows the hero: {before} {now}");
    let world = &p.engine.state().world;
    let hero = |alpha| world.interpolated(HERO, alpha).unwrap().position;
    let heroes = [0.0, 0.25, 0.5, 1.0].map(|alpha| (alpha, hero(alpha)));
    for (alpha, hero) in heroes {
        p.draw(alpha);
        let cut = p.headless.renderer().cutaway(0, 0).unwrap();
        let line = before + (now - before) * alpha;
        assert!(
            (cut.line - line).abs() < 1e-5,
            "at {alpha}: {} against {line}",
            cut.line
        );
        assert_eq!(cut.height, HEIGHT);
        assert!((cut.toward[0]).abs() < 1e-6 && (cut.toward[1] - 1.0).abs() < 1e-6);
        assert_eq!(cut.hero, [hero.x, hero.z]);
    }
    assert_eq!(
        p.headless.renderer().pipelines_created(),
        made,
        "no pipeline while playing"
    );
}

#[test]
fn a_cutaway_cuts_nothing_until_its_line_is_placed() {
    let mut p = played();
    p.draw(1.0);
    assert_eq!(p.headless.renderer().cutaway(0, 0), None, "no hero yet");
    p.step(1);
    p.draw(1.0);
    assert!(
        p.headless.renderer().cutaway(0, 0).is_some(),
        "placed with the hero"
    );
}
