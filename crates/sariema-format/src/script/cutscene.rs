// SPDX-License-Identifier: Apache-2.0

//! Compiled cutscenes as the game pack stores them. Authored as JSON
//! (`schemas/cutscene.schema.json`) and compiled by `sariema build`.
//!
//! Roles are indices into [`Cutscene::roles`]; names are
//! [`name_id`](super::name_id) ids; enum fields hold the `u8` values of the
//! batch's wire enums and are checked by [`super::decode_game`].

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cutscene {
    pub name: String,
    pub skippable: bool,
    pub roles: Vec<Role>,
    pub body: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    /// Where the role's actor must be when the cutscene starts; `None` only
    /// for a role the cutscene spawns itself.
    pub start: Option<Place>,
    /// Actor asset a `spawn` block creates.
    pub actor: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub position: [f32; 3],
    /// Degrees; 0 faces −Z.
    pub yaw: f32,
}

/// Packs store each variant by its position, and earlier game packs are
/// read with this enum: a new variant goes at the end.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Block {
    Sequence(Vec<Block>),
    /// Every branch starts together; the block ends when all have.
    Parallel(Vec<Vec<Block>>),
    Wait(f32),
    /// Ends on the event, or after `timeout` seconds when given.
    WaitFor {
        on: Await,
        timeout: Option<f32>,
    },
    Branch {
        flag: u32,
        then: Vec<Block>,
        otherwise: Vec<Block>,
    },
    /// Ends at once; the camera keeps blending. `blend` 0 is a cut.
    Shot {
        shot: Shot,
        blend: f32,
        ease: u8,
    },
    /// Ends when the actor has arrived and turned to `face`, or to the
    /// direction it walked in.
    Walk {
        role: u16,
        to: [f32; 3],
        speed: f32,
        face: Option<f32>,
    },
    /// `speed` scales the clip's rate; its time follows the steps.
    Clip {
        role: u16,
        clip: u32,
        mode: u8,
        blend: f32,
        speed: f32,
    },
    /// Ends when the actor faces the target.
    Face {
        role: u16,
        toward: Toward,
    },
    LookAt {
        role: u16,
        target: Option<u16>,
    },
    Line {
        speaker: u16,
        text: String,
        voice: u32,
        advance: u8,
        /// Seconds an `Auto` line stays.
        duration: f32,
    },
    Sound {
        sound: u32,
        bus: u8,
        at: Option<u16>,
        volume: f32,
    },
    /// `None` stops the music.
    Music {
        music: Option<u32>,
        fade: f32,
    },
    /// Ends when the fade has reached `to`.
    Fade {
        to: f32,
        duration: f32,
        color: [f32; 3],
    },
    Letterbox {
        to: f32,
        duration: f32,
    },
    SetFlag {
        flag: u32,
        on: bool,
    },
    Spawn {
        role: u16,
        at: Place,
    },
    Despawn {
        role: u16,
    },
    Show {
        role: u16,
    },
    Hide {
        role: u16,
    },
    /// The only discontinuous move a cutscene may make, and only under the
    /// declared cover; `None` is allowed only while the role is not visible.
    Teleport {
        role: u16,
        to: Place,
        cover: Option<Cover>,
    },
    Call {
        call: u32,
    },
    /// Plays the world at `scale` times real time from here, reached over
    /// `blend` seconds; the cutscene's end gives back the scale it found.
    TimeScale {
        scale: f32,
        blend: f32,
    },
    CameraShake {
        amplitude: f32,
        frequency: f32,
        duration: f32,
    },
    /// Turns a hinged node with the time of `role`'s clip `clip`: from the
    /// angle its scene parameter `hinge` holds when the block starts, at
    /// `span[0]` seconds of the clip, to `to` degrees at `span[1]`, eased.
    /// Ends once the clip is past `span[1]`, or at once when the role plays
    /// another clip or none.
    Hinge {
        hinge: u32,
        role: u16,
        clip: u32,
        span: [f32; 2],
        to: f32,
        ease: u8,
    },
    /// Plays `clip` on `role` from `from` seconds into it, still for `hold`
    /// seconds first, moving the role's root only within `spans`, as the
    /// `clip_path` command does; then the clips of `then` in a row, each
    /// from where the one before ends. Ends when the last clip has played
    /// through and the root has landed, or, with `hold_end`, is held where
    /// the last span left it.
    Path {
        role: u16,
        clip: u32,
        mode: u8,
        blend: f32,
        from: f32,
        hold: f32,
        hold_end: bool,
        spans: Vec<PathSpan>,
        then: Vec<Then>,
    },
    /// Tweens scene parameters of the active room, each a name id with the
    /// value it goes to, as one `scene_tween`; ends at once. A skip lands
    /// them, and every one still under way, at their values.
    Scene {
        values: Vec<(u32, f32)>,
        duration: f32,
        ease: u8,
    },
}

/// A span of a [`Block::Path`]'s clip, `from` to `until` seconds of it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathSpan {
    pub from: f32,
    pub until: f32,
    pub rate: f32,
    pub to: Option<[f32; 3]>,
    /// Degrees.
    pub yaw: Option<f32>,
    pub ease: u8,
    pub arc: f32,
}

/// A clip that follows a [`Block::Path`]'s clip, or the one before it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Then {
    pub clip: u32,
    pub mode: u8,
    pub blend: f32,
    pub speed: f32,
    /// Seconds into the clip before at which this one begins; 0 is that
    /// clip's end.
    pub after: f32,
}

/// The point of a role a shot aims at: its origin, or a bone or socket
/// (`anchor` holds an [`Anchor`](super::Anchor) value, `name` its id).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub anchor: u8,
    pub name: u32,
}

impl Point {
    pub const ORIGIN: Point = Point { anchor: 0, name: 0 };
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Toward {
    Yaw(f32),
    Role(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shot {
    Follow {
        subject: u16,
        distance: f32,
        pitch: f32,
        yaw: f32,
        height: f32,
        fov: f32,
        point: Point,
    },
    Frame {
        a: u16,
        b: u16,
        pitch: f32,
        yaw: f32,
        height: f32,
        fov: f32,
        margin: f32,
        points: [Point; 2],
    },
    Fixed {
        position: [f32; 3],
        look_at: [f32; 3],
        fov: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Await {
    /// A `signal` command from the game.
    Signal(u32),
    Arrived(u16),
    ClipFinished(u16),
    /// A role's clip reached a marker: a beat a cut can land on.
    ClipMarker {
        role: u16,
        clip: u32,
        marker: u32,
    },
}

/// What hides a teleport: a camera cut in the same step, or a full fade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cover {
    Cut,
    Fade,
}
