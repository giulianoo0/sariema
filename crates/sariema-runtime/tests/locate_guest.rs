// SPDX-License-Identifier: Apache-2.0

//! `locate` from a WebAssembly guest gives the same `located` answers under
//! every interpreter, and replaying the command batches the guest sent
//! gives them again: the answers come from the step's state alone.

use sariema_format::script::{BATCH_VERSION, Command, Entity, Event, Kind, records};
use sariema_runtime::game::WasmLogic;
use sariema_runtime::input::Actions;
use sariema_runtime::settings::Settings;
use sariema_runtime::sim::{Logic, Simulation, Stop};
use sariema_vm::{Interpreter, Vm, Wasmi};

/// The step the guest asks at.
const ASKED: u32 = 25;
const STEPS: usize = 40;

fn entity(index: u32) -> Entity {
    Entity {
        index,
        generation: 0,
    }
}

/// The first batch: a walker going +x, a rider on a turned cart.
fn opening() -> Vec<u8> {
    let mut out = Vec::new();
    for (index, kind) in [(0, Kind::Actor), (1, Kind::Empty), (2, Kind::Actor)] {
        Command::Spawn {
            entity: entity(index),
            kind,
            asset: 0,
        }
        .write(&mut out);
    }
    Command::SetTransform {
        entity: entity(1),
        position: [4.0, 0.5, -2.0],
        yaw: 250.0,
    }
    .write(&mut out);
    Command::SetParent {
        entity: entity(2),
        parent: entity(1),
    }
    .write(&mut out);
    Command::SetTransform {
        entity: entity(2),
        position: [0.0, 0.0, -1.0],
        yaw: 30.0,
    }
    .write(&mut out);
    Command::MoveTo {
        entity: entity(0),
        target: [9.0, 0.0, 3.0],
        speed: 1.3,
    }
    .write(&mut out);
    out
}

/// The batch at `ASKED`: where each is, and a handle never spawned.
fn asking() -> Vec<u8> {
    let mut out = Vec::new();
    for index in [0, 2, 7] {
        Command::Locate {
            entity: entity(index),
        }
        .write(&mut out);
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

/// A guest that submits `opening` at its first tick and `asking` at tick
/// `ASKED`, and nothing else.
fn guest() -> Vec<u8> {
    let (open, ask) = (opening(), asking());
    wat::parse_str(format!(
        r#"(module
             (import "sariema" "submit" (func $submit (param i32 i32)))
             (memory (export "memory") 1)
             (data (i32.const 1024) "{}")
             (data (i32.const 8192) "{}")
             (global $n (mut i32) (i32.const 0))
             (func (export "sariema_tick")
               (if (i32.eqz (global.get $n))
                 (then (call $submit (i32.const 1024) (i32.const {}))))
               (if (i32.eq (global.get $n) (i32.const {ASKED}))
                 (then (call $submit (i32.const 8192) (i32.const {}))))
               (global.set $n (i32.add (global.get $n) (i32.const 1))))
             (func (export "sariema_batch_version") (result i32) (i32.const {BATCH_VERSION})))"#,
        hex(&open),
        hex(&ask),
        open.len(),
        ask.len(),
    ))
    .unwrap()
}

/// What a game heard back: `located` as (tick, entity, position, yaw) and
/// `rejected` as (tick, code, entity).
#[derive(Debug, Default, PartialEq)]
struct Heard {
    located: Vec<(u64, u32, [f32; 3], f32)>,
    rejected: Vec<(u64, String, u32)>,
}

/// Runs `inner`, writing down what it heard and the batches it sent.
struct Tee<L> {
    inner: L,
    tick: u64,
    heard: Heard,
    sent: Vec<Vec<u8>>,
}

impl<L: Logic> Logic for Tee<L> {
    fn tick(&mut self, events: &[u8], ends: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        for event in records::<Event>(events) {
            match event.unwrap() {
                Event::Located {
                    entity,
                    position,
                    yaw,
                    held: false,
                } => self
                    .heard
                    .located
                    .push((self.tick, entity.index, position, yaw)),
                Event::Rejected { code, entity, .. } => {
                    self.heard
                        .rejected
                        .push((self.tick, code.to_string(), entity.index))
                }
                _ => {}
            }
        }
        self.inner.tick(events, ends, commands)?;
        self.sent.push(commands.clone());
        self.tick += 1;
        Ok(())
    }
}

/// Sends back the batches a run sent, one per tick.
struct Replay(std::vec::IntoIter<Vec<u8>>);

impl Logic for Replay {
    fn tick(&mut self, _: &[u8], _: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        commands.clear();
        commands.extend(self.0.next().unwrap_or_default());
        Ok(())
    }
}

fn run<L: Logic>(inner: L) -> (Heard, Vec<Vec<u8>>) {
    let tee = Tee {
        inner,
        tick: 0,
        heard: Heard::default(),
        sent: Vec::new(),
    };
    let mut sim = Simulation::new(Settings::default(), Vec::new(), tee, 0);
    let actions = Actions::for_settings(&sim.state().settings.input);
    for _ in 0..STEPS {
        sim.step(&actions);
    }
    let tee = sim.logic_mut();
    (
        std::mem::take(&mut tee.heard),
        std::mem::take(&mut tee.sent),
    )
}

fn in_the_machine<I: Interpreter>() -> (Heard, Vec<Vec<u8>>) {
    let vm = Vm::load_with::<I>(&guest(), Settings::default().vm).unwrap();
    run(WasmLogic::new(vm))
}

fn same_as_the_replay(heard: &Heard, sent: Vec<Vec<u8>>) {
    let (replayed, _) = run(Replay(sent.into_iter()));
    assert_eq!(&replayed, heard);
}

#[test]
fn a_guest_hears_where_its_actors_are_and_a_replay_hears_the_same() {
    let (heard, sent) = in_the_machine::<Wasmi>();
    let asked = ASKED as u64 + 1;
    assert_eq!(heard.rejected, [(asked, "E_ENTITY_STALE".to_string(), 7)]);
    let who: Vec<_> = heard.located.iter().map(|l| (l.0, l.1)).collect();
    assert_eq!(who, [(asked, 0), (asked, 2)]);
    for &(_, _, _, yaw) in &heard.located {
        assert!(yaw > -180.0 && yaw <= 180.0, "{yaw}");
    }
    same_as_the_replay(&heard, sent);
    assert_eq!(in_the_machine::<Wasmi>().0, heard);
}

#[cfg(feature = "tinywasm")]
#[test]
fn every_interpreter_hears_the_same() {
    let (heard, sent) = in_the_machine::<sariema_vm::Tinywasm>();
    assert_eq!(heard, in_the_machine::<Wasmi>().0);
    same_as_the_replay(&heard, sent);
}
