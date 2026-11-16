// SPDX-License-Identifier: Apache-2.0

//! The Sariema actor pack: a skinned character or a rigid prop, with its
//! skeleton, clips, sockets and face atlases.
//!
//! A pack is the 4-byte [`MAGIC`], the [`VERSION`] as a little-endian `u32`,
//! then a postcard body holding an [`ActorPack`]. Conventions are the room
//! pack's: right-handed, Y up, metres, column-major matrices. An actor faces
//! +Z, glTF's front, with its feet at y = 0.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Aabb, Material, Texture};

#[cfg(feature = "demo")]
pub mod demo;
mod validate;

pub const MAGIC: [u8; 4] = *b"SRMK";
pub const VERSION: u32 = 1;

const HEADER_LEN: usize = MAGIC.len() + size_of::<u32>();

#[repr(C)]
#[derive(
    Clone, Copy, Debug, PartialEq, Serialize, Deserialize, bytemuck::Pod, bytemuck::Zeroable,
)]
pub struct SkinnedVertex {
    /// In the bind pose, model space.
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [u8; 4],
    /// Indices into [`ActorPack::joints`].
    pub joints: [u16; 4],
    /// Unsigned normalized; they sum to 65535.
    pub weights: [u16; 4],
}

const _: () = assert!(size_of::<SkinnedVertex>() == 52);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkinnedMesh {
    pub vertices: Vec<SkinnedVertex>,
    pub indices: Vec<u32>,
}

/// One drawable piece: a mesh with its material. Parts a game shows and
/// hides, such as held props, share a name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    /// Index into [`ActorPack::meshes`].
    pub mesh: u32,
    /// Index into [`ActorPack::materials`].
    pub material: u32,
    pub visible: bool,
    pub casts_shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: [f32; 3],
    /// Unit quaternion, `[x, y, z, w]`.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Joint {
    pub name: String,
    /// Always lower than this joint's own index, so parents come first.
    pub parent: Option<u32>,
    /// Relative to the parent.
    pub rest: Transform,
    /// Model space to this joint's space in the bind pose.
    pub inverse_bind: [f32; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpolation {
    Step,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Values {
    /// Translations or scales.
    Vec3(Vec<[f32; 3]>),
    /// Rotations as `[x, y, z, w]`.
    Quat(Vec<[f32; 4]>),
    /// Rotations as `[x, y, z, w]` in signed normalized 16-bit integers.
    QuatSnorm16(Vec<[i16; 4]>),
}

impl Values {
    pub fn len(&self) -> usize {
        match self {
            Values::Vec3(v) => v.len(),
            Values::Quat(v) => v.len(),
            Values::QuatSnorm16(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The keys of one property of one joint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    pub joint: u32,
    pub property: Property,
    pub interpolation: Interpolation,
    /// Seconds, ascending, within the clip's duration.
    pub times: Vec<f32>,
    pub values: Values,
}

/// A named instant of a clip, reported as an event when playback passes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub name: String,
    pub time: f32,
}

/// Cells of one face atlas over a clip, each held until the next.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceTrack {
    /// Index into [`ActorPack::faces`].
    pub face: u32,
    pub times: Vec<f32>,
    pub cells: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub name: String,
    /// Seconds.
    pub duration: f32,
    /// Whether the clip plays in a loop unless told otherwise.
    pub looping: bool,
    /// At most one per joint and property; joints without one keep their rest pose.
    pub channels: Vec<Channel>,
    pub markers: Vec<Marker>,
    pub faces: Vec<FaceTrack>,
    /// Model space, around every vertex of every part over the whole clip.
    pub bounds: Aabb,
    /// Metres per second the planted foot moves backwards at speed 1: the
    /// ground speed at which the feet do not slide. `None` without feet.
    pub ground_speed: Option<f32>,
}

/// A point fixed to a joint, where props attach.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Socket {
    pub name: String,
    pub joint: u32,
    /// The socket in its joint's space.
    pub offset: [f32; 16],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceFrame {
    pub name: String,
    /// Row-major index into the grid.
    pub cell: u32,
}

/// A grid of expressions in one texture, shown by moving the UV offset of
/// the materials that sample it: cell `c` is at
/// `origin + (c % columns, c / columns) × step`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceAtlas {
    pub name: String,
    /// Indices into [`ActorPack::materials`].
    pub materials: Vec<u32>,
    pub columns: u32,
    pub rows: u32,
    pub origin: [f32; 2],
    pub step: [f32; 2],
    pub frames: Vec<FaceFrame>,
    /// The cell shown before anything picks one.
    pub start: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActorPack {
    pub name: String,
    pub meshes: Vec<SkinnedMesh>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub parts: Vec<Part>,
    /// At least one; a rigid prop has one joint that every vertex follows.
    pub joints: Vec<Joint>,
    pub clips: Vec<Clip>,
    pub sockets: Vec<Socket>,
    pub faces: Vec<FaceAtlas>,
    /// Model space, around every part in the rest pose and in every clip.
    pub bounds: Aabb,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActorPackError {
    BadMagic,
    Version { found: u32 },
    Truncated,
    Invalid(String),
}

impl ActorPackError {
    pub fn code(&self) -> &'static str {
        match self {
            ActorPackError::BadMagic => "E_PACK_BAD_MAGIC",
            ActorPackError::Version { .. } => "E_PACK_VERSION",
            ActorPackError::Truncated => "E_PACK_TRUNCATED",
            ActorPackError::Invalid(_) => "E_SCHEMA",
        }
    }
}

impl fmt::Display for ActorPackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorPackError::BadMagic => {
                write!(f, "not an actor pack: the file does not start with SRMK")
            }
            ActorPackError::Version { found } => write!(
                f,
                "actor pack version {found} is not supported (expected {VERSION})"
            ),
            ActorPackError::Truncated => write!(f, "actor pack is truncated"),
            ActorPackError::Invalid(why) => write!(f, "actor pack is invalid: {why}"),
        }
    }
}

impl std::error::Error for ActorPackError {}

/// Writes [`MAGIC`], [`VERSION`] and the postcard body. Does not validate.
pub fn encode(actor: &ActorPack) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    postcard::to_extend(actor, out).expect("postcard cannot fail to serialize an ActorPack")
}

/// Decodes a pack and validates it; see [`validate`] for what that checks.
/// Never panics, and a length read from the input cannot cause a large
/// allocation (as for room packs).
pub fn decode(bytes: &[u8]) -> Result<ActorPack, ActorPackError> {
    let seen = bytes.len().min(MAGIC.len());
    if bytes[..seen] != MAGIC[..seen] {
        return Err(ActorPackError::BadMagic);
    }
    let Some((header, body)) = bytes.split_first_chunk::<HEADER_LEN>() else {
        return Err(ActorPackError::Truncated);
    };
    let found = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if found != VERSION {
        return Err(ActorPackError::Version { found });
    }
    let (actor, rest) = postcard::take_from_bytes::<ActorPack>(body).map_err(|e| match e {
        postcard::Error::DeserializeUnexpectedEnd => ActorPackError::Truncated,
        other => ActorPackError::Invalid(other.to_string()),
    })?;
    if !rest.is_empty() {
        return Err(ActorPackError::Invalid(format!(
            "{} unexpected bytes after the actor",
            rest.len()
        )));
    }
    validate(&actor).map_err(ActorPackError::Invalid)?;
    Ok(actor)
}

/// Every index points at something that exists, joints come after their
/// parents, every vertex's weights sum to 65535, key times are finite,
/// ascending and within their clip, each channel's values match its
/// property, and every bound is finite and ordered.
pub fn validate(actor: &ActorPack) -> Result<(), String> {
    validate::actor(actor)
}
