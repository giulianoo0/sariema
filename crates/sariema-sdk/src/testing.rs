// SPDX-License-Identifier: Apache-2.0

//! Runs a game's logic natively, without the engine, for unit tests:
//! queue events on a [`TestHost`], [`crate::Runner::run`] a tick, read the
//! commands back.

use alloc::string::String;
use alloc::vec::Vec;

use crate::host::Host;
use crate::table::{Command, Event};
use crate::wire::Reader;

#[derive(Default)]
pub struct TestHost {
    events: Vec<u8>,
    ends: Vec<usize>,
    cursor: usize,
    /// Commands submitted, in order, until taken.
    pub commands: Vec<u8>,
    pub logs: Vec<(u32, String)>,
    /// Registered regions: kind, address and length.
    pub regions: Vec<(u32, usize, usize)>,
}

impl TestHost {
    pub fn new() -> TestHost {
        TestHost::default()
    }

    /// Queues an event for the next tick.
    pub fn event(&mut self, event: Event<'_>) {
        event.write(&mut self.events);
        self.ends.push(self.events.len());
    }

    /// The commands submitted since the last call.
    pub fn take_commands(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.commands)
    }
}

impl Host for TestHost {
    fn submit(&mut self, bytes: &[u8]) {
        self.commands.extend_from_slice(bytes);
    }

    fn events(&mut self, dst: &mut [u8]) -> i32 {
        let start = self.cursor.checked_sub(1).map_or(0, |i| self.ends[i]);
        let mut next = self.cursor;
        while next < self.ends.len() && self.ends[next] - start <= dst.len() {
            next += 1;
        }
        if next == self.cursor {
            if next < self.ends.len() {
                return -((self.ends[next] - start) as i32);
            }
            self.events.clear();
            self.ends.clear();
            self.cursor = 0;
            return 0;
        }
        let end = self.ends[next - 1];
        dst[..end - start].copy_from_slice(&self.events[start..end]);
        self.cursor = next;
        (end - start) as i32
    }

    fn register(&mut self, kind: u32, bytes: &[u8]) -> i32 {
        self.regions.push((kind, bytes.as_ptr() as usize, bytes.len()));
        self.regions.len() as i32 - 1
    }

    fn log(&mut self, level: u32, text: &str) {
        self.logs.push((level, text.into()));
    }
}

/// Decodes a run of commands.
///
/// # Panics
///
/// On bytes that are not a valid batch.
pub fn commands(bytes: &[u8]) -> impl Iterator<Item = Command<'_>> {
    let mut reader = Reader::new(bytes);
    core::iter::from_fn(move || {
        (!reader.is_empty()).then(|| Command::read(&mut reader).expect("a valid command batch"))
    })
}
