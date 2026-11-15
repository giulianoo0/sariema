// SPDX-License-Identifier: Apache-2.0

//! Builds Sariema room packs from a game's glTF kit and room files.

use std::path::Path;
use std::{fmt, io};

mod gltf;
mod kit;
mod room;
mod texture;

pub use kit::{Kit, KitImage, KitPiece, KitPrimitive, load_kit};
pub use room::{RoomSource, build_room};
pub use texture::mip_chain_rgba8;

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
    Schema(String),
}

impl ImportError {
    pub fn code(&self) -> &'static str {
        match self {
            ImportError::Io(_) => "E_IO",
            ImportError::Gltf(_) | ImportError::Schema(_) => "E_SCHEMA",
            ImportError::Extension(_) => "E_GLTF_EXTENSION",
            ImportError::PieceMissing { .. } => "E_ROOM_PIECE_MISSING",
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
            ImportError::Schema(why) => write!(f, "does not match the schema: {why}"),
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
