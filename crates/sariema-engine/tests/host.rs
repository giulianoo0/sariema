// SPDX-License-Identifier: Apache-2.0

//! The game's messages to the host wait in a bounded queue until a host
//! takes them, and every `save_load` is answered: by the host's save
//! handler, at once as no save when it has none, or as no save when its
//! answer takes longer than `host.save_load_timeout_s`.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use sariema_engine::{Engine, EngineSettings};
use sariema_format::script::{Command, Event};
use sariema_runtime::input::Actions;
use sariema_runtime::outbox::HostOut;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};

use common::{native, send};

fn bare<L: Logic>(logic: L, settings: EngineSettings) -> Engine<L> {
    let mut assets = common::assets();
    assets.rooms[0].collision = None;
    let sim = Simulation::new(Settings::default(), Vec::new(), logic, 1);
    Engine::new(sim, assets, settings).unwrap()
}

fn steps<L: Logic>(engine: &mut Engine<L>, n: u64) {
    let actions = Actions::for_settings(&engine.state().settings.input);
    for _ in 0..n {
        engine.step(&actions);
    }
}

type Seen = Rc<RefCell<Vec<(u64, String)>>>;

/// Logic that asks for its saves at step 1 and logs what comes back.
fn loads_at_one() -> (impl Logic, Seen) {
    let seen = Seen::default();
    let log = seen.clone();
    let logic = native(move |n, events, out| {
        for e in events {
            match e {
                Event::SaveLoaded { key, value } => log
                    .borrow_mut()
                    .push((n, format!("{key}={}", String::from_utf8_lossy(value)))),
                Event::SaveLoadDone {} => log.borrow_mut().push((n, "done".into())),
                _ => {}
            }
        }
        if n == 1 {
            send(out, Command::SaveLoad {});
        }
    });
    (logic, seen)
}

#[test]
fn the_host_queue_is_bounded_when_no_host_drains_it() {
    let payload = vec![7u8; 64 * 1024];
    let refused = Rc::new(RefCell::new(Vec::new()));
    let log = refused.clone();
    let logic = native(move |_, events, out| {
        for e in events {
            if let Event::Rejected { opcode, code, .. } = e {
                log.borrow_mut().push((*opcode, code.to_string()));
            }
        }
        send(
            out,
            Command::HostSend {
                topic: "t",
                payload: &payload,
            },
        );
    });
    let settings = EngineSettings::default();
    let limit = settings.host.max_bytes;
    let mut engine = bare(logic, settings);
    steps(&mut engine, 120);
    let kept: Vec<(u64, HostOut)> = engine.take_host().collect();
    let bytes: usize = kept
        .iter()
        .map(|(_, m)| match m {
            HostOut::Message { topic, payload } => topic.len() + payload.len(),
            _ => 0,
        })
        .sum();
    assert!(
        bytes <= limit,
        "{} messages, {bytes} bytes kept",
        kept.len()
    );
    assert_eq!(kept.len(), limit / (64 * 1024 + 1));
    assert_eq!(
        kept.iter().map(|(step, _)| *step).collect::<Vec<_>>(),
        (0..kept.len() as u64).collect::<Vec<_>>(),
        "the oldest are kept"
    );
    let opcode = Command::HostSend {
        topic: "",
        payload: &[],
    }
    .opcode();
    let refused = refused.take();
    assert_eq!(
        refused.len(),
        119 - kept.len(),
        "every later one is refused"
    );
    assert!(
        refused
            .iter()
            .all(|(o, code)| (*o, code.as_str()) == (opcode, "E_HOST_QUEUE_FULL"))
    );
}

#[test]
fn the_host_queue_holds_at_most_its_messages() {
    let logic = native(|_, _, out| send(out, Command::HostReady {}));
    let mut settings = EngineSettings::default();
    settings.host.max_messages = 4;
    let mut engine = bare(logic, settings);
    steps(&mut engine, 10);
    assert_eq!(engine.take_host().count(), 4);
    steps(&mut engine, 2);
    assert_eq!(
        engine.take_host().count(),
        2,
        "a host that drains makes room"
    );
}

#[test]
fn a_load_is_answered_as_no_save_at_once_without_a_save_handler() {
    let (logic, seen) = loads_at_one();
    let mut engine = bare(logic, EngineSettings::default());
    steps(&mut engine, 4);
    drop(engine);
    assert_eq!(seen.take(), [(2, "done".to_owned())]);
}

#[test]
fn a_save_handler_that_answers_steps_later_delivers_the_save() {
    let (logic, seen) = loads_at_one();
    let mut engine = bare(logic, EngineSettings::default());
    engine.answers_saves(true);
    steps(&mut engine, 2);
    let asked: Vec<HostOut> = engine.take_host().map(|(_, m)| m).collect();
    assert_eq!(asked, [HostOut::LoadSaves]);
    steps(&mut engine, 30);
    assert!(seen.borrow().is_empty(), "nothing before the answer");
    assert!(engine.answer_saves(vec![("best".into(), b"9".to_vec())]));
    assert!(!engine.answer_saves(Vec::new()), "one answer per load");
    steps(&mut engine, 2);
    drop(engine);
    assert_eq!(
        seen.take(),
        [(32, "best=9".to_owned()), (32, "done".to_owned())]
    );
}

#[test]
fn a_save_handler_that_never_answers_gives_no_save_at_the_timeout() {
    let (logic, seen) = loads_at_one();
    let settings = EngineSettings::default();
    let timeout = (settings.host.save_load_timeout_s * 60.0).round() as u64;
    let mut engine = bare(logic, settings);
    engine.answers_saves(true);
    steps(&mut engine, 2 + timeout);
    assert!(!engine.answer_saves(vec![("late".into(), vec![1])]));
    steps(&mut engine, 2);
    drop(engine);
    assert_eq!(seen.take(), [(timeout + 2, "done".to_owned())]);
}

#[test]
fn a_host_that_stops_answering_saves_answers_the_loads_it_left() {
    let (logic, seen) = loads_at_one();
    let mut engine = bare(logic, EngineSettings::default());
    engine.answers_saves(true);
    steps(&mut engine, 10);
    engine.answers_saves(false);
    steps(&mut engine, 2);
    drop(engine);
    assert_eq!(seen.take(), [(10, "done".to_owned())]);
}

/// Says it is ready every tick, and stops for good at the fourth.
struct ReadyThenStop(u64);

impl Logic for ReadyThenStop {
    fn tick(&mut self, _: &[u8], _: &[u32], out: &mut Vec<u8>) -> Result<(), Stop> {
        out.clear();
        self.0 += 1;
        if self.0 == 4 {
            return Err(Stop {
                code: "E_GAME_TRAP",
                message: "unreachable".into(),
            });
        }
        send(out, Command::HostReady {});
        Ok(())
    }
}

#[test]
fn the_games_stop_reaches_a_full_queue() {
    let mut settings = EngineSettings::default();
    settings.host.max_messages = 2;
    let mut engine = bare(ReadyThenStop(0), settings);
    steps(&mut engine, 6);
    let kept: Vec<HostOut> = engine.take_host().map(|(_, m)| m).collect();
    assert!(
        matches!(
            kept.as_slice(),
            [
                HostOut::Ready,
                HostOut::Ready,
                HostOut::Stopped {
                    code: "E_GAME_TRAP",
                    ..
                }
            ]
        ),
        "{kept:?}"
    );
}
