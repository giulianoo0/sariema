// SPDX-License-Identifier: Apache-2.0

//! The Sariema mixer.
//!
//! [`Audio`] lives on the simulation thread: it decodes, opens streams and
//! sends commands stamped with the mixer frame they take effect on.
//! [`Mixer`] lives on the audio thread and fills output buffers from those
//! commands without allocating, locking or reading files, so its output is a
//! pure function of the commands and their frames. With
//! [`Feeding::Manual`], streams are decoded by the caller too and the whole
//! pipeline is deterministic, which is what [`offline::render`] uses.
//!
//! Sounds play on buses (music, voice, effects and ambience by default),
//! each with a gain under a master. Music changes crossfade, stingers return
//! to the track they interrupted, and ducking lowers chosen buses while
//! another has a voice playing. Every start, stop, steal, natural end, pause
//! and stream gap is faded.

use std::fmt;

mod bank;
mod check;
mod command;
mod controller;
mod decode;
#[cfg(feature = "device")]
mod device;
mod duck;
mod loudness;
mod mixer;
pub mod offline;
mod package;
mod pcm;
mod ramp;
mod resample;
mod settings;
mod stream;
mod voice;
mod wav;

pub use bank::{effective_lufs, runtime, sound_gain_db};
pub use check::{CheckSettings, Warning, check};
pub use command::VoiceId;
pub use controller::{Audio, Feeding, MusicFades, PlayParams, SoundDef, SoundId};
pub use decode::{Decoded, container_of, decode_all};
#[cfg(feature = "device")]
pub use device::{Output, default_rate};
pub use loudness::{LoudnessMeter, integrated_lufs, seam_step_db};
pub use mixer::{MixStats, Mixer, Tap};
pub use package::{AudioProject, BusDesc, LoopDesc, SoundDesc, build, default_buses};
pub use pcm::Pcm;
pub use ramp::{Curve, db_to_gain, gain_to_db};
pub use settings::{BusSettings, DuckRule, MixerSettings, MusicSettings, Steal, StreamSettings};
pub use stream::{Feeder, Pump};
pub use wav::{WavFormat, wav};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioError {
    Io(String),
    Decode(String),
    UnknownSound(String),
    /// A sound loaded on demand was played before its preload.
    NotLoaded(String),
    /// More commands are waiting than `MixerSettings::commands`.
    QueueFull,
    Settings(String),
    Device(String),
}

impl AudioError {
    pub fn code(&self) -> &'static str {
        match self {
            AudioError::Io(_) => "E_IO",
            AudioError::Decode(_) => "E_AUDIO_DECODE",
            AudioError::UnknownSound(_) => "E_AUDIO_UNKNOWN_SOUND",
            AudioError::NotLoaded(_) => "E_AUDIO_NOT_LOADED",
            AudioError::QueueFull => "E_AUDIO_QUEUE_FULL",
            AudioError::Settings(_) => "E_SCHEMA",
            AudioError::Device(_) => "E_DEVICE",
        }
    }
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::Io(why) => write!(f, "{why}"),
            AudioError::Decode(why) => write!(f, "cannot decode: {why}"),
            AudioError::UnknownSound(name) => write!(f, "unknown sound or voice: {name}"),
            AudioError::NotLoaded(name) => {
                write!(f, "{name} is loaded on demand and was not preloaded")
            }
            AudioError::QueueFull => write!(f, "the mixer's command queue is full"),
            AudioError::Settings(why) => write!(f, "invalid audio settings: {why}"),
            AudioError::Device(why) => write!(f, "audio device: {why}"),
        }
    }
}

impl std::error::Error for AudioError {}
