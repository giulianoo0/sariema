// SPDX-License-Identifier: Apache-2.0

//! The time of each actor's clip, counted in steps. The animation module
//! seeks every clip to this time instead of running it on its own clock, so
//! a clip slows with the world in slow motion and never drifts from the
//! steps that wait on it; drawn between steps, it never stands still.
//!
//! A clip's rate can change without starting it over, at once or at a time
//! of the clip, and the clip queued after it takes over at the exact time
//! it ends, with the time left over: no seam shows a still frame.

use sariema_core::STEP_SECONDS;
use sariema_format::script::{ClipMode, Entity};

use crate::outbox::ClipRequest;

/// A clip queued after the one an actor plays.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Next {
    pub clip: u32,
    pub mode: ClipMode,
    /// Crossfade seconds from the clip before.
    pub blend: f32,
    pub rate: f32,
    /// Seconds of the clip before after which this one begins; that clip's
    /// whole length when `None`, or at once when its length is unknown.
    pub after: Option<f32>,
    /// Seconds this clip lasts, when known.
    pub length: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
struct Playing {
    entity: Entity,
    clip: u32,
    /// Clip seconds per step of world time, over 1/60 s.
    rate: f32,
    /// Its time when `rate` began, `steps` steps ago.
    base: f32,
    steps: u32,
    /// Its time a step before `base`, drawn in the step `rate` began.
    before: f32,
    /// Steps it stays where it is before playing at `held_rate`.
    held: u32,
    held_rate: f32,
    length: Option<f32>,
    /// Rates due at times of this clip, in order; `due` of them taken.
    changes: Vec<(f32, f32)>,
    due: usize,
    queue: Vec<Next>,
}

impl Playing {
    fn now(&self) -> f32 {
        self.at(self.steps as f32)
    }

    /// Its time `steps` steps after `base`, as a constant rate gives it.
    fn at(&self, steps: f32) -> f32 {
        self.base + steps * STEP_SECONDS * self.rate
    }

    fn start(&mut self, clip: u32, rate: f32, from: f32, length: Option<f32>) {
        self.clip = clip;
        self.rate = rate;
        self.base = from;
        self.steps = 0;
        self.before = from;
        self.held = 0;
        self.held_rate = rate;
        self.length = length;
        self.changes.clear();
        self.due = 0;
    }

    /// From here on at `rate`, from the time it is at.
    fn rebase(&mut self, rate: f32) {
        let now = self.now();
        self.before = if self.steps == 0 {
            self.before
        } else {
            self.at(self.steps as f32 - 1.0)
        };
        self.base = now;
        self.steps = 0;
        self.rate = rate;
    }

    /// When the clip ends and the next queued one begins.
    fn end(&self) -> Option<f32> {
        let next = self.queue.first()?;
        next.after.or(self.length).or(Some(self.base))
    }

    fn step(&mut self, out: &mut Vec<ClipRequest>) {
        let before = self.now();
        self.steps += 1;
        if self.held > 0 {
            self.held -= 1;
            if self.held == 0 {
                self.rebase(self.held_rate);
            }
            return;
        }
        if self.rate <= 0.0 {
            return;
        }
        let mut t = self.now();
        let mut crossed = false;
        let mut switched = false;
        loop {
            let change = self.changes.get(self.due).map(|c| c.0);
            let end = self.end();
            match (change, end) {
                (Some(at), end) if at <= t && end.is_none_or(|e| at <= e) => {
                    let over = (t - at) / self.rate;
                    self.rate = self.changes[self.due].1;
                    self.due += 1;
                    t = at + over * self.rate;
                }
                (_, Some(at)) if at <= t => {
                    let over = (t - at) / self.rate;
                    let next = self.queue.remove(0);
                    self.start(next.clip, next.rate, 0.0, next.length);
                    t = over * next.rate;
                    switched = true;
                    out.push(ClipRequest {
                        entity: self.entity,
                        clip: next.clip,
                        mode: next.mode,
                        blend: next.blend,
                        speed: next.rate,
                        from: 0.0,
                    });
                }
                _ => break,
            }
            crossed = true;
            if self.rate <= 0.0 {
                break;
            }
        }
        if crossed {
            self.before = if switched { 0.0 } else { before };
            self.base = t;
            self.steps = 0;
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ClipClock {
    list: Vec<Playing>,
    /// Seconds each clip lasts, by actor asset then clip, in order.
    lengths: Vec<(u32, u32, f32)>,
}

impl ClipClock {
    /// `clip` of the actors drawn with `asset` lasts `seconds`.
    pub fn learn(&mut self, asset: u32, clip: u32, seconds: f32) {
        match self
            .lengths
            .binary_search_by_key(&(asset, clip), |l| (l.0, l.1))
        {
            Ok(k) => self.lengths[k].2 = seconds,
            Err(k) => self.lengths.insert(k, (asset, clip, seconds)),
        }
    }

    pub fn length_of(&self, asset: u32, clip: u32) -> Option<f32> {
        let k = (self.lengths)
            .binary_search_by_key(&(asset, clip), |l| (l.0, l.1))
            .ok()?;
        Some(self.lengths[k].2)
    }

    /// `clip` starts on `entity` at time 0, replacing what it played.
    pub fn play(&mut self, entity: Entity, clip: u32, speed: f32) {
        self.play_from(entity, clip, speed, 0.0);
    }

    /// `clip` starts on `entity` at `from` seconds into it, replacing what
    /// it played and what was queued after it.
    pub fn play_from(&mut self, entity: Entity, clip: u32, rate: f32, from: f32) {
        let p = match self.list.iter().position(|p| p.entity == entity) {
            Some(k) => &mut self.list[k],
            None => {
                self.list.push(Playing {
                    entity,
                    clip,
                    rate,
                    base: from,
                    steps: 0,
                    before: from,
                    held: 0,
                    held_rate: rate,
                    length: None,
                    changes: Vec::new(),
                    due: 0,
                    queue: Vec::new(),
                });
                return;
            }
        };
        p.start(clip, rate, from, None);
        p.queue.clear();
    }

    fn get(&mut self, entity: Entity) -> Option<&mut Playing> {
        self.list.iter_mut().find(|p| p.entity == entity)
    }

    /// How long the clip `entity` plays lasts, when the animation module
    /// knows it: a queued clip waits for it.
    pub fn set_length(&mut self, entity: Entity, length: Option<f32>) {
        if let Some(p) = self.get(entity) {
            p.length = length;
        }
    }

    /// From this step on `entity`'s `clip` plays at `rate`, from the time
    /// it is at. False when `entity` plays another clip or none.
    pub fn set_rate(&mut self, entity: Entity, clip: u32, rate: f32) -> bool {
        match self.get(entity) {
            Some(p) if p.clip == clip => {
                if p.held > 0 {
                    p.held_rate = rate;
                } else {
                    p.rebase(rate);
                }
                true
            }
            _ => false,
        }
    }

    /// When the clip `entity` plays reaches `at` seconds, it goes on at
    /// `rate`, within the step it gets there. Due times come in order.
    pub fn rate_at(&mut self, entity: Entity, at: f32, rate: f32) {
        if let Some(p) = self.get(entity) {
            p.changes.push((at, rate));
        }
    }

    /// The clip `entity` plays stays where it is for `steps` steps first.
    pub fn hold(&mut self, entity: Entity, steps: u32) {
        if let Some(p) = self.get(entity)
            && steps > 0
        {
            if p.held == 0 {
                p.held_rate = p.rate;
                p.rebase(0.0);
            }
            p.held = steps;
        }
    }

    /// Queues `next` after the clips `entity` plays and has queued.
    pub fn then(&mut self, entity: Entity, next: Next) {
        if let Some(p) = self.get(entity) {
            p.queue.push(next);
        }
    }

    /// Whether `entity` plays the last clip queued and it reached its end;
    /// a clip of unknown length counts as at its end.
    pub fn through(&self, entity: Entity) -> bool {
        self.list
            .iter()
            .find(|p| p.entity == entity)
            .is_none_or(|p| p.queue.is_empty() && p.length.is_none_or(|l| p.now() >= l))
    }

    /// Lands `entity`'s clip and its queue at their end, as if played.
    pub fn finish(&mut self, entity: Entity, out: &mut Vec<ClipRequest>) {
        let Some(p) = self.get(entity) else { return };
        if let Some(last) = p.queue.pop() {
            out.push(ClipRequest {
                entity,
                clip: last.clip,
                mode: last.mode,
                blend: 0.0,
                speed: last.rate,
                from: 0.0,
            });
            p.start(last.clip, last.rate, 0.0, last.length);
            p.queue.clear();
        }
        let end = p.length.unwrap_or(p.now()).max(p.now());
        p.changes.clear();
        p.due = 0;
        if p.held > 0 {
            p.rate = p.held_rate;
            p.held = 0;
        }
        p.base = end;
        p.before = end;
        p.steps = 0;
    }

    pub fn forget(&mut self, entity: Entity) {
        self.list.retain(|p| p.entity != entity);
    }

    /// One more step for every clip started before this one; a queued clip
    /// that takes over asks the animation module for it in `out`.
    pub fn step(&mut self, out: &mut Vec<ClipRequest>) {
        for p in &mut self.list {
            p.step(out);
        }
    }

    /// The clip `entity` plays and its time in seconds.
    pub fn time(&self, entity: Entity) -> Option<(u32, f32)> {
        self.list
            .iter()
            .find(|p| p.entity == entity)
            .map(|p| (p.clip, p.now()))
    }

    /// The time at `alpha` between the previous step and the last one, as
    /// poses are drawn; never before the clip began.
    pub fn time_at(&self, entity: Entity, alpha: f32) -> Option<(u32, f32)> {
        self.list.iter().find(|p| p.entity == entity).map(|p| {
            let t = if p.steps == 0 {
                p.before + (p.base - p.before) * alpha
            } else {
                p.at((p.steps as f32 - 1.0 + alpha).max(0.0))
            };
            (p.clip, t)
        })
    }
}
