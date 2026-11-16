// SPDX-License-Identifier: Apache-2.0

//! The game pack and the batch of commands and events between a game's
//! WebAssembly logic and the engine.
//!
//! A game pack is the 4-byte [`GAME_MAGIC`], [`GAME_VERSION`] as a
//! little-endian `u32`, then a postcard body holding a [`GamePack`].
//!
//! A batch is a run of records: a little-endian `u16` opcode, then the
//! fields [`COMMANDS`] or [`EVENTS`] list for it, in order, with no padding.

use std::fmt;
use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

#[macro_use]
mod codegen;
pub mod cutscene;
mod table;
mod wire;

pub use table::{BATCH_VERSION, COMMANDS, Command, ENUMS, EVENTS, Event};
pub use wire::{
    Advance, Anchor, Bus, Bytes, ClipMode, Controller, Ease, Entity, EnumSpec, FieldSpec, Kind,
    MessageSpec, Orientation, Property, Reader, Sink, Str, Value, VariantSpec, Vec3, Wire,
    WireError, name_id,
};

use cutscene::{Await, Block, Cutscene, Place, Point, Shot, Toward};

pub const GAME_MAGIC: [u8; 4] = *b"SRMG";
pub const GAME_VERSION: u32 = 1;

const HEADER_LEN: usize = GAME_MAGIC.len() + size_of::<u32>();

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GamePack {
    pub name: String,
    /// The [`BATCH_VERSION`] the logic was built against.
    pub batch_version: u32,
    /// The logic module.
    pub wasm: Vec<u8>,
    pub cutscenes: Vec<Cutscene>,
    /// Every name the game's data refers to by id, so tools can print names.
    pub names: Vec<String>,
    /// The project's runtime settings, a JSON object the runtime checks at load.
    pub settings: String,
}

impl Sink for Vec<u8> {
    fn put(&mut self, bytes: &[u8]) {
        self.extend_from_slice(bytes);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameFormatError {
    BadMagic,
    Version { found: u32 },
    Truncated,
    Invalid(String),
    BatchVersion { found: u32 },
}

impl GameFormatError {
    pub fn code(&self) -> &'static str {
        match self {
            GameFormatError::BadMagic => "E_PACK_BAD_MAGIC",
            GameFormatError::Version { .. } => "E_PACK_VERSION",
            GameFormatError::Truncated => "E_PACK_TRUNCATED",
            GameFormatError::Invalid(_) => "E_SCHEMA",
            GameFormatError::BatchVersion { .. } => "E_BATCH_VERSION",
        }
    }
}

impl fmt::Display for GameFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameFormatError::BadMagic => {
                write!(f, "not a game pack: the file does not start with SRMG")
            }
            GameFormatError::Version { found } => write!(
                f,
                "game pack version {found} is not supported (expected {GAME_VERSION})"
            ),
            GameFormatError::Truncated => write!(f, "game pack is truncated"),
            GameFormatError::Invalid(why) => write!(f, "game pack is invalid: {why}"),
            GameFormatError::BatchVersion { found } => write!(
                f,
                "the game targets batch version {found}; this engine speaks {BATCH_VERSION}"
            ),
        }
    }
}

impl std::error::Error for GameFormatError {}

pub fn encode_game(game: &GamePack) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + game.wasm.len());
    out.extend_from_slice(&GAME_MAGIC);
    out.extend_from_slice(&GAME_VERSION.to_le_bytes());
    postcard::to_extend(game, out).expect("postcard cannot fail to serialize a GamePack")
}

/// Decodes and validates a game pack: the batch version must be this
/// engine's, names distinct with distinct ids, and every cutscene's role
/// indices, enum values and numbers in range.
pub fn decode_game(bytes: &[u8]) -> Result<GamePack, GameFormatError> {
    let seen = bytes.len().min(GAME_MAGIC.len());
    if bytes[..seen] != GAME_MAGIC[..seen] {
        return Err(GameFormatError::BadMagic);
    }
    let Some((header, body)) = bytes.split_first_chunk::<HEADER_LEN>() else {
        return Err(GameFormatError::Truncated);
    };
    let found = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if found != GAME_VERSION {
        return Err(GameFormatError::Version { found });
    }
    let (game, rest) = postcard::take_from_bytes::<GamePack>(body).map_err(|e| match e {
        postcard::Error::DeserializeUnexpectedEnd => GameFormatError::Truncated,
        other => GameFormatError::Invalid(other.to_string()),
    })?;
    if !rest.is_empty() {
        return Err(GameFormatError::Invalid(format!(
            "{} unexpected bytes after the game",
            rest.len()
        )));
    }
    if game.batch_version != BATCH_VERSION {
        return Err(GameFormatError::BatchVersion {
            found: game.batch_version,
        });
    }
    validate_names(&game.names)?;
    for cutscene in &game.cutscenes {
        validate_cutscene(cutscene).map_err(|why| {
            GameFormatError::Invalid(format!("cutscene {}: {why}", cutscene.name))
        })?;
    }
    Ok(game)
}

fn validate_names(names: &[String]) -> Result<(), GameFormatError> {
    let mut ids: Vec<(u32, &str)> = names.iter().map(|n| (name_id(n), n.as_str())).collect();
    ids.sort_unstable();
    for pair in ids.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(GameFormatError::Invalid(format!(
                "names {:?} and {:?} have the same id {:#010x}",
                pair[0].1, pair[1].1, pair[0].0
            )));
        }
    }
    Ok(())
}

fn validate_cutscene(c: &Cutscene) -> Result<(), String> {
    for role in &c.roles {
        if let Some(place) = &role.start {
            place_ok(place)?;
        }
    }
    let roles = c.roles.len();
    c.body.iter().try_for_each(|b| validate_block(b, roles))
}

fn validate_block(block: &Block, roles: usize) -> Result<(), String> {
    let role = |r: u16| {
        if (r as usize) < roles {
            Ok(())
        } else {
            Err(format!("role {r} of {roles}"))
        }
    };
    let finite = |what: &str, v: f32| {
        if v.is_finite() {
            Ok(())
        } else {
            Err(format!("{what} is {v}"))
        }
    };
    let seconds = |what: &str, v: f32| {
        if v.is_finite() && v >= 0.0 {
            Ok(())
        } else {
            Err(format!("{what} is {v}, not a duration"))
        }
    };
    let unit = |what: &str, v: f32| {
        if (0.0..=1.0).contains(&v) {
            Ok(())
        } else {
            Err(format!("{what} is {v}, not in [0, 1]"))
        }
    };
    let known = |what: &str, ok: bool| {
        if ok {
            Ok(())
        } else {
            Err(format!("bad {what}"))
        }
    };
    let all = |blocks: &[Block]| blocks.iter().try_for_each(|b| validate_block(b, roles));
    match block {
        Block::Sequence(blocks) => all(blocks),
        Block::Parallel(branches) => branches.iter().try_for_each(|b| all(b)),
        Block::Wait(t) => seconds("wait", *t),
        Block::WaitFor { on, timeout } => {
            if let Some(t) = timeout {
                seconds("timeout", *t)?;
            }
            match on {
                Await::Signal(_) => Ok(()),
                Await::Arrived(r) | Await::ClipFinished(r) | Await::ClipMarker { role: r, .. } => {
                    role(*r)
                }
            }
        }
        Block::Branch {
            then, otherwise, ..
        } => {
            all(then)?;
            all(otherwise)
        }
        Block::Shot { shot, blend, ease } => {
            seconds("blend", *blend)?;
            known("ease", Ease::from_u8(*ease).is_some())?;
            match shot {
                Shot::Follow {
                    subject,
                    distance,
                    pitch,
                    yaw,
                    height,
                    fov,
                    point,
                } => {
                    role(*subject)?;
                    point_ok(point)?;
                    [distance, pitch, yaw, height, fov]
                        .into_iter()
                        .try_for_each(|v| finite("shot", *v))
                }
                Shot::Frame {
                    a,
                    b,
                    pitch,
                    yaw,
                    height,
                    fov,
                    margin,
                    points,
                } => {
                    role(*a)?;
                    role(*b)?;
                    points.iter().try_for_each(point_ok)?;
                    [pitch, yaw, height, fov, margin]
                        .into_iter()
                        .try_for_each(|v| finite("shot", *v))
                }
                Shot::Fixed {
                    position,
                    look_at,
                    fov,
                } => position
                    .iter()
                    .chain(look_at)
                    .chain([fov])
                    .try_for_each(|v| finite("shot", *v)),
            }
        }
        Block::Walk {
            role: r,
            to,
            speed,
            face,
        } => {
            role(*r)?;
            to.iter().try_for_each(|v| finite("walk target", *v))?;
            if !(speed.is_finite() && *speed > 0.0) {
                return Err(format!("walk speed is {speed}"));
            }
            face.map_or(Ok(()), |f| finite("walk face", f))
        }
        Block::Clip {
            role: r,
            mode,
            blend,
            speed,
            ..
        } => {
            role(*r)?;
            known("clip mode", ClipMode::from_u8(*mode).is_some())?;
            seconds("clip blend", *blend)?;
            positive("clip speed", *speed)
        }
        Block::Face { role: r, toward } => {
            role(*r)?;
            match toward {
                Toward::Yaw(y) => finite("face yaw", *y),
                Toward::Role(t) => role(*t),
            }
        }
        Block::LookAt { role: r, target } => {
            role(*r)?;
            target.map_or(Ok(()), role)
        }
        Block::Line {
            speaker,
            advance,
            duration,
            ..
        } => {
            role(*speaker)?;
            known("line advance", Advance::from_u8(*advance).is_some())?;
            seconds("line duration", *duration)
        }
        Block::Sound {
            bus, at, volume, ..
        } => {
            known("bus", Bus::from_u8(*bus).is_some())?;
            at.map_or(Ok(()), role)?;
            finite("volume", *volume)
        }
        Block::Music { fade, .. } => seconds("music fade", *fade),
        Block::Fade {
            to,
            duration,
            color,
        } => {
            unit("fade", *to)?;
            seconds("fade duration", *duration)?;
            color.iter().try_for_each(|c| finite("fade color", *c))
        }
        Block::Letterbox { to, duration } => {
            unit("letterbox", *to)?;
            seconds("letterbox duration", *duration)
        }
        Block::SetFlag { .. } | Block::Call { .. } => Ok(()),
        Block::Spawn { role: r, at } => {
            role(*r)?;
            place_ok(at)
        }
        Block::Despawn { role: r } | Block::Show { role: r } | Block::Hide { role: r } => role(*r),
        Block::Teleport { role: r, to, .. } => {
            role(*r)?;
            place_ok(to)
        }
        Block::TimeScale { scale, blend } => {
            positive("time scale", *scale)?;
            seconds("time scale blend", *blend)
        }
        Block::CameraShake {
            amplitude,
            frequency,
            duration,
        } => {
            finite("shake amplitude", *amplitude)?;
            finite("shake frequency", *frequency)?;
            seconds("shake duration", *duration)
        }
    }
}

fn positive(what: &str, v: f32) -> Result<(), String> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(format!("{what} is {v}, not above 0"))
    }
}

fn point_ok(point: &Point) -> Result<(), String> {
    if Anchor::from_u8(point.anchor).is_some() {
        Ok(())
    } else {
        Err(format!("bad anchor {}", point.anchor))
    }
}

fn place_ok(place: &Place) -> Result<(), String> {
    if place
        .position
        .iter()
        .chain([&place.yaw])
        .all(|v| v.is_finite())
    {
        Ok(())
    } else {
        Err(format!("place {place:?} is not finite"))
    }
}

/// A record of a batch that failed to decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchError {
    /// Index of the record in the batch.
    pub record: usize,
    /// Byte offset where the record starts.
    pub offset: usize,
    pub error: WireError,
}

impl BatchError {
    pub fn code(&self) -> &'static str {
        self.error.code()
    }
}

impl fmt::Display for BatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = format!("record {} at byte {}", self.record, self.offset);
        match self.error {
            WireError::Truncated => write!(f, "batch is truncated in {at}"),
            WireError::UnknownOpcode(op) => write!(f, "unknown opcode {op:#06x} in {at}"),
            WireError::Invalid(ty) => write!(f, "invalid {ty} value in {at}"),
        }
    }
}

impl std::error::Error for BatchError {}

/// A message type a batch holds: [`Command`] or [`Event`].
pub trait Message<'a>: Sized {
    fn read(r: &mut Reader<'a>) -> Result<Self, WireError>;
}

impl<'a> Message<'a> for Command<'a> {
    fn read(r: &mut Reader<'a>) -> Result<Self, WireError> {
        Command::read(r)
    }
}

impl<'a> Message<'a> for Event<'a> {
    fn read(r: &mut Reader<'a>) -> Result<Self, WireError> {
        Event::read(r)
    }
}

/// The records of a batch in order, borrowing from it. Stops after the
/// first error, which it yields.
pub struct Records<'a, M> {
    reader: Reader<'a>,
    record: usize,
    failed: bool,
    message: PhantomData<M>,
}

pub fn records<'a, M: Message<'a>>(batch: &'a [u8]) -> Records<'a, M> {
    Records {
        reader: Reader::new(batch),
        record: 0,
        failed: false,
        message: PhantomData,
    }
}

impl<'a, M: Message<'a>> Iterator for Records<'a, M> {
    type Item = Result<M, BatchError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.reader.is_empty() {
            return None;
        }
        let offset = self.reader.position();
        let record = self.record;
        self.record += 1;
        Some(M::read(&mut self.reader).map_err(|error| {
            self.failed = true;
            BatchError {
                record,
                offset,
                error,
            }
        }))
    }
}
