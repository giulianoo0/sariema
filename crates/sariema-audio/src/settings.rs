// SPDX-License-Identifier: Apache-2.0

//! Everything that bounds or tunes the mixer.

use crate::ramp::Curve;

#[derive(Clone, Debug, PartialEq)]
pub struct MixerSettings {
    /// Output frames per second. Default 48 000.
    pub sample_rate: u32,
    /// Frames mixed at a time, whatever the device asks for. Commands land on
    /// their exact frame; ducking follows the key bus once per block. Adds at
    /// most this much latency. Default 64.
    pub block_frames: u32,
    /// Default music, voice, effects and ambience, all at 0 dB.
    pub buses: Vec<BusSettings>,
    /// Sounds audible at once; a play beyond it steals. Default 32.
    pub voices: u32,
    /// Extra voices that may still be fading out after a stop or a steal.
    /// A play finding none free is refused. Default 16.
    pub fading_voices: u32,
    /// Default [`Steal::Oldest`].
    pub steal: Steal,
    /// Shortest fade, in seconds, applied to every start, stop, steal, natural
    /// end, pause and stream gap. Default 0.003.
    pub declick_s: f32,
    /// Default: music and ambience drop under voice.
    pub ducking: Vec<DuckRule>,
    pub music: MusicSettings,
    pub stream: StreamSettings,
    /// Commands that may wait in the queue or for their time. Default 1024.
    pub commands: u32,
}

impl Default for MixerSettings {
    fn default() -> MixerSettings {
        MixerSettings {
            sample_rate: 48_000,
            block_frames: 64,
            buses: ["music", "voice", "effects", "ambience"]
                .into_iter()
                .map(BusSettings::named)
                .collect(),
            voices: 32,
            fading_voices: 16,
            steal: Steal::Oldest,
            declick_s: 0.003,
            ducking: vec![DuckRule::default()],
            music: MusicSettings::default(),
            stream: StreamSettings::default(),
            commands: 1024,
        }
    }
}

impl MixerSettings {
    pub fn bus(&self, name: &str) -> Option<usize> {
        self.buses.iter().position(|b| b.name == name)
    }

    pub(crate) fn declick_frames(&self) -> u32 {
        seconds_to_frames(self.declick_s, self.sample_rate).max(1)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BusSettings {
    pub name: String,
    /// Default 0.
    pub gain_db: f32,
}

impl BusSettings {
    pub fn named(name: &str) -> BusSettings {
        BusSettings {
            name: name.into(),
            gain_db: 0.0,
        }
    }
}

/// Which voice a play takes over when every voice is busy. Only voices of a
/// priority at most the new play's are candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Steal {
    Oldest,
    /// The one whose gain, fades included, is lowest.
    Quietest,
    /// Refuse the new play instead.
    Refuse,
}

/// While any voice plays on `key`, the `targets` drop by `depth_db`.
#[derive(Clone, Debug, PartialEq)]
pub struct DuckRule {
    /// Default "voice".
    pub key: String,
    /// Default "music" and "ambience".
    pub targets: Vec<String>,
    /// Negative. Default −9, the game's former 0.35 gain.
    pub depth_db: f32,
    /// Seconds from no duck to the full depth. Default 0.15.
    pub attack_s: f32,
    /// Seconds the duck stays after the key goes quiet, so lines close
    /// together do not pump. Default 0.25.
    pub hold_s: f32,
    /// Seconds from the full depth back to no duck. Default 0.6.
    pub release_s: f32,
}

impl Default for DuckRule {
    fn default() -> DuckRule {
        DuckRule {
            key: "voice".into(),
            targets: vec!["music".into(), "ambience".into()],
            depth_db: -9.0,
            attack_s: 0.15,
            hold_s: 0.25,
            release_s: 0.6,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MusicSettings {
    /// Bus music tracks and stingers play on. Default "music".
    pub bus: String,
    /// Seconds a new track takes to come in when neither the command nor the
    /// sound gives one. Default 1.2.
    pub fade_in_s: f32,
    /// Seconds the previous track takes to go. Default 1.2.
    pub fade_out_s: f32,
    /// Default [`Curve::EqualPower`], which keeps the summed power of two
    /// unrelated tracks constant.
    pub curve: Curve,
    /// Seconds the track takes to drop when a stinger starts. Default 0.13.
    pub stinger_out_s: f32,
    /// Seconds the held track takes to return after the stinger. Default 1.0.
    pub stinger_return_s: f32,
    /// Tracks that may still be fading out at once. Beyond it the quietest
    /// is cut, and counted. Default 2.
    pub fading_tracks: u32,
}

impl Default for MusicSettings {
    fn default() -> MusicSettings {
        MusicSettings {
            bus: "music".into(),
            fade_in_s: 1.2,
            fade_out_s: 1.2,
            curve: Curve::EqualPower,
            stinger_out_s: 0.13,
            stinger_return_s: 1.0,
            fading_tracks: 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamSettings {
    /// Seconds of decoded audio buffered ahead of each streamed track.
    /// Default 1.0.
    pub buffer_s: f32,
    /// Fraction of the buffer that must be filled again before a starved
    /// stream fades back in. Default 0.5.
    pub resume_fill: f32,
    /// Source frames decoded and dropped before a loop start after seeking
    /// back to it, so codecs with overlapping blocks are settled. Default 4096.
    pub seek_preroll_frames: u32,
    /// Sleep, in milliseconds, of the decoding thread between refills.
    /// Default 5.
    pub poll_ms: u32,
    /// Taps on each side of the resampling filter. Default 16.
    pub resampler_half_taps: u32,
}

impl Default for StreamSettings {
    fn default() -> StreamSettings {
        StreamSettings {
            buffer_s: 1.0,
            resume_fill: 0.5,
            seek_preroll_frames: 4096,
            poll_ms: 5,
            resampler_half_taps: 16,
        }
    }
}

pub(crate) fn seconds_to_frames(s: f32, rate: u32) -> u32 {
    (f64::from(s.max(0.0)) * f64::from(rate)).round() as u32
}
