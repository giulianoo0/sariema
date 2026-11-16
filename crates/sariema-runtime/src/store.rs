// SPDX-License-Identifier: Apache-2.0

//! Flags and numeric variables the game, cutscenes and other modules share.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    pub flags: BTreeSet<u32>,
    /// A variable never set reads 0.
    pub vars: BTreeMap<u32, f32>,
}

impl Store {
    pub fn flag(&self, flag: u32) -> bool {
        self.flags.contains(&flag)
    }

    /// Whether the flag changed.
    pub fn set_flag(&mut self, flag: u32, on: bool) -> bool {
        if on {
            self.flags.insert(flag)
        } else {
            self.flags.remove(&flag)
        }
    }

    pub fn var(&self, var: u32) -> f32 {
        self.vars.get(&var).copied().unwrap_or(0.0)
    }
}
