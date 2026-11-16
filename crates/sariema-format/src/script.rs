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
mod previous;
mod span;
mod table;
mod wire;

pub use span::{CLIP_SPAN, ClipSpan, clip_spans};
pub use table::{BATCH_VERSION, COMMANDS, Command, ENUMS, EVENTS, Event};
pub use wire::{
    Advance, Anchor, Bus, Bytes, ClipMode, Controller, Ease, Entity, EnumSpec, FieldSpec, Kind,
    MessageSpec, Orientation, PointList, Points, Property, Reader, RoomTransition, Sink, Str,
    Value, VariantSpec, Vec3, Wire, WireError, name_id, put_scene_value,
};

use crate::scenery::{RoomIndex, Target};
use cutscene::{Await, Block, Cutscene, Place, Point, Shot, Toward};

/// Bytes of one parameter in a `scene_tween`'s `values`.
const SCENE_VALUE: usize = 8;

/// The parameters of a `scene_tween`'s `values` with the value each goes
/// to, as [`put_scene_value`] writes them; `None` when its length is not a
/// whole number of them.
pub fn scene_values(values: &[u8]) -> Option<impl Iterator<Item = (u32, f32)> + Clone + '_> {
    let (pairs, rest) = values.as_chunks::<SCENE_VALUE>();
    rest.is_empty().then(|| {
        pairs.iter().map(|&[a, b, c, d, e, f, g, h]| {
            let param = u32::from_le_bytes([a, b, c, d]);
            (param, f32::from_le_bytes([e, f, g, h]))
        })
    })
}

pub const GAME_MAGIC: [u8; 4] = *b"SRMG";
pub const GAME_VERSION: u32 = 4;

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
    pub assets: GameAssets,
}

/// What a game draws, hears and collides with, carried in its pack so one
/// file runs it on every host.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameAssets {
    /// The rooms it plays in, the first where it starts. Every room has a
    /// collision pack or none has.
    pub rooms: Vec<GameRoom>,
    /// Encoded actor packs (`SRMK`) by name; an entity spawned with the
    /// name's id as its `asset` is drawn and animated with it.
    pub actors: Vec<(String, Vec<u8>)>,
    /// The sound bank (`SRMS`).
    pub sounds: Option<Vec<u8>>,
    pub hud: Option<Hud>,
}

/// A room of a game pack: what the host draws, what the engine collides
/// with, and what the step addresses by name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GameRoom {
    pub name: String,
    /// The encoded room pack (`SRMA`), which the engine never decodes: the
    /// host does, off the step.
    pub room: Option<Vec<u8>>,
    /// The room's collision pack (`SRMC`); with it the engine moves actors
    /// with collision instead of the runtime's built-in mover.
    pub collision: Option<Vec<u8>>,
    /// The room's groups, nodes, pieces, lights and scene parameters.
    pub index: RoomIndex,
    /// The rooms, by index, a host keeps loaded while this one is active,
    /// nearest first.
    pub neighbours: Vec<u32>,
    /// The rooms, by index, whose geometry continues this one's, drawn with
    /// it whole and lit by their own lights while loaded: walking from one
    /// into the other shows no edge.
    pub beside: Vec<u32>,
    /// A sound of the bank looped while the room is active, crossfaded with
    /// the next room's at a switch.
    pub ambience: Option<String>,
}

impl GameRoom {
    /// `room` encoded and indexed, with no collision, neighbour or ambience.
    pub fn of(name: &str, room: &crate::RoomPack) -> GameRoom {
        GameRoom {
            name: name.into(),
            room: Some(crate::encode(room)),
            index: RoomIndex::of(room),
            ..GameRoom::default()
        }
    }
}

/// The HUD: a UI description and the files it names.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Hud {
    /// JSON, `schemas/ui.schema.json`.
    pub description: String,
    /// Fonts and images by the path the description names them by.
    pub files: Vec<(String, Vec<u8>)>,
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
                "game pack version {found} is not supported (reads {} and {GAME_VERSION})",
                previous::VERSION
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
    let game = match found {
        GAME_VERSION => read::<GamePack>(body)?,
        previous::VERSION => read::<previous::GamePack3>(body)?.into(),
        _ => return Err(GameFormatError::Version { found }),
    };
    if game.batch_version != BATCH_VERSION {
        return Err(GameFormatError::BatchVersion {
            found: game.batch_version,
        });
    }
    validate_names(&game.names)?;
    validate_rooms(&game.assets.rooms).map_err(GameFormatError::Invalid)?;
    for cutscene in &game.cutscenes {
        validate_cutscene(cutscene).map_err(|why| {
            GameFormatError::Invalid(format!("cutscene {}: {why}", cutscene.name))
        })?;
    }
    Ok(game)
}

fn read<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, GameFormatError> {
    let (game, rest) = postcard::take_from_bytes::<T>(body).map_err(|e| match e {
        postcard::Error::DeserializeUnexpectedEnd => GameFormatError::Truncated,
        other => GameFormatError::Invalid(other.to_string()),
    })?;
    match rest.len() {
        0 => Ok(game),
        n => Err(GameFormatError::Invalid(format!(
            "{n} unexpected bytes after the game"
        ))),
    }
}

/// Every parameter's light, node and cutaway in range, and each cutaway
/// with one height parameter and a line [`crate::CutLine::check`] accepts.
fn validate_index(index: &RoomIndex) -> Result<(), String> {
    let mut heights = vec![0; index.cutaways.len()];
    for p in &index.params {
        let (what, i, len) = match p.target {
            Target::Light(i, _) => ("light", i, index.lights.len()),
            Target::Hinge(i) => ("node", i, index.nodes.len()),
            Target::CutawayHeight(i) => ("cutaway", i, index.cutaways.len()),
            _ => continue,
        };
        if i as usize >= len {
            return Err(format!("parameter {} names {what} {i} of {len}", p.name));
        }
        if let Target::CutawayHeight(i) = p.target {
            heights[i as usize] += 1;
        }
    }
    for (c, n) in index.cutaways.iter().zip(heights) {
        if n != 1 {
            return Err(format!("cutaway {} has {n} height parameters", c.name));
        }
        c.line
            .check()
            .map_err(|why| format!("cutaway {} {why}", c.name))?;
    }
    Ok(())
}

fn validate_rooms(rooms: &[GameRoom]) -> Result<(), String> {
    let mut ids: Vec<(u32, &str)> = rooms
        .iter()
        .map(|r| (name_id(&r.name), r.name.as_str()))
        .collect();
    ids.sort_unstable();
    if let Some(pair) = ids.windows(2).find(|p| p[0].0 == p[1].0) {
        return Err(format!(
            "rooms {:?} and {:?} have the same id",
            pair[0].1, pair[1].1
        ));
    }
    for (i, room) in rooms.iter().enumerate() {
        validate_index(&room.index).map_err(|why| format!("room {}: {why}", room.name))?;
        if let Some(&n) = (room.neighbours.iter().chain(&room.beside))
            .find(|&&n| n as usize >= rooms.len() || n as usize == i)
        {
            return Err(format!(
                "room {} has neighbour {n} of {} rooms",
                room.name,
                rooms.len()
            ));
        }
        if room.collision.is_some() != rooms[0].collision.is_some() {
            let (with, without) = match room.collision {
                Some(_) => (&room.name, &rooms[0].name),
                None => (&rooms[0].name, &room.name),
            };
            return Err(format!(
                "room {with} has a collision pack and room {without} has none; every room has one or none has"
            ));
        }
    }
    Ok(())
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
        Block::Hinge {
            role: r,
            span,
            to,
            ease,
            ..
        } => {
            role(*r)?;
            seconds("hinge span start", span[0])?;
            seconds("hinge span end", span[1])?;
            if span[1] <= span[0] {
                return Err(format!("hinge span {span:?} ends before it starts"));
            }
            finite("hinge angle", *to)?;
            known("ease", Ease::from_u8(*ease).is_some())
        }
        Block::Path {
            role: r,
            mode,
            blend,
            from,
            hold,
            spans,
            then,
            ..
        } => {
            role(*r)?;
            known("clip mode", ClipMode::from_u8(*mode).is_some())?;
            seconds("path blend", *blend)?;
            seconds("path start", *from)?;
            seconds("path hold", *hold)?;
            let mut last = *from;
            for s in spans {
                seconds("span start", s.from)?;
                seconds("span end", s.until)?;
                if s.from < last || s.until <= s.from {
                    return Err(format!(
                        "span {:?} starts before {last} or ends before it starts",
                        [s.from, s.until]
                    ));
                }
                if !(s.rate.is_finite() && s.rate > 0.0) {
                    return Err(format!("span rate is {}", s.rate));
                }
                for v in s.to.iter().flatten().chain(&s.yaw).chain([&s.arc]) {
                    finite("span number", *v)?;
                }
                known("ease", Ease::from_u8(s.ease).is_some())?;
                last = s.until;
            }
            for t in then {
                known("clip mode", ClipMode::from_u8(t.mode).is_some())?;
                seconds("blend", t.blend)?;
                seconds("after", t.after)?;
                positive("speed", t.speed)?;
            }
            Ok(())
        }
        Block::Scene {
            values,
            duration,
            ease,
        } => {
            values
                .iter()
                .try_for_each(|(_, v)| finite("scene value", *v))?;
            seconds("scene duration", *duration)?;
            known("ease", Ease::from_u8(*ease).is_some())
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
