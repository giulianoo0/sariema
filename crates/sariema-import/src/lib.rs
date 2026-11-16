// SPDX-License-Identifier: Apache-2.0

//! Builds Sariema room packs from a game's glTF kit and room files.

use std::path::Path;
use std::{fmt, io};

mod actor;
mod collision;
mod gltf;
mod kit;
mod ktx2;
mod room;
mod texture;

pub use actor::{ActorSource, FaceSource, KeySettings, MaterialLook, SocketSource, build_actor};
pub use collision::{BakeSettings, CollisionSource, build_collision};
pub use kit::{Kit, KitHinge, KitImage, KitPiece, KitPrimitive, load_kit};
pub use ktx2::{Ktx2Settings, decode_ktx2, decode_ktx2_with};
pub use room::{CutawaySource, FogSource, Hemisphere, RoomSource, StyleSource, build_room};
pub use texture::{TextureMode, encode_texture, mip_chain_rgba8};

#[derive(Debug)]
pub enum ImportError {
    Io(std::io::Error),
    /// The file is not a glTF the reader can use.
    Gltf(String),
    /// The kit requires this glTF extension and the reader does not support it.
    Extension(String),
    PieceMissing {
        piece: String,
        room: String,
    },
    /// The room description hides a piece that none of its shown rooms place.
    HiddenPieceMissing {
        piece: String,
        room: String,
    },
    Schema(String),
    /// The ASTC encoder failed on this texture.
    Encode {
        texture: String,
        why: String,
    },
    /// An actor description names a joint, clip, material, face or frame
    /// the actor does not have.
    ActorName(String),
}

impl ImportError {
    pub fn code(&self) -> &'static str {
        match self {
            ImportError::Io(_) => "E_IO",
            ImportError::Gltf(_) | ImportError::Schema(_) => "E_SCHEMA",
            ImportError::Extension(_) => "E_GLTF_EXTENSION",
            ImportError::PieceMissing { .. } | ImportError::HiddenPieceMissing { .. } => {
                "E_ROOM_PIECE_MISSING"
            }
            ImportError::Encode { .. } => "E_TEXTURE_ENCODE",
            ImportError::ActorName(_) => "E_ACTOR_NAME",
        }
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Io(e) => write!(f, "{e}"),
            ImportError::Gltf(why) => write!(f, "unusable glTF: {why}"),
            ImportError::Extension(name) => {
                write!(f, "the kit requires the unsupported glTF extension {name}")
            }
            ImportError::PieceMissing { piece, room } => {
                write!(f, "room {room} uses piece {piece}, which is not in the kit")
            }
            ImportError::HiddenPieceMissing { piece, room } => {
                write!(
                    f,
                    "room {room} hides piece {piece}, which no room in show places"
                )
            }
            ImportError::Schema(why) => write!(f, "does not match the schema: {why}"),
            ImportError::Encode { texture, why } => {
                write!(f, "texture {texture} could not be compressed: {why}")
            }
            ImportError::ActorName(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ImportError::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Reads a whole file; the error names the path.
fn read(path: &Path) -> Result<Vec<u8>, ImportError> {
    std::fs::read(path)
        .map_err(|e| ImportError::Io(io::Error::new(e.kind(), format!("{}: {e}", path.display()))))
}
