// SPDX-License-Identifier: Apache-2.0

//! Timers counted in whole steps, so a timer of d seconds fires on step
//! round(d · 60) after it was set, exactly.

use crate::tween::steps_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timer {
    id: u32,
    period: u32,
    left: u32,
    repeat: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Timers {
    list: Vec<Timer>,
}

impl Timers {
    /// Setting a timer that exists restarts it.
    pub fn set(&mut self, id: u32, duration: f32, repeat: bool) {
        self.cancel(id);
        let period = steps_of(duration);
        self.list.push(Timer {
            id,
            period,
            left: period,
            repeat,
        });
    }

    pub fn cancel(&mut self, id: u32) {
        self.list.retain(|t| t.id != id);
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn step(&mut self, mut fired: impl FnMut(u32)) {
        self.list.retain_mut(|t| {
            t.left -= 1;
            if t.left > 0 {
                return true;
            }
            fired(t.id);
            t.left = t.period;
            t.repeat
        });
    }
}
