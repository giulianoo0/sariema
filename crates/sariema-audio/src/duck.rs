// SPDX-License-Identifier: Apache-2.0

//! Ducking: buses drop while another bus has a voice playing. The key is
//! the bus's activity, not its signal, so a pause inside a spoken line does
//! not let the music swell.

use crate::ramp::{Curve, Ramp, db_to_gain};
use crate::settings::{DuckRule, seconds_to_frames};

pub(crate) struct Duck {
    pub key: u16,
    pub targets: Vec<u16>,
    depth_db: f32,
    attack: u32,
    hold: u32,
    release: u32,
    /// In dB, between `depth_db` and 0.
    level: Ramp,
    active: bool,
    release_at: Option<u64>,
}

impl Duck {
    pub fn new(rule: &DuckRule, key: u16, targets: Vec<u16>, rate: u32) -> Duck {
        Duck {
            key,
            targets,
            depth_db: rule.depth_db.min(0.0),
            attack: seconds_to_frames(rule.attack_s, rate),
            hold: seconds_to_frames(rule.hold_s, rate),
            release: seconds_to_frames(rule.release_s, rate),
            level: Ramp::constant(0.0),
            active: false,
            release_at: None,
        }
    }

    /// Follows the key at frame `t`. A change of direction starts from the
    /// current level and keeps the configured slope, so an interrupted
    /// release does not jump.
    pub fn update(&mut self, t: u64, key_active: bool) {
        let now = self.level.value(t);
        let depth = self.depth_db;
        if key_active {
            self.release_at = None;
            if !self.active {
                self.active = true;
                let len = scaled(self.attack, (now - depth) / -depth);
                self.level.retarget(t, depth, len, Curve::Linear);
            }
            return;
        }
        if self.active {
            self.active = false;
            self.release_at = Some(t + u64::from(self.hold));
        }
        if self.release_at.is_some_and(|r| t >= r) {
            self.release_at = None;
            let len = scaled(self.release, now / depth);
            self.level.retarget(t, 0.0, len, Curve::Linear);
        }
    }

    /// Frame after `t` at which [`Duck::update`] must run again.
    pub fn next_event(&self, t: u64) -> Option<u64> {
        self.release_at.filter(|&r| r > t)
    }

    pub fn gain(&self, t: u64) -> f32 {
        db_to_gain(self.level.value(t))
    }

    pub fn db(&self, t: u64) -> f32 {
        self.level.value(t)
    }

    pub fn is_flat(&self, t: u64, n: u32) -> bool {
        self.level.is_flat(t, n)
    }
}

fn scaled(frames: u32, fraction: f32) -> u32 {
    if fraction.is_finite() {
        (frames as f32 * fraction.clamp(0.0, 1.0)).round() as u32
    } else {
        0
    }
}
