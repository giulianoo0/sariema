// SPDX-License-Identifier: Apache-2.0

//! What the game asked of the other modules and of the host, as plain data.
//! The per-module queues hold one step's requests and are emptied when the
//! next step starts; the host queue keeps messages until the host takes them.

use glam::Vec3;
use sariema_format::script::{Bus, ClipMode, Entity, Orientation};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipRequest {
    pub entity: Entity,
    pub clip: u32,
    pub mode: ClipMode,
    /// Crossfade seconds.
    pub blend: f32,
    pub speed: f32,
    /// Seconds into the clip it starts at.
    pub from: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundRequest {
    pub sound: u32,
    pub bus: Bus,
    /// NONE plays without position.
    pub emitter: Entity,
    pub volume: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MusicRequest {
    Play { music: u32, fade: f32 },
    Stop { fade: f32 },
    BusVolume { bus: Bus, volume: f32, fade: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub enum UiOp {
    Visible { node: u32, visible: bool },
    Text { node: u32, text: String },
    Value { node: u32, value: f32 },
    Play { animation: u32 },
}

/// Movement and collision requests for a collision module. The built-in
/// mover acts on the same requests when it is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PhysicsIntent {
    MoveTo {
        entity: Entity,
        target: Vec3,
        speed: f32,
    },
    Velocity {
        entity: Entity,
        velocity: Vec3,
    },
    Stop {
        entity: Entity,
    },
    Teleport {
        entity: Entity,
        position: Vec3,
        yaw: f32,
    },
    /// Hold exactly at `position` facing `yaw`, collider off, with no
    /// gravity and no push-out, until the next movement intent.
    Hold {
        entity: Entity,
        position: Vec3,
        yaw: f32,
    },
    Knockback {
        entity: Entity,
        impulse: Vec3,
    },
    Collider {
        entity: Entity,
        enabled: bool,
    },
    SpeedScale {
        entity: Entity,
        scale: f32,
    },
    /// Turn in place to `yaw` degrees.
    Face {
        entity: Entity,
        yaw: f32,
    },
    Traverse {
        entity: Entity,
        traversal: u32,
        duration: f32,
    },
    /// A skipped cutscene walk just placed `entity` at `target`. A module
    /// that could not have walked it there puts it back where it stood and
    /// reports `blocked`, so the walk ends as it would have played.
    SkippedWalk {
        entity: Entity,
        target: Vec3,
    },
    /// Turn no faster than `rate` degrees per second; none is
    /// `motion.turn_rate`.
    TurnRate {
        entity: Entity,
        rate: Option<f32>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookAt {
    pub entity: Entity,
    /// NONE stops looking.
    pub target: Entity,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HostOut {
    Ready,
    Exit,
    Orientation(Orientation),
    Save {
        key: String,
        value: Vec<u8>,
    },
    LoadSaves,
    Message {
        topic: String,
        payload: Vec<u8>,
    },
    /// The game stopped for good; `code` is one of the `E_GAME_*` or `E_BATCH_*` codes.
    Stopped {
        code: &'static str,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum HostIn {
    /// Every saved key, answering `save_load`.
    Saves(Vec<(String, Vec<u8>)>),
    Message {
        topic: String,
        payload: Vec<u8>,
    },
    Orientation(Orientation),
    Exit,
}

#[derive(Clone, Debug, Default)]
pub struct Outbox {
    pub clips: Vec<ClipRequest>,
    pub sounds: Vec<SoundRequest>,
    pub music: Vec<MusicRequest>,
    pub ui: Vec<UiOp>,
    pub physics: Vec<PhysicsIntent>,
    pub look_at: Vec<LookAt>,
    pub host: Vec<HostOut>,
}

impl Outbox {
    pub fn begin_step(&mut self) {
        self.clips.clear();
        self.sounds.clear();
        self.music.clear();
        self.ui.clear();
        self.physics.clear();
        self.look_at.clear();
    }
}
