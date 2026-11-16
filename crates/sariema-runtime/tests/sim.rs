// SPDX-License-Identifier: Apache-2.0

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use glam::Vec2;
use sariema_format::script::{Command, Controller, Ease, Entity, Event, Kind, name_id};
use sariema_runtime::input::{Actions, InputLog, Replay};
use sariema_runtime::outbox::{HostIn, HostOut};
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};

use common::{entity, native, send};

/// A small game: a hero the input moves, 50 props driven by tweens and a
/// seeded random walk the game computes itself, a timer and a zone.
fn game() -> impl Logic {
    let mut rng = 0u64;
    native(move |step, events, out| {
        for e in events {
            if let Event::Started { seed } = e {
                rng = *seed;
            }
        }
        if step == 0 {
            send(
                out,
                Command::Spawn {
                    entity: entity(0),
                    kind: Kind::Actor,
                    asset: 1,
                },
            );
            send(
                out,
                Command::SetHero {
                    entity: entity(0),
                    speed: 3.0,
                },
            );
            send(
                out,
                Command::RigFollow {
                    rig: 1,
                    subject: entity(0),
                    distance: 6.0,
                    pitch: 45.0,
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
            send(
                out,
                Command::Timer {
                    timer: 1,
                    duration: 0.5,
                    repeat: true,
                },
            );
            send(
                out,
                Command::Zone {
                    zone: 9,
                    center: [0.0, 0.0, -4.0],
                    radius: 1.0,
                },
            );
            for i in 1..=50 {
                send(
                    out,
                    Command::Spawn {
                        entity: entity(i),
                        kind: Kind::RoomNode,
                        asset: i,
                    },
                );
            }
        }
        for i in 1..=50u32 {
            rng = rng
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            if (rng >> 60) == 0 {
                let to = (rng >> 33) as f32 / (1u64 << 31) as f32 * 8.0 - 4.0;
                send(
                    out,
                    Command::Tween {
                        tween: i,
                        entity: entity(i),
                        property: sariema_format::script::Property::PositionX,
                        var: 0,
                        to,
                        duration: 0.7,
                        ease: Ease::InOutSine,
                    },
                );
            }
        }
    })
}

fn log() -> InputLog {
    serde_json::from_str(
        r#"{"changes":[
            {"step":10,"axes":{"move":[0,1]}},
            {"step":90,"axes":{"move":[0.7,0.2]},"down":["confirm"]},
            {"step":95,"up":["confirm"]},
            {"step":200,"axes":{"move":[0,0]}},
            {"step":260,"axes":{"move":[-1,-1]}},
            {"step":500,"axes":{"move":[0,0]}}
        ]}"#,
    )
    .unwrap()
}

/// The state hash after every display frame, keyed by step count.
fn hashes(frames: impl Iterator<Item = Duration>, steps: u64) -> BTreeMap<u64, u64> {
    let settings = Settings::default();
    let mut replay = Replay::new(&log(), &settings.input).unwrap();
    let mut sim = Simulation::new(settings, vec![], game(), 42);
    let mut out = BTreeMap::new();
    for dt in frames {
        sim.frame_with(dt, |step, a| a.clone_from(replay.actions(step)));
        out.insert(sim.steps(), sim.state_hash());
        if sim.steps() >= steps {
            break;
        }
    }
    out
}

#[test]
fn the_same_input_log_gives_the_same_hash_at_any_display_rate() {
    let steps = 600;
    let at = |hz: u64| {
        hashes(
            std::iter::repeat(Duration::from_nanos(1_000_000_000 / hz)),
            steps,
        )
    };
    let mut seed = 11u32;
    let irregular = hashes(
        std::iter::repeat_with(move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            Duration::from_micros(3_000 + (seed >> 16) as u64 % 40_000)
        }),
        steps,
    );
    let runs = [at(60), at(30), at(120), at(144), irregular];
    let mut compared = 0;
    for (step, hash) in &runs[0] {
        let others: Vec<Option<&u64>> = runs[1..].iter().map(|r| r.get(step)).collect();
        for other in others.iter().flatten() {
            assert_eq!(*other, hash, "step {step}");
            compared += 1;
        }
    }
    assert!(compared > 600, "only {compared} comparisons");
    for (i, run) in runs.iter().enumerate().skip(1) {
        let late = run
            .keys()
            .filter(|s| runs[0].contains_key(s))
            .max()
            .unwrap();
        assert!(*late >= 590, "run {i} shares no late step: {late}");
    }
}

#[test]
fn a_long_stall_runs_the_step_cap_and_drops_the_rest() {
    let mut sim = Simulation::new(Settings::default(), vec![], game(), 1);
    let tick = sim.frame_with(Duration::from_secs(3), |_, _| {});
    assert_eq!((tick.steps, tick.dropped), (5, 175));
    assert_eq!(sim.steps(), 5);
}

fn rejected(events: &[Event<'_>]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Rejected { code, .. } => Some(code.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn stale_handles_and_jumps_of_visible_actors_are_rejected_at_run_time() {
    let mut codes = Vec::new();
    let logic = native(|step, events, out| {
        codes.extend(rejected(events).into_iter().map(|c| (step, c)));
        let (a, b) = (entity(0), entity(1));
        match step {
            0 => {
                send(
                    out,
                    Command::Spawn {
                        entity: a,
                        kind: Kind::Actor,
                        asset: 0,
                    },
                );
                send(
                    out,
                    Command::Spawn {
                        entity: b,
                        kind: Kind::Empty,
                        asset: 0,
                    },
                );
                send(out, Command::Despawn { entity: b });
                send(
                    out,
                    Command::SetVisible {
                        entity: b,
                        visible: false,
                    },
                );
            }
            1 => send(
                out,
                Command::Teleport {
                    entity: a,
                    position: [5.0, 0.0, 0.0],
                    yaw: 0.0,
                    force: false,
                },
            ),
            2 => {
                send(
                    out,
                    Command::RigFixed {
                        rig: 1,
                        position: [0.0, 5.0, 5.0],
                        look_at: [0.0; 3],
                        fov: 40.0,
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
                send(
                    out,
                    Command::Teleport {
                        entity: a,
                        position: [6.0, 0.0, 0.0],
                        yaw: 0.0,
                        force: false,
                    },
                );
            }
            3 => send(
                out,
                Command::Fade {
                    to: 1.0,
                    duration: 0.0,
                    color: [0.0; 3],
                },
            ),
            4 => send(
                out,
                Command::SetTransform {
                    entity: a,
                    position: [7.0, 0.0, 0.0],
                    yaw: 0.0,
                },
            ),
            5 => send(
                out,
                Command::Spawn {
                    entity: Entity {
                        index: 1,
                        generation: 0,
                    },
                    kind: Kind::Empty,
                    asset: 0,
                },
            ),
            _ => {}
        }
    });
    let settings = Settings::default();
    let idle = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    let mut xs = Vec::new();
    for _ in 0..8 {
        sim.step(&idle);
        xs.push(sim.state().world.node(entity(0)).unwrap().local.position.x);
    }
    drop(sim);
    assert_eq!(xs[1], 0.0, "the visible teleport did not happen");
    assert_eq!(xs[2], 6.0, "a teleport in the step of a cut is allowed");
    assert_eq!(xs[4], 7.0, "a placement under a full fade is allowed");
    assert_eq!(
        codes,
        [
            (1, "E_ENTITY_STALE".to_string()),
            (2, "E_TELEPORT_VISIBLE".into()),
            (6, "E_ENTITY_IN_USE".into())
        ]
    );
}

#[test]
fn handing_the_hero_over_clears_buffered_presses() {
    let mut buffered = Vec::new();
    let mut controls = Vec::new();
    let logic = native(|step, events, out| {
        for e in events {
            match e {
                Event::Buffered { .. } => buffered.push(step),
                Event::Control { by } => controls.push(*by),
                _ => {}
            }
        }
        if step == 3 {
            send(
                out,
                Command::Control {
                    by: Controller::Script,
                },
            );
        }
    });
    let settings = Settings::default();
    let mut press = Actions::for_settings(&settings.input);
    press.buttons[0] = true;
    let idle = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    sim.step(&idle);
    sim.step(&press);
    for _ in 0..8 {
        sim.step(&idle);
    }
    drop(sim);
    assert_eq!(
        buffered,
        [1, 2, 3],
        "buffered until the hand-over at step 3"
    );
    assert_eq!(controls, [Controller::Script]);
}

struct Broken;

impl Logic for Broken {
    fn tick(&mut self, _: &[u8], _: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        commands.clear();
        commands.extend_from_slice(&[0x01, 0x00, 0x05]);
        Ok(())
    }
}

#[test]
fn a_corrupt_batch_stops_the_game_with_a_stable_code_and_the_engine_keeps_stepping() {
    let settings = Settings::default();
    let idle = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], Broken, 0);
    for _ in 0..3 {
        sim.step(&idle);
    }
    assert_eq!(sim.stopped().unwrap().code, "E_BATCH_TRUNCATED");
    assert_eq!(sim.steps(), 3);
    let host = sim.take_host();
    assert!(
        matches!(
            host.as_slice(),
            [HostOut::Stopped {
                code: "E_BATCH_TRUNCATED",
                ..
            }]
        ),
        "{host:?}"
    );
}

#[test]
fn saves_and_host_messages_cross_as_commands_and_events() {
    let mut got = Vec::new();
    let logic = native(|step, events, out| {
        for e in events {
            match e {
                Event::SaveLoaded { key, value } => {
                    got.push(format!("{key}={}", String::from_utf8_lossy(value)))
                }
                Event::SaveLoadDone {} => got.push("done".into()),
                Event::OrientationChanged { orientation } => got.push(format!("{orientation:?}")),
                _ => {}
            }
        }
        if step == 0 {
            send(out, Command::HostReady {});
            send(out, Command::SaveLoad {});
            send(
                out,
                Command::Save {
                    key: "best",
                    value: b"12",
                },
            );
        }
    });
    let settings = Settings::default();
    let idle = Actions::for_settings(&settings.input);
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    sim.step(&idle);
    assert_eq!(
        sim.take_host(),
        [
            HostOut::Ready,
            HostOut::LoadSaves,
            HostOut::Save {
                key: "best".into(),
                value: b"12".to_vec()
            }
        ]
    );
    sim.deliver(HostIn::Saves(vec![("best".into(), b"9".to_vec())]));
    sim.deliver(HostIn::Orientation(
        sariema_format::script::Orientation::Landscape,
    ));
    sim.step(&idle);
    drop(sim);
    assert_eq!(got, ["best=9", "done", "Landscape"]);
}

#[test]
fn a_zone_reports_an_actor_once_on_entry_and_once_on_exit() {
    let mut crossings = Vec::new();
    let logic = native(|step, events, out| {
        for e in events {
            match e {
                Event::TriggerEnter { trigger, .. } => crossings.push((step, "enter", *trigger)),
                Event::TriggerExit { trigger, .. } => crossings.push((step, "exit", *trigger)),
                _ => {}
            }
        }
        if step == 0 {
            send(
                out,
                Command::Spawn {
                    entity: entity(0),
                    kind: Kind::Actor,
                    asset: 0,
                },
            );
            send(
                out,
                Command::SetHero {
                    entity: entity(0),
                    speed: 3.0,
                },
            );
            send(
                out,
                Command::Zone {
                    zone: name_id("door"),
                    center: [0.0, 0.0, -3.0],
                    radius: 0.5,
                },
            );
        }
    });
    let settings = Settings {
        motion: sariema_runtime::motion::MotionSettings {
            camera_relative: false,
            ..Default::default()
        },
        ..Settings::default()
    };
    let mut sim = Simulation::new(settings.clone(), vec![], logic, 0);
    let mut north = Actions::for_settings(&settings.input);
    north.axes[0] = Vec2::new(0.0, 1.0);
    for _ in 0..120 {
        sim.step(&north);
    }
    drop(sim);
    let door = name_id("door");
    assert_eq!(
        crossings.iter().map(|c| (c.1, c.2)).collect::<Vec<_>>(),
        [("enter", door), ("exit", door)]
    );
}

#[test]
fn a_huge_time_scale_is_bounded_and_never_panics_the_host() {
    let logic = native(|step, _, out| {
        if step == 0 {
            send(
                out,
                Command::TimeScale {
                    scale: 1e30,
                    blend: 0.0,
                },
            );
        }
    });
    let settings = Settings::default();
    let max = settings.max_time_scale;
    let mut sim = Simulation::new(settings, vec![], logic, 0);
    for _ in 0..3 {
        sim.frame_with(Duration::from_millis(17), |_, _| {});
    }
    assert_eq!(sim.state().time_scale(), max);
    let tick = sim.frame_with(Duration::MAX, |_, _| {});
    assert_eq!(tick.steps, 5, "capped by max_steps_per_frame");
}
