// SPDX-License-Identifier: Apache-2.0

//! Abstract input: move axes and named buttons fed by keyboard, gamepad or
//! the UI's touch joystick, sampled once per step. The per-step action state
//! is what is recorded and replayed, so replays do not depend on bindings.

use std::collections::BTreeMap;

use glam::Vec2;
use sariema_format::script::Event;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Up,
    Down,
    Left,
    Right,
    Space,
    Enter,
    Escape,
    Tab,
    Shift,
    Control,
    Alt,
    Backspace,
}

/// Buttons by position on a standard gamepad (south is A on Xbox, Cross on PlayStation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadButton {
    South,
    East,
    West,
    North,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    LeftStick,
    RightStick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stick {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisBinding {
    pub name: String,
    #[serde(default)]
    pub up: Vec<Key>,
    #[serde(default)]
    pub down: Vec<Key>,
    #[serde(default)]
    pub left: Vec<Key>,
    #[serde(default)]
    pub right: Vec<Key>,
    #[serde(default)]
    pub stick: Option<Stick>,
    /// The touch joystick drives this axis.
    #[serde(default)]
    pub touch: bool,
    /// Stick deflection below this reads 0; above, it is rescaled to start at 0.
    #[serde(default = "AxisBinding::default_dead_zone")]
    pub dead_zone: f32,
}

impl AxisBinding {
    fn default_dead_zone() -> f32 {
        0.2
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ButtonBinding {
    pub name: String,
    #[serde(default)]
    pub keys: Vec<Key>,
    #[serde(default)]
    pub pad: Vec<PadButton>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct InputSettings {
    pub axes: Vec<AxisBinding>,
    pub buttons: Vec<ButtonBinding>,
    /// Steps a press stays buffered waiting to be consumed.
    pub buffer_steps: u16,
}

impl Default for InputSettings {
    fn default() -> InputSettings {
        use Key::*;
        let button = |name: &str, keys: Vec<Key>, pad: Vec<PadButton>| ButtonBinding {
            name: name.into(),
            keys,
            pad,
        };
        InputSettings {
            axes: vec![AxisBinding {
                name: "move".into(),
                up: vec![W, Up],
                down: vec![S, Down],
                left: vec![A, Left],
                right: vec![D, Right],
                stick: Some(Stick::Left),
                touch: true,
                dead_zone: AxisBinding::default_dead_zone(),
            }],
            buttons: vec![
                button("confirm", vec![Space, Enter, E], vec![PadButton::South]),
                button("back", vec![Backspace], vec![PadButton::East]),
                button("skip", vec![Escape], vec![PadButton::Start]),
            ],
            buffer_steps: 6,
        }
    }
}

impl InputSettings {
    pub fn axis(&self, name: &str) -> Option<u16> {
        self.axes
            .iter()
            .position(|a| a.name == name)
            .map(|i| i as u16)
    }

    pub fn button(&self, name: &str) -> Option<u16> {
        self.buttons
            .iter()
            .position(|b| b.name == name)
            .map(|i| i as u16)
    }
}

/// The devices' state, as the host reports it between frames.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawInput {
    keys: u64,
    pad: u32,
    pub sticks: [Vec2; 2],
    /// The touch joystick, already in [−1, 1]², while a finger holds it.
    pub touch: Option<Vec2>,
}

impl RawInput {
    pub fn set_key(&mut self, key: Key, down: bool) {
        let bit = 1u64 << key as u32;
        self.keys = if down {
            self.keys | bit
        } else {
            self.keys & !bit
        };
    }

    pub fn set_pad(&mut self, button: PadButton, down: bool) {
        let bit = 1u32 << button as u32;
        self.pad = if down {
            self.pad | bit
        } else {
            self.pad & !bit
        };
    }

    fn key(&self, key: Key) -> bool {
        self.keys & (1u64 << key as u32) != 0
    }

    fn any(&self, keys: &[Key]) -> bool {
        keys.iter().any(|&k| self.key(k))
    }
}

/// The action state of one step.
#[derive(Debug, Default, PartialEq)]
pub struct Actions {
    pub axes: Vec<Vec2>,
    pub buttons: Vec<bool>,
}

// By hand so `clone_from` reuses the vectors: input copies actions every step.
impl Clone for Actions {
    fn clone(&self) -> Actions {
        Actions {
            axes: self.axes.clone(),
            buttons: self.buttons.clone(),
        }
    }

    fn clone_from(&mut self, source: &Actions) {
        self.axes.clone_from(&source.axes);
        self.buttons.clone_from(&source.buttons);
    }
}

impl Actions {
    pub fn for_settings(settings: &InputSettings) -> Actions {
        Actions {
            axes: vec![Vec2::ZERO; settings.axes.len()],
            buttons: vec![false; settings.buttons.len()],
        }
    }
}

fn dead_zone(v: Vec2, zone: f32) -> Vec2 {
    let len = v.length();
    if len <= zone || zone >= 1.0 {
        return Vec2::ZERO;
    }
    v / len * ((len.min(1.0) - zone) / (1.0 - zone))
}

pub struct Input {
    settings: InputSettings,
    current: Actions,
    previous: Actions,
    /// Age in steps of a pending press of each button.
    buffer: Vec<Option<u16>>,
}

impl Input {
    pub fn new(settings: InputSettings) -> Input {
        let current = Actions::for_settings(&settings);
        Input {
            buffer: vec![None; settings.buttons.len()],
            previous: current.clone(),
            current,
            settings,
        }
    }

    pub fn settings(&self) -> &InputSettings {
        &self.settings
    }

    /// Maps device state to actions. Keys make a unit-length direction;
    /// the touch joystick, then a stick, override them when deflected.
    pub fn map(&self, raw: &RawInput, out: &mut Actions) {
        for (axis, binding) in out.axes.iter_mut().zip(&self.settings.axes) {
            let keys = Vec2::new(
                raw.any(&binding.right) as i32 as f32 - raw.any(&binding.left) as i32 as f32,
                raw.any(&binding.up) as i32 as f32 - raw.any(&binding.down) as i32 as f32,
            )
            .normalize_or_zero();
            let stick = binding
                .stick
                .map(|s| dead_zone(raw.sticks[s as usize], binding.dead_zone))
                .unwrap_or(Vec2::ZERO);
            let touch = raw
                .touch
                .filter(|_| binding.touch)
                .map(|t| dead_zone(t, binding.dead_zone))
                .unwrap_or(Vec2::ZERO);
            *axis = [touch, stick, keys]
                .into_iter()
                .find(|v| *v != Vec2::ZERO)
                .unwrap_or(Vec2::ZERO);
        }
        for (down, binding) in out.buttons.iter_mut().zip(&self.settings.buttons) {
            *down = raw.any(&binding.keys)
                || binding.pad.iter().any(|&b| raw.pad & (1 << b as u32) != 0);
        }
    }

    /// Takes this step's actions and reports what changed: axes that moved,
    /// button edges, then every press still buffered. Actions past the
    /// settings' are ignored; missing ones count as released.
    pub fn step<'a>(&mut self, actions: &Actions, mut emit: impl FnMut(Event<'a>)) {
        std::mem::swap(&mut self.previous, &mut self.current);
        self.current.clone_from(actions);
        self.current
            .axes
            .resize(self.settings.axes.len(), Vec2::ZERO);
        self.current.buttons.resize(self.buffer.len(), false);
        for (i, (now, before)) in self
            .current
            .axes
            .iter()
            .zip(&self.previous.axes)
            .enumerate()
        {
            if now != before {
                emit(Event::Axis {
                    action: i as u16,
                    x: now.x,
                    y: now.y,
                });
            }
        }
        for (i, buffered) in self.buffer.iter_mut().enumerate() {
            let (now, before) = (self.current.buttons[i], self.previous.buttons[i]);
            if now != before {
                emit(Event::Button {
                    action: i as u16,
                    down: now,
                });
            }
            let window = self.settings.buffer_steps;
            *buffered = match (*buffered, now && !before) {
                (_, true) if window > 0 => Some(0),
                (Some(age), false) if age + 1 < window => Some(age + 1),
                _ => None,
            };
        }
        for (i, buffered) in self.buffer.iter().enumerate() {
            if let Some(age) = buffered {
                emit(Event::Buffered {
                    action: i as u16,
                    age: *age,
                });
            }
        }
    }

    /// Pressed this step.
    pub fn pressed(&self, button: u16) -> bool {
        let i = button as usize;
        self.current.buttons.get(i) == Some(&true) && self.previous.buttons.get(i) == Some(&false)
    }

    pub fn buffered(&self, button: u16) -> bool {
        self.buffer
            .get(button as usize)
            .is_some_and(|b| b.is_some())
    }

    /// Takes a buffered press; whether there was one.
    pub fn consume(&mut self, button: u16) -> bool {
        self.buffer
            .get_mut(button as usize)
            .and_then(Option::take)
            .is_some()
    }

    pub fn clear_buffer(&mut self) {
        self.buffer.iter_mut().for_each(|b| *b = None);
    }

    pub fn axis(&self, axis: u16) -> Vec2 {
        self.current
            .axes
            .get(axis as usize)
            .copied()
            .unwrap_or(Vec2::ZERO)
    }

    pub fn current(&self) -> &Actions {
        &self.current
    }
}

/// A recorded input: the steps at which actions changed. An action keeps
/// its value until a later change names it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputLog {
    pub changes: Vec<Change>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub step: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub axes: BTreeMap<String, [f32; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub down: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub up: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogError {
    UnknownAction(String),
    OutOfOrder { step: u64 },
}

impl LogError {
    pub fn code(&self) -> &'static str {
        "E_INPUT_LOG"
    }
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogError::UnknownAction(name) => write!(
                f,
                "the input log names action {name:?}, which the game does not define"
            ),
            LogError::OutOfOrder { step } => write!(
                f,
                "the input log's change at step {step} is not after the one before"
            ),
        }
    }
}

enum Set {
    Axis(usize, Vec2),
    Button(usize, bool),
}

/// Plays an [`InputLog`] back one step at a time.
pub struct Replay {
    changes: Vec<(u64, Vec<Set>)>,
    next: usize,
    actions: Actions,
}

impl Replay {
    pub fn new(log: &InputLog, settings: &InputSettings) -> Result<Replay, LogError> {
        let mut changes = Vec::with_capacity(log.changes.len());
        let mut last = None;
        for c in &log.changes {
            if last.is_some_and(|l| c.step <= l) {
                return Err(LogError::OutOfOrder { step: c.step });
            }
            last = Some(c.step);
            let unknown = |n: &String| LogError::UnknownAction(n.clone());
            let mut sets = Vec::new();
            for (name, [x, y]) in &c.axes {
                let i = settings.axis(name).ok_or_else(|| unknown(name))?;
                sets.push(Set::Axis(i as usize, Vec2::new(*x, *y)));
            }
            for (names, down) in [(&c.down, true), (&c.up, false)] {
                for name in names {
                    let i = settings.button(name).ok_or_else(|| unknown(name))?;
                    sets.push(Set::Button(i as usize, down));
                }
            }
            changes.push((c.step, sets));
        }
        Ok(Replay {
            changes,
            next: 0,
            actions: Actions::for_settings(settings),
        })
    }

    /// The actions of `step`; steps must be asked for in increasing order.
    pub fn actions(&mut self, step: u64) -> &Actions {
        while let Some((at, sets)) = self.changes.get(self.next) {
            if *at > step {
                break;
            }
            for set in sets {
                match *set {
                    Set::Axis(i, v) => self.actions.axes[i] = v,
                    Set::Button(i, down) => self.actions.buttons[i] = down,
                }
            }
            self.next += 1;
        }
        &self.actions
    }
}

/// Builds an [`InputLog`] from the actions of successive steps.
pub struct Recorder {
    names: (Vec<String>, Vec<String>),
    last: Actions,
    log: InputLog,
}

impl Recorder {
    pub fn new(settings: &InputSettings) -> Recorder {
        Recorder {
            names: (
                settings.axes.iter().map(|a| a.name.clone()).collect(),
                settings.buttons.iter().map(|b| b.name.clone()).collect(),
            ),
            last: Actions::for_settings(settings),
            log: InputLog::default(),
        }
    }

    pub fn record(&mut self, step: u64, actions: &Actions) {
        let mut change = Change {
            step,
            ..Change::default()
        };
        for (i, (now, before)) in actions.axes.iter().zip(&self.last.axes).enumerate() {
            if now != before {
                change.axes.insert(self.names.0[i].clone(), now.to_array());
            }
        }
        for (i, (now, before)) in actions.buttons.iter().zip(&self.last.buttons).enumerate() {
            if now != before {
                let list = if *now {
                    &mut change.down
                } else {
                    &mut change.up
                };
                list.push(self.names.1[i].clone());
            }
        }
        if !(change.axes.is_empty() && change.down.is_empty() && change.up.is_empty()) {
            self.log.changes.push(change);
        }
        self.last.clone_from(actions);
    }

    pub fn finish(self) -> InputLog {
        self.log
    }
}
