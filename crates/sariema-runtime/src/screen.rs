// SPDX-License-Identifier: Apache-2.0

//! What covers the view: the fade, the cinema bars and the dialogue line.

use glam::Vec3;
use sariema_format::script::{Advance, Entity};

use crate::tween::steps_of;

/// A value moving linearly to a target over whole steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fader {
    value: f32,
    from: f32,
    to: f32,
    steps: u32,
    elapsed: u32,
}

impl Fader {
    pub const ZERO: Fader = Fader::at(0.0);

    pub const fn at(value: f32) -> Fader {
        Fader {
            value,
            from: value,
            to: value,
            steps: 0,
            elapsed: 0,
        }
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn done(&self) -> bool {
        self.elapsed >= self.steps
    }

    /// A duration of 0 takes effect at once.
    pub fn set(&mut self, to: f32, duration: f32) {
        self.from = self.value;
        self.to = to;
        self.elapsed = 0;
        if duration <= 0.0 {
            self.value = to;
            self.steps = 0;
        } else {
            self.steps = steps_of(duration);
        }
    }

    pub fn step(&mut self) {
        if self.done() {
            return;
        }
        self.elapsed += 1;
        self.value = if self.done() {
            self.to
        } else {
            self.from + (self.to - self.from) * (self.elapsed as f32 / self.steps as f32)
        };
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    /// 0 clear, 1 fully covered.
    pub fade: Fader,
    /// Linear RGB.
    pub fade_color: Vec3,
    pub letterbox: Fader,
}

impl Default for Screen {
    fn default() -> Screen {
        Screen {
            fade: Fader::ZERO,
            fade_color: Vec3::ZERO,
            letterbox: Fader::ZERO,
        }
    }
}

impl Screen {
    pub fn covered(&self) -> bool {
        self.fade.value() >= 1.0
    }

    pub fn step(&mut self) {
        self.fade.step();
        self.letterbox.step();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub line: u32,
    pub speaker: Entity,
    pub text: String,
    pub voice: u32,
    pub advance: Advance,
    steps_left: u32,
}

/// The dialogue line on screen, for the UI to draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dialogue {
    pub line: Option<Line>,
}

impl Dialogue {
    /// Shows a line; returns the id of a line it replaced.
    pub fn say(
        &mut self,
        line: u32,
        speaker: Entity,
        text: &str,
        voice: u32,
        advance: Advance,
        duration: f32,
    ) -> Option<u32> {
        let replaced = self.line.take().map(|l| l.line);
        self.line = Some(Line {
            line,
            speaker,
            text: text.into(),
            voice,
            advance,
            steps_left: steps_of(duration),
        });
        replaced
    }

    /// Ends a tap line; returns its id.
    pub fn advance(&mut self) -> Option<u32> {
        match &self.line {
            Some(l) if l.advance == Advance::Tap => self.line.take().map(|l| l.line),
            _ => None,
        }
    }

    /// Counts an auto line down; returns its id when it ends.
    pub fn step(&mut self) -> Option<u32> {
        let l = self.line.as_mut()?;
        if l.advance != Advance::Auto {
            return None;
        }
        l.steps_left = l.steps_left.saturating_sub(1);
        if l.steps_left == 0 {
            return self.line.take().map(|l| l.line);
        }
        None
    }
}
