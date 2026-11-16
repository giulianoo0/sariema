// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use sariema_format::script::{Command, Entity, Event, records};
use sariema_runtime::sim::{Logic, Stop};

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
