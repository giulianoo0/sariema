// SPDX-License-Identifier: Apache-2.0

use alloc::vec;
use alloc::vec::Vec;

use crate::wire::Entity;

/// A bulk region of kind 1: entity placements the engine applies after
/// every tick, in place of one `set_transform` command each. Its buffer is
/// sized once and never moves, so it can be registered once. Actors are
/// ignored here; they move by commands.
pub struct Transforms {
    bytes: Vec<u8>,
    capacity: usize,
}

const RECORD: usize = 24;

impl Transforms {
    pub const KIND: u32 = 1;

    pub fn with_capacity(capacity: usize) -> Transforms {
        Transforms {
            bytes: vec![0; 4 + RECORD * capacity],
            capacity,
        }
    }

    pub fn len(&self) -> usize {
        u32::from_le_bytes([self.bytes[0], self.bytes[1], self.bytes[2], self.bytes[3]]) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        self.bytes[..4].copy_from_slice(&0u32.to_le_bytes());
    }

    /// Adds a placement for this tick.
    ///
    /// # Panics
    ///
    /// Past the capacity, which would move the buffer.
    pub fn push(&mut self, entity: Entity, position: [f32; 3], yaw: f32) {
        let n = self.len();
        assert!(n < self.capacity, "the transforms region is full");
        let r = &mut self.bytes[4 + n * RECORD..4 + (n + 1) * RECORD];
        r[0..4].copy_from_slice(&entity.index.to_le_bytes());
        r[4..8].copy_from_slice(&entity.generation.to_le_bytes());
        for (i, v) in position.into_iter().chain([yaw]).enumerate() {
            r[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        self.bytes[..4].copy_from_slice(&(n as u32 + 1).to_le_bytes());
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}
