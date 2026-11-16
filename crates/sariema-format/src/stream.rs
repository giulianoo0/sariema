// SPDX-License-Identifier: Apache-2.0

//! A room pack decoded a share at a time, for a host without threads that
//! cannot spend a whole room's decode in one frame: each
//! [`RoomDecoder::step`] reads about a byte budget's worth, one mesh or one
//! texture at a least, and the last one gives the room, checked as
//! [`decode`](crate::decode) checks it.
//!
//! A postcard body is its fields one after the other, so the room's are
//! read in the order [`RoomPack`] declares them.

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::{
    Ambient, Background, Draw, FormatError, Group, HEADER_LEN, Light, Look, MAGIC, Material, Mesh,
    Node, Piece, RoomCamera, RoomPack, Stats, Sun, Texture, VERSION, validate_mesh, validate_rest,
};

/// The fields of [`RoomPack`] after its textures, in its order.
#[derive(Deserialize)]
struct Tail {
    draws: Vec<Draw>,
    lights: Vec<Light>,
    ambient: Ambient,
    sun: Option<Sun>,
    background: Background,
    look: Look,
    camera: RoomCamera,
    stats: Stats,
    groups: Vec<Group>,
    nodes: Vec<Node>,
    pieces: Vec<Piece>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Header,
    Name,
    MeshCount,
    Mesh(usize),
    Materials,
    TextureCount,
    Texture(usize),
    Tail,
    Done,
}

pub struct RoomDecoder<B> {
    bytes: B,
    at: usize,
    part: Part,
    name: String,
    meshes: Vec<Mesh>,
    materials: Vec<Material>,
    textures: Vec<Texture>,
}

impl<B: AsRef<[u8]>> RoomDecoder<B> {
    pub fn new(bytes: B) -> RoomDecoder<B> {
        RoomDecoder {
            bytes,
            at: 0,
            part: Part::Header,
            name: String::new(),
            meshes: Vec::new(),
            materials: Vec::new(),
            textures: Vec::new(),
        }
    }

    /// Bytes read so far, and in all.
    pub fn progress(&self) -> (usize, usize) {
        (self.at, self.bytes.as_ref().len())
    }

    /// Reads parts of the room until about `budget` bytes went or the room
    /// is whole, which it then gives. After an error or the room, it gives
    /// nothing more.
    pub fn step(&mut self, budget: usize) -> Result<Option<RoomPack>, FormatError> {
        let start = self.at;
        while self.part != Part::Done {
            let read = self.read_part();
            let read = match (read, self.part) {
                (Ok(()), Part::Done) => self.finish().map(Some),
                (Ok(()), _) if self.at - start >= budget => return Ok(None),
                (Ok(()), _) => continue,
                (Err(e), _) => Err(e),
            };
            self.part = Part::Done;
            return read;
        }
        Ok(None)
    }

    /// Decodes the whole room; the same as [`decode`](crate::decode).
    pub fn decode(bytes: B) -> Result<RoomPack, FormatError> {
        let mut decoder = RoomDecoder::new(bytes);
        loop {
            if let Some(room) = decoder.step(usize::MAX)? {
                return Ok(room);
            }
        }
    }

    fn read_part(&mut self) -> Result<(), FormatError> {
        self.part = match self.part {
            Part::Header => {
                let bytes = self.bytes.as_ref();
                let seen = bytes.len().min(MAGIC.len());
                if bytes[..seen] != MAGIC[..seen] {
                    return Err(FormatError::BadMagic);
                }
                let Some((header, _)) = bytes.split_first_chunk::<HEADER_LEN>() else {
                    return Err(FormatError::Truncated);
                };
                let found = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
                if found != VERSION {
                    return Err(FormatError::Version { found });
                }
                self.at = HEADER_LEN;
                Part::Name
            }
            Part::Name => {
                self.name = self.take()?;
                Part::MeshCount
            }
            Part::MeshCount => {
                let count: usize = self.take()?;
                self.meshes = Vec::with_capacity(count.min(self.left()));
                Part::Mesh(count)
            }
            Part::Mesh(0) => Part::Materials,
            Part::Mesh(left) => {
                let mesh: Mesh = self.take()?;
                validate_mesh(self.meshes.len(), &mesh)?;
                self.meshes.push(mesh);
                Part::Mesh(left - 1)
            }
            Part::Materials => {
                self.materials = self.take()?;
                Part::TextureCount
            }
            Part::TextureCount => {
                let count: usize = self.take()?;
                self.textures = Vec::with_capacity(count.min(self.left()));
                Part::Texture(count)
            }
            Part::Texture(0) => Part::Tail,
            Part::Texture(left) => {
                let texture = self.take()?;
                self.textures.push(texture);
                Part::Texture(left - 1)
            }
            Part::Tail | Part::Done => Part::Done,
        };
        Ok(())
    }

    fn finish(&mut self) -> Result<RoomPack, FormatError> {
        let tail: Tail = self.take()?;
        let rest = self.left();
        if rest > 0 {
            return Err(FormatError::Invalid(format!(
                "{rest} unexpected bytes after the room"
            )));
        }
        let room = RoomPack {
            name: std::mem::take(&mut self.name),
            meshes: std::mem::take(&mut self.meshes),
            materials: std::mem::take(&mut self.materials),
            textures: std::mem::take(&mut self.textures),
            draws: tail.draws,
            lights: tail.lights,
            ambient: tail.ambient,
            sun: tail.sun,
            background: tail.background,
            look: tail.look,
            camera: tail.camera,
            stats: tail.stats,
            groups: tail.groups,
            nodes: tail.nodes,
            pieces: tail.pieces,
        };
        validate_rest(&room)?;
        Ok(room)
    }

    fn left(&self) -> usize {
        self.bytes.as_ref().len() - self.at
    }

    fn take<T: DeserializeOwned>(&mut self) -> Result<T, FormatError> {
        let bytes = &self.bytes.as_ref()[self.at..];
        let (value, rest) = postcard::take_from_bytes::<T>(bytes).map_err(|e| match e {
            postcard::Error::DeserializeUnexpectedEnd => FormatError::Truncated,
            other => FormatError::Invalid(other.to_string()),
        })?;
        self.at += bytes.len() - rest.len();
        Ok(value)
    }
}
