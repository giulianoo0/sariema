// SPDX-License-Identifier: Apache-2.0

//! The Sariema sound bank: every sound a game plays, still compressed, with
//! what the mixer needs to play it and what packaging measured about it.
//!
//! A bank is the 4-byte [`MAGIC`], the [`VERSION`] as a little-endian `u32`,
//! then a postcard body holding a [`SoundBank`].

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 4] = *b"SRMS";
pub const VERSION: u32 = 1;

const HEADER_LEN: usize = MAGIC.len() + size_of::<u32>();

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundBank {
    pub buses: Vec<Bus>,
    pub sounds: Vec<Sound>,
    pub mixes: Vec<Mix>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bus {
    pub name: String,
    pub gain_db: f32,
    /// Integrated loudness every sound of the bus is brought to before its
    /// own `gain_db`; `None` plays sounds at the level they were measured.
    pub target_lufs: Option<f32>,
}

/// How the compressed bytes of a [`Sound`] are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Container {
    Mp3,
    Ogg,
    Wav,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Load {
    /// Decoded when the bank is loaded.
    Bank,
    /// Decoded only on an explicit preload, released on unload.
    OnDemand,
}

/// A loop region in frames of the decoded sound at its own sample rate:
/// playback reaches `end` and continues at `start`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopPoints {
    pub start: u64,
    pub end: u64,
}

/// What packaging measured on the decoded sound.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Loudness {
    /// ITU-R BS.1770 integrated loudness; a sound shorter than one 400 ms
    /// block is measured as a single block over its whole length.
    pub integrated_lufs: f32,
    pub peak_dbfs: f32,
    /// Level difference, in dB, between the end of the loop region and its
    /// start; `None` for sounds that do not loop.
    pub seam_step_db: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sound {
    pub name: String,
    /// Index into [`SoundBank::buses`].
    pub bus: u32,
    pub gain_db: f32,
    pub container: Container,
    pub sample_rate: u32,
    pub channels: u16,
    /// Decoded length, encoder delay and padding excluded.
    pub frames: u64,
    pub loop_points: Option<LoopPoints>,
    /// Decoded while it plays instead of all at once.
    pub stream: bool,
    pub load: Load,
    /// Fade, in seconds, when this sound becomes the music; `None` uses the
    /// mixer's default.
    pub enter_s: Option<f32>,
    /// Fade, in seconds, when this sound stops being the music.
    pub leave_s: Option<f32>,
    pub loudness: Loudness,
    pub data: Vec<u8>,
}

/// Sounds that play together, compared with each other by the loudness checks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mix {
    pub name: String,
    /// Indices into [`SoundBank::sounds`].
    pub sounds: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BankError {
    BadMagic,
    Version { found: u32 },
    Truncated,
    Invalid(String),
}

impl BankError {
    pub fn code(&self) -> &'static str {
        match self {
            BankError::BadMagic => "E_PACK_BAD_MAGIC",
            BankError::Version { .. } => "E_PACK_VERSION",
            BankError::Truncated => "E_PACK_TRUNCATED",
            BankError::Invalid(_) => "E_SCHEMA",
        }
    }
}

impl fmt::Display for BankError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BankError::BadMagic => {
                write!(f, "not a sound bank: the file does not start with SRMS")
            }
            BankError::Version { found } => write!(
                f,
                "sound bank version {found} is not supported (expected {VERSION})"
            ),
            BankError::Truncated => write!(f, "sound bank is truncated"),
            BankError::Invalid(why) => write!(f, "sound bank is invalid: {why}"),
        }
    }
}

impl std::error::Error for BankError {}

/// Writes [`MAGIC`], [`VERSION`] and the postcard body. Does not validate.
pub fn encode(bank: &SoundBank) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    postcard::to_extend(bank, out).expect("postcard cannot fail to serialize a SoundBank")
}

/// Decodes a bank and validates it: names unique and not empty, every bus
/// and sound index in range, loop regions not empty and inside the sound,
/// sample rate and channel count positive, every number finite.
///
/// Never panics; see [`crate::decode`] for why lengths read from the input
/// cannot cause a large allocation.
pub fn decode(bytes: &[u8]) -> Result<SoundBank, BankError> {
    let seen = bytes.len().min(MAGIC.len());
    if bytes[..seen] != MAGIC[..seen] {
        return Err(BankError::BadMagic);
    }
    let Some((header, body)) = bytes.split_first_chunk::<HEADER_LEN>() else {
        return Err(BankError::Truncated);
    };
    let found = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if found != VERSION {
        return Err(BankError::Version { found });
    }
    let (bank, rest) = postcard::take_from_bytes::<SoundBank>(body).map_err(|e| match e {
        postcard::Error::DeserializeUnexpectedEnd => BankError::Truncated,
        other => BankError::Invalid(other.to_string()),
    })?;
    if !rest.is_empty() {
        return Err(BankError::Invalid(format!(
            "{} unexpected bytes after the bank",
            rest.len()
        )));
    }
    validate(&bank)?;
    Ok(bank)
}

fn validate(bank: &SoundBank) -> Result<(), BankError> {
    let invalid = |why: String| Err(BankError::Invalid(why));
    unique("bus", bank.buses.iter().map(|b| b.name.as_str()))?;
    unique("sound", bank.sounds.iter().map(|s| s.name.as_str()))?;
    unique("mix", bank.mixes.iter().map(|m| m.name.as_str()))?;
    for bus in &bank.buses {
        let target_ok = bus.target_lufs.is_none_or(f32::is_finite);
        if !bus.gain_db.is_finite() || !target_ok {
            return invalid(format!(
                "bus {} has a gain or target that is not finite",
                bus.name
            ));
        }
    }
    for sound in &bank.sounds {
        let name = &sound.name;
        if sound.bus as usize >= bank.buses.len() {
            return invalid(format!(
                "sound {name} uses bus {} but there are {} buses",
                sound.bus,
                bank.buses.len()
            ));
        }
        if sound.sample_rate == 0 || sound.channels == 0 {
            return invalid(format!("sound {name} has no sample rate or no channels"));
        }
        if let Some(LoopPoints { start, end }) = sound.loop_points
            && !(start < end && end <= sound.frames)
        {
            return invalid(format!(
                "sound {name} loops over {start}..{end}, which is empty or past its {} frames",
                sound.frames
            ));
        }
        let fades = [sound.enter_s, sound.leave_s];
        let numbers = [
            sound.gain_db,
            sound.loudness.integrated_lufs,
            sound.loudness.peak_dbfs,
        ];
        if !numbers
            .iter()
            .all(|x| x.is_finite() || *x == f32::NEG_INFINITY)
            || !fades.iter().flatten().all(|s| s.is_finite() && *s >= 0.0)
            || !sound.loudness.seam_step_db.is_none_or(f32::is_finite)
        {
            return invalid(format!("sound {name} has a number that is not finite"));
        }
    }
    for mix in &bank.mixes {
        if let Some(&i) = mix
            .sounds
            .iter()
            .find(|&&i| i as usize >= bank.sounds.len())
        {
            return invalid(format!(
                "mix {} uses sound {i} but there are {} sounds",
                mix.name,
                bank.sounds.len()
            ));
        }
    }
    Ok(())
}

fn unique<'a>(what: &str, names: impl Iterator<Item = &'a str>) -> Result<(), BankError> {
    let mut seen = BTreeSet::new();
    for name in names {
        if name.is_empty() {
            return Err(BankError::Invalid(format!("a {what} has an empty name")));
        }
        if !seen.insert(name) {
            return Err(BankError::Invalid(format!("two {what}s are called {name}")));
        }
    }
    Ok(())
}
