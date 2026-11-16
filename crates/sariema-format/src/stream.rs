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

use crate::previous::{Cutaway9, missing_fields, upgrade};
use crate::{
    Ambient, Background, CharacterLight, Cutaway, Draw, Fog, FormatError, Group, HEADER_LEN, Light,
    Look, Material, Mesh, Node, Piece, RoomCamera, RoomPack, Stats, Sun, Texture, VERSION,
    header_version, validate_mesh, validate_rest,
};

/// The fields of [`RoomPack`] after its textures, in its order, with the
/// cutaways of the version read.
#[derive(Deserialize)]
struct Tail<C> {
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
    character: CharacterLight,
    fog: Fog,
    cutaways: Vec<C>,
}

impl Tail<Cutaway9> {
    fn upgraded(self) -> Tail<Cutaway> {
        Tail {
            draws: self.draws,
            lights: self.lights,
            ambient: self.ambient,
            sun: self.sun,
            background: self.background,
            look: self.look,
            camera: self.camera,
            stats: self.stats,
            groups: self.groups,
            nodes: self.nodes,
            pieces: self.pieces,
            character: self.character,
            fog: self.fog,
            cutaways: self.cutaways.into_iter().map(Cutaway::from).collect(),
        }
    }
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
    version: u32,
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
            version: VERSION,
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
                self.version = header_version(self.bytes.as_ref())?;
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
        let tail: Tail<Cutaway> = match self.version {
            VERSION => self.take()?,
            _ => {
                let missing = missing_fields(self.version);
                let padded = [&self.bytes.as_ref()[self.at..], &missing].concat();
                let (tail, rest) = read::<Tail<Cutaway9>>(&padded)?;
                self.at = self.bytes.as_ref().len() - rest.len().min(self.left());
                tail.upgraded()
            }
        };
        let rest = self.left();
        if rest > 0 {
            return Err(FormatError::Invalid(format!(
                "{rest} unexpected bytes after the room"
            )));
        }
        let mut room = RoomPack {
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
            character: tail.character,
            fog: tail.fog,
            cutaways: tail.cutaways,
        };
        upgrade(self.version, &mut room);
        validate_rest(&room)?;
        Ok(room)
    }

    fn left(&self) -> usize {
        self.bytes.as_ref().len() - self.at
    }

    fn take<T: DeserializeOwned>(&mut self) -> Result<T, FormatError> {
        let bytes = &self.bytes.as_ref()[self.at..];
        let (value, rest) = read(bytes)?;
        self.at += bytes.len() - rest.len();
        Ok(value)
    }
}

fn read<T: DeserializeOwned>(bytes: &[u8]) -> Result<(T, &[u8]), FormatError> {
    postcard::take_from_bytes::<T>(bytes).map_err(|e| match e {
        postcard::Error::DeserializeUnexpectedEnd => FormatError::Truncated,
        other => FormatError::Invalid(other.to_string()),
    })
}
