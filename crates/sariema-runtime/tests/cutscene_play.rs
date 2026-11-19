// SPDX-License-Identifier: Apache-2.0

mod common;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use glam::Vec3;
use sariema_format::script::{Advance, Command, Controller, Ease, Event, Kind, name_id};
use sariema_runtime::cutscene::author::compile_json;
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::Simulation;

use common::{entity, native, send};

const HERO: u32 = 0;
const FRIEND: u32 = 1;
const SETTLE: u64 = 120;

fn fixture(name: &str) -> sariema_format::script::cutscene::Cutscene {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cutscenes")
        .join(name);
    compile_json(&std::fs::read_to_string(path).unwrap())
        .unwrap()
        .0
}

#[derive(Clone, Debug, Default)]
struct Seen {
    events: Vec<(u64, String)>,
    ended: Option<u64>,
}

#[derive(Clone, Debug)]
struct Frame {
    hero: Vec3,
    friend: Vec3,
    hero_yaw: f32,
    covered: bool,
    cut: bool,
    letterbox: f32,
    control: Controller,
}

struct Run {
    frames: Vec<Frame>,
    seen: Seen,
    world_hash: u64,
    body_start: Option<u64>,
}

/// Plays `meet` with a game that spawns the hero far from its start and the
/// friend near its start, answers the `greeted` call with the `door`
/// signal, and skips at `skip_at` when given. Taps confirm on every tap line.
fn play(skip_at: Option<u64>, press_at_end: bool) -> Run {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let log = seen.clone();
    let mut signal_at = None;
    let logic = native(move |step, events, out| {
        for e in events {
            let mut log = log.borrow_mut();
            log.events.push((step, format!("{e:?}")));
            match *e {
                Event::CutsceneCall { call, .. } if call == name_id("greeted") => {
                    signal_at = Some(step + 10)
                }
                Event::CutsceneEnded { .. } => log.ended = Some(step),
                _ => {}
            }
        }
        let (hero, friend) = (entity(HERO), entity(FRIEND));
        if step == 0 {
            send(
                out,
                Command::Spawn {
                    entity: hero,
                    kind: Kind::Actor,
                    asset: name_id("hero"),
                },
            );
            send(
                out,
                Command::SetTransform {
                    entity: hero,
                    position: [0.0, 0.0, 0.0],
                    yaw: 180.0,
                },
            );
            send(
                out,
                Command::Spawn {
                    entity: friend,
                    kind: Kind::Actor,
                    asset: name_id("friend"),
                },
            );
            send(
                out,
                Command::SetTransform {
                    entity: friend,
                    position: [1.6, 0.0, -3.05],
                    yaw: 0.0,
                },
            );
            send(
                out,
                Command::SetHero {
                    entity: hero,
                    speed: 2.0,
                },
            );
            send(
                out,
                Command::RigFollow {
                    rig: 1,
                    subject: hero,
                    distance: 6.0,
                    pitch: 40.0,
                    yaw: 0.0,
                    height: 1.0,
                    fov: 45.0,
                },
            );
            send(
                out,
                Command::CameraUse {
                    rig: 1,
                    blend: 0.0,
                    ease: Ease::Linear,
                },
            );
        }
        if step == 1 {
            send(
                out,
                Command::CutsceneBind {
                    role: name_id("hero"),
                    entity: hero,
                },
            );
            send(
                out,
                Command::CutsceneBind {
                    role: name_id("friend"),
                    entity: friend,
                },
            );
            send(
                out,
                Command::CutscenePlay {
                    cutscene: name_id("meet"),
                },
            );
        }
        if signal_at == Some(step) {
            send(
                out,
                Command::Signal {
                    signal: name_id("door"),
                },
            );
        }
        if skip_at == Some(step) {
            send(out, Command::CutsceneSkip {});
        }
    });
    let settings = Settings::default();
    let confirm = settings.input.button("confirm").unwrap() as usize;
    let mut sim = Simulation::new(settings.clone(), vec![fixture("meet.json")], logic, 7);
    let mut frames = Vec::new();
    let mut line_up = 0;
    let mut body_start = None;
    for step in 0..3000u64 {
        let mut actions = Actions::for_settings(&settings.input);
        let tap = sim
            .state()
            .dialogue
            .line
            .as_ref()
            .is_some_and(|l| l.advance == Advance::Tap);
        line_up = if tap { line_up + 1 } else { 0 };
        actions.buttons[confirm] = line_up == 8;
        let ending = sim.playing_cutscene().is_some()
            && sim.state().screen.letterbox.target() == 0.0
            && step > 30;
        if press_at_end && ending {
            actions.buttons[confirm] = step % 2 == 0;
        }
        sim.step(&actions);
        let s = sim.state();
        if body_start.is_none() && s.screen.letterbox.value() > 0.0 {
            body_start = Some(step);
        }
        let pose = |i| s.world.pose(entity(i)).unwrap();
        frames.push(Frame {
            hero: pose(HERO).position,
            friend: pose(FRIEND).position,
            hero_yaw: s.world.node(entity(HERO)).unwrap().local.yaw,
            covered: s.screen.covered(),
            cut: s.camera.cut(),
            letterbox: s.screen.letterbox.value(),
            control: s.control.by,
        });
        if seen.borrow().ended.is_some_and(|end| step > end + SETTLE) {
            break;
        }
    }
    let seen = seen.borrow().clone();
    assert!(seen.ended.is_some(), "the cutscene never ended");
    Run {
        frames,
        world_hash: sim.state().world_hash(),
        seen,
        body_start,
    }
}

#[test]
fn played_through_it_runs_every_block_and_never_jumps_an_actor() {
    let run = play(None, false);
    for (step, pair) in run.frames.windows(2).enumerate() {
        let (a, b) = (&pair[0], &pair[1]);
        for (what, from, to) in [("hero", a.hero, b.hero), ("friend", a.friend, b.friend)] {
            let moved = from.distance(to);
            // The frame before was fully covered, or this one follows a cut.
            assert!(
                moved < 0.05 || a.covered || b.cut,
                "step {}: {what} jumped {moved} m on screen",
                step + 1
            );
        }
    }
    let events: Vec<&str> = run.seen.events.iter().map(|(_, e)| e.as_str()).collect();
    let has = |needle: &str| events.iter().any(|e| e.contains(needle));
    assert!(has("CutsceneStarted"));
    assert!(has(&format!(
        "CutsceneCall {{ cutscene: {}, call: {}, skipped: false }}",
        name_id("meet"),
        name_id("greeted")
    )));
    assert!(!has(&format!("call: {},", name_id("never"))));
    assert!(has(&format!(
        "FlagChanged {{ flag: {}, on: true }}",
        name_id("met")
    )));
    assert!(has("CutsceneEnded") && has("skipped: false"));
    let last = run.frames.last().unwrap();
    assert_eq!(
        last.friend,
        Vec3::new(4.0, 0.0, -6.0),
        "the teleport under the fade happened"
    );
    assert_eq!(
        last.hero,
        Vec3::new(0.0, 0.0, -2.5),
        "the hero stays where the cutscene left them"
    );
    assert_eq!(last.letterbox, 0.0);
    assert_eq!(last.control, Controller::Player);
}

#[test]
fn an_actor_away_from_its_start_is_walked_there_never_snapped() {
    let run = play(None, false);
    let start = run.body_start.expect("the body never started") as usize;
    let placed = &run.frames[start - 1];
    assert_eq!(
        placed.hero,
        Vec3::new(0.0, 0.0, -2.0),
        "the body starts with the hero on its start place"
    );
    assert_eq!(placed.hero_yaw, 0.0);
    assert!(
        start > 70,
        "a 2 m walk at 1.5 m/s takes 80 steps, the body started at {start}"
    );
    assert_eq!(
        placed.friend,
        Vec3::new(1.5, 0.0, -3.0),
        "the friend was eased onto its start"
    );
    let placing = &run.frames[..start];
    assert!(
        placing
            .iter()
            .all(|f| !f.covered && !f.cut || placing[0].cut),
        "placement happens in view"
    );
}

#[test]
fn skipping_at_any_point_gives_the_same_end_state_as_playing_it_through() {
    let played = play(None, false);
    let end = played.seen.ended.unwrap();
    for skip_at in [3, 40, 100, 150, 200, end - 12] {
        let skipped = play(Some(skip_at), false);
        assert!(
            skipped
                .seen
                .events
                .iter()
                .any(|(_, e)| e.contains("skipped: true")),
            "skip at {skip_at} did not skip"
        );
        assert_eq!(
            skipped.world_hash, played.world_hash,
            "skip at step {skip_at}"
        );
        for (step, pair) in skipped.frames.windows(2).enumerate() {
            let moved = pair[0]
                .hero
                .distance(pair[1].hero)
                .max(pair[0].friend.distance(pair[1].friend));
            assert!(
                moved < 0.05 || pair[0].covered || pair[1].cut,
                "skip at {skip_at}: jump at step {}",
                step + 1
            );
        }
    }
}

#[test]
fn control_returns_to_the_player_with_the_buffer_cleared_and_the_camera_blending_back() {
    let run = play(None, true);
    let end = run.seen.ended.unwrap();
    let after: Vec<&String> = run
        .seen
        .events
        .iter()
        .filter(|(step, _)| *step > end)
        .map(|(_, e)| e)
        .collect();
    assert!(
        !after.iter().any(|e| e.starts_with("Buffered")),
        "a press made during the cutscene was still buffered after it: {after:?}"
    );
    let controls: Vec<&str> = run
        .seen
        .events
        .iter()
        .filter(|(_, e)| e.starts_with("Control"))
        .map(|(_, e)| e.as_str())
        .collect();
    assert_eq!(
        controls,
        ["Control { by: Cutscene }", "Control { by: Player }"]
    );
    let ended = end as usize;
    assert!(
        !run.frames[ended..].iter().any(|f| f.cut),
        "the camera cut back instead of blending"
    );
}
