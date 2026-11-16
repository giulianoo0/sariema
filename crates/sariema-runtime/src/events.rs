// SPDX-License-Identifier: Apache-2.0

//! The batch of events the game reads at its next tick, and the plain
//! happenings the sequencer waits on.

use sariema_format::script::{Entity, Event};

#[derive(Clone, Debug, Default)]
pub struct EventQueue {
    bytes: Vec<u8>,
    ends: Vec<u32>,
}

impl EventQueue {
    pub fn push(&mut self, event: Event<'_>) {
        event.write(&mut self.bytes);
        self.ends.push(self.bytes.len() as u32);
    }

    pub fn append(&mut self, other: &EventQueue) {
        let base = self.bytes.len() as u32;
        self.bytes.extend_from_slice(&other.bytes);
        self.ends.extend(other.ends.iter().map(|e| base + e));
    }

    pub fn clear(&mut self) {
        self.bytes.clear();
        self.ends.clear();
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// End offset of each record.
    pub fn ends(&self) -> &[u32] {
        &self.ends
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Happening {
    Arrived(Entity),
    Blocked(Entity),
    ClipFinished(Entity),
    Signal(u32),
    LineDone(u32),
}
