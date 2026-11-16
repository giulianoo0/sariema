// SPDX-License-Identifier: Apache-2.0

//! Wire encoding of the command and event batch.
//!
//! A batch is a run of records: a little-endian `u16` opcode, then the
//! fields its table entry lists, in order, little-endian, with no padding.
//! `f32` fields must be finite; `bool` is one byte, 0 or 1; `Str` and `Bytes`
//! are a `u32` length then that many bytes, `Str` being UTF-8.
//!
//! This file is shared verbatim with the no_std `sariema-sdk`, so it uses
//! only `core`.

/// A world entity: a slot index and the generation that slot had when the
/// handle was made. A handle whose generation is not the slot's current one
/// is stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entity {
    pub index: u32,
    pub generation: u32,
}

impl Entity {
    /// No entity, where a field is optional.
    pub const NONE: Entity = Entity {
        index: u32::MAX,
        generation: u32::MAX,
    };

    pub const fn is_none(self) -> bool {
        self.index == u32::MAX && self.generation == u32::MAX
    }
}

/// FNV-1a over the UTF-8 bytes: how names (flags, cutscenes, roles, clips,
/// sounds, signals) become the `u32` ids the batch carries.
pub const fn name_id(name: &str) -> u32 {
    let bytes = name.as_bytes();
    let mut hash: u32 = 0x811c_9dc5;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(0x0100_0193);
        i += 1;
    }
    hash
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    Truncated,
    UnknownOpcode(u16),
    /// A field held a value its type does not allow; names the type.
    Invalid(&'static str),
}

impl WireError {
    pub const fn code(self) -> &'static str {
        match self {
            WireError::Truncated => "E_BATCH_TRUNCATED",
            WireError::UnknownOpcode(_) => "E_BATCH_UNKNOWN",
            WireError::Invalid(_) => "E_BATCH_INVALID",
        }
    }
}

/// Appends to a `scene_tween`'s `values` the parameter `param`, a
/// [`name_id`], going to `value`.
pub fn put_scene_value(out: &mut impl Sink, param: u32, value: f32) {
    out.put(&param.to_le_bytes());
    out.put(&value.to_le_bytes());
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub const fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, pos: 0 }
    }

    pub const fn position(&self) -> usize {
        self.pos
    }

    pub const fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    pub fn take<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let bytes = self.take_slice(N)?;
        let mut out = [0; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    pub fn take_slice(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError::Truncated)?;
        let bytes = self.bytes.get(self.pos..end).ok_or(WireError::Truncated)?;
        self.pos = end;
        Ok(bytes)
    }
}

pub trait Sink {
    fn put(&mut self, bytes: &[u8]);
}

/// A decoded field, for tools that print messages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    U64(u64),
    I64(i64),
    F32(f32),
    Bool(bool),
    Entity(Entity),
    Vec3([f32; 3]),
    Str(&'a str),
    Bytes(&'a [u8]),
    /// A variant of a table enum, by name.
    Enum(&'static str),
}

/// A field type of the batch table. `Ref` is what a decoded message holds,
/// borrowing from the batch for `Str` and `Bytes`.
pub trait Wire {
    type Ref<'a>: Copy;
    const NAME: &'static str;
    fn read<'a>(r: &mut Reader<'a>) -> Result<Self::Ref<'a>, WireError>;
    fn write(v: Self::Ref<'_>, out: &mut impl Sink);
    fn size(v: Self::Ref<'_>) -> usize;
    /// A valid value derived from `n`, for generated tests.
    fn sample(n: u32) -> Self::Ref<'static>;
    fn value<'a>(v: Self::Ref<'a>) -> Value<'a>;
}

macro_rules! wire_int {
    ($($t:ident => $variant:ident),*) => {$(
        impl Wire for $t {
            type Ref<'a> = $t;
            const NAME: &'static str = stringify!($t);

            fn read<'a>(r: &mut Reader<'a>) -> Result<$t, WireError> {
                Ok($t::from_le_bytes(r.take()?))
            }

            fn write(v: $t, out: &mut impl Sink) {
                out.put(&v.to_le_bytes());
            }

            fn size(_: $t) -> usize {
                size_of::<$t>()
            }

            fn sample(n: u32) -> $t {
                n as $t
            }

            fn value<'a>(v: Self::Ref<'a>) -> Value<'a> {
                Value::$variant(v.into())
            }
        }
    )*};
}

wire_int!(u8 => U64, u16 => U64, u32 => U64, u64 => U64, i32 => I64);

impl Wire for f32 {
    type Ref<'a> = f32;
    const NAME: &'static str = "f32";

    fn read<'a>(r: &mut Reader<'a>) -> Result<f32, WireError> {
        let v = f32::from_le_bytes(r.take()?);
        if v.is_finite() {
            Ok(v)
        } else {
            Err(WireError::Invalid("f32"))
        }
    }

    fn write(v: f32, out: &mut impl Sink) {
        out.put(&v.to_le_bytes());
    }

    fn size(_: f32) -> usize {
        4
    }

    fn sample(n: u32) -> f32 {
        (n % 1000) as f32 * 0.25 - 100.0
    }

    fn value<'a>(v: Self::Ref<'a>) -> Value<'a> {
        Value::F32(v)
    }
}

impl Wire for bool {
    type Ref<'a> = bool;
    const NAME: &'static str = "bool";

    fn read<'a>(r: &mut Reader<'a>) -> Result<bool, WireError> {
        match r.take::<1>()? {
            [0] => Ok(false),
            [1] => Ok(true),
            _ => Err(WireError::Invalid("bool")),
        }
    }

    fn write(v: bool, out: &mut impl Sink) {
        out.put(&[v as u8]);
    }

    fn size(_: bool) -> usize {
        1
    }

    fn sample(n: u32) -> bool {
        n % 2 == 1
    }

    fn value<'a>(v: Self::Ref<'a>) -> Value<'a> {
        Value::Bool(v)
    }
}

impl Wire for Entity {
    type Ref<'a> = Entity;
    const NAME: &'static str = "Entity";

    fn read<'a>(r: &mut Reader<'a>) -> Result<Entity, WireError> {
        Ok(Entity {
            index: u32::read(r)?,
            generation: u32::read(r)?,
        })
    }

    fn write(v: Entity, out: &mut impl Sink) {
        u32::write(v.index, out);
        u32::write(v.generation, out);
    }

    fn size(_: Entity) -> usize {
        8
    }

    fn sample(n: u32) -> Entity {
        Entity {
            index: n % 4096,
            generation: n / 4096,
        }
    }

    fn value<'a>(v: Self::Ref<'a>) -> Value<'a> {
        Value::Entity(v)
    }
}

/// Three `f32`: a point or a direction in metres, or an RGB colour.
pub enum Vec3 {}

impl Wire for Vec3 {
    type Ref<'a> = [f32; 3];
    const NAME: &'static str = "Vec3";

    fn read<'a>(r: &mut Reader<'a>) -> Result<[f32; 3], WireError> {
        Ok([f32::read(r)?, f32::read(r)?, f32::read(r)?])
    }

    fn write(v: [f32; 3], out: &mut impl Sink) {
        v.into_iter().for_each(|c| f32::write(c, out));
    }

    fn size(_: [f32; 3]) -> usize {
        12
    }

    fn sample(n: u32) -> [f32; 3] {
        [f32::sample(n), f32::sample(n + 1), f32::sample(n + 2)]
    }

    fn value<'a>(v: Self::Ref<'a>) -> Value<'a> {
        Value::Vec3(v)
    }
}

/// UTF-8 text.
pub enum Str {}

impl Wire for Str {
    type Ref<'a> = &'a str;
    const NAME: &'static str = "Str";

    fn read<'a>(r: &mut Reader<'a>) -> Result<&'a str, WireError> {
        let len = u32::read(r)? as usize;
        let bytes = r.take_slice(len)?;
        core::str::from_utf8(bytes).map_err(|_| WireError::Invalid("Str"))
    }

    fn write(v: &str, out: &mut impl Sink) {
        u32::write(v.len() as u32, out);
        out.put(v.as_bytes());
    }

    fn size(v: &str) -> usize {
        4 + v.len()
    }

    fn sample(n: u32) -> &'static str {
        const WORDS: [&str; 4] = ["", "olá", "hero", "a longer line of text"];
        WORDS[n as usize % WORDS.len()]
    }

    fn value<'a>(v: Self::Ref<'a>) -> Value<'a> {
        Value::Str(v)
    }
}

/// Opaque bytes.
pub enum Bytes {}

impl Wire for Bytes {
    type Ref<'a> = &'a [u8];
    const NAME: &'static str = "Bytes";

    fn read<'a>(r: &mut Reader<'a>) -> Result<&'a [u8], WireError> {
        let len = u32::read(r)? as usize;
        r.take_slice(len)
    }

    fn write(v: &[u8], out: &mut impl Sink) {
        u32::write(v.len() as u32, out);
        out.put(v);
    }

    fn size(v: &[u8]) -> usize {
        4 + v.len()
    }

    fn sample(n: u32) -> &'static [u8] {
        const BLOBS: [&[u8]; 3] = [&[], &[0, 255, 7], b"{\"k\":1}"];
        BLOBS[n as usize % BLOBS.len()]
    }

    fn value<'a>(v: Self::Ref<'a>) -> Value<'a> {
        Value::Bytes(v)
    }
}

pub struct FieldSpec {
    pub name: &'static str,
    pub ty: &'static str,
    pub doc: &'static str,
}

pub struct MessageSpec {
    pub opcode: u16,
    pub name: &'static str,
    pub doc: &'static str,
    pub fields: &'static [FieldSpec],
}

pub struct VariantSpec {
    pub value: u8,
    pub name: &'static str,
    pub doc: &'static str,
}

pub struct EnumSpec {
    pub name: &'static str,
    pub doc: &'static str,
    pub variants: &'static [VariantSpec],
}

wire_enum! {
    /// What an entity stands for. `asset` in `spawn` is the name id of what
    /// it stands for: a node or a light of the active room, which the entity
    /// then places, shows and hides (it starts where the room has it), an
    /// actor of the pack, or a node of the UI tree; it is unused for the
    /// other kinds.
    Kind {
        Empty = 0 => "empty",
        RoomNode = 1 => "room_node",
        Actor = 2 => "actor",
        Light = 3 => "light",
        Camera = 4 => "camera",
        SoundEmitter = 5 => "sound_emitter",
        UiNode = 6 => "ui_node",
    }
}

wire_enum! {
    /// Easing curve from 0 to 1 over a tween or a blend.
    Ease {
        Linear = 0 => "linear",
        InQuad = 1 => "in_quad",
        OutQuad = 2 => "out_quad",
        InOutQuad = 3 => "in_out_quad",
        InCubic = 4 => "in_cubic",
        OutCubic = 5 => "out_cubic",
        InOutCubic = 6 => "in_out_cubic",
        InQuart = 7 => "in_quart",
        OutQuart = 8 => "out_quart",
        InOutQuart = 9 => "in_out_quart",
        InSine = 10 => "in_sine",
        OutSine = 11 => "out_sine",
        InOutSine = 12 => "in_out_sine",
        InExpo = 13 => "in_expo",
        OutExpo = 14 => "out_expo",
        InOutExpo = 15 => "in_out_expo",
        /// Overshoots below 0 at the start.
        InBack = 16 => "in_back",
        /// Overshoots above 1 before settling.
        OutBack = 17 => "out_back",
        InOutBack = 18 => "in_out_back",
        /// 3t² − 2t³.
        Smoothstep = 19 => "smoothstep",
    }
}

wire_enum! {
    /// A numeric property a tween or spring drives. `var` selects the world
    /// variable for `Var` and is ignored otherwise.
    Property {
        PositionX = 0 => "position_x",
        PositionY = 1 => "position_y",
        PositionZ = 2 => "position_z",
        /// Degrees about +Y; 0 faces −Z (north).
        Yaw = 3 => "yaw",
        /// Degrees about the entity's +X.
        Pitch = 4 => "pitch",
        /// Degrees about the entity's −Z.
        Roll = 5 => "roll",
        /// Uniform scale.
        Scale = 6 => "scale",
        /// A world variable, read by other modules (material parameters, UI, audio).
        Var = 7 => "var",
        /// A scene parameter of the active room named by `var`, such as
        /// `look.exposure`, `light.<name>.intensity` or `hinge.<node>`.
        Scene = 8 => "scene",
    }
}

wire_enum! {
    /// How a room switch is shown.
    RoomTransition {
        /// At once: under a fade, or where both rooms look the same.
        Cut = 0 => "cut",
        /// Both rooms drawn while the lighting blends from the old room's to
        /// the new one's: where the geometry continues and the camera goes on.
        Continuous = 1 => "continuous",
        /// The old room dissolves into the new one.
        Dissolve = 2 => "dissolve",
    }
}

wire_enum! {
    /// How an animation clip plays.
    ClipMode {
        Loop = 0 => "loop",
        Once = 1 => "once",
        /// Plays once and holds the last pose.
        Hold = 2 => "hold",
    }
}

wire_enum! {
    /// Mixer bus.
    Bus {
        Music = 0 => "music",
        Voice = 1 => "voice",
        Effects = 2 => "effects",
        Ambience = 3 => "ambience",
        Ui = 4 => "ui",
    }
}

wire_enum! {
    /// Who moves the hero.
    Controller {
        /// The input's move axis, natively, every step.
        Player = 0 => "player",
        /// The game's commands only.
        Script = 1 => "script",
        /// The cutscene sequencer only.
        Cutscene = 2 => "cutscene",
    }
}

wire_enum! {
    /// How a dialogue line ends.
    Advance {
        /// After its duration.
        Auto = 0 => "auto",
        /// When the advance action is pressed, or `advance_line` is sent.
        Tap = 1 => "tap",
    }
}

wire_enum! {
    /// The point of an entity a camera rig aims at. A bone or socket is
    /// named by id and resolved by the animation module.
    Anchor {
        Origin = 0 => "origin",
        Bone = 1 => "bone",
        Socket = 2 => "socket",
    }
}

wire_enum! {
    /// Screen orientation the game asks the host for.
    Orientation {
        Portrait = 0 => "portrait",
        Landscape = 1 => "landscape",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_id_is_fnv1a() {
        assert_eq!(name_id(""), 0x811c_9dc5);
        assert_eq!(name_id("a"), 0xe40c_292c);
        assert_eq!(name_id("foobar"), 0xbf9c_f968);
    }

    #[test]
    fn reader_reports_truncation_without_moving_past_the_end() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert_eq!(r.take::<2>(), Ok([1, 2]));
        assert_eq!(r.take::<2>(), Err(WireError::Truncated));
        assert_eq!(r.take_slice(usize::MAX), Err(WireError::Truncated));
        assert_eq!(r.position(), 2);
    }
}
