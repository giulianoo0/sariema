// SPDX-License-Identifier: Apache-2.0

#![allow(dead_code)]

use std::f32::consts::TAU;
use std::sync::Arc;

use sariema_audio::{Audio, Feeding, Mixer, MixerSettings, SoundDef, WavFormat, gain_to_db, wav};
use sariema_format::sound::{Container, Load, LoopPoints};

pub const RATE: u32 = 48_000;

pub fn sine(freq: f32, amp: f32, rate: u32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|n| amp * (TAU * freq * n as f32 / rate as f32).sin())
        .collect()
}

/// Interleaves a mono signal into identical stereo channels.
pub fn stereo(mono: &[f32]) -> Vec<f32> {
    mono.iter().flat_map(|&s| [s, s]).collect()
}

pub struct Def {
    pub name: &'static str,
    pub bus: usize,
    pub channels: u16,
    pub rate: u32,
    pub samples: Vec<f32>,
    pub loop_points: Option<(u64, u64)>,
    pub stream: bool,
    pub on_demand: bool,
    pub gain: f32,
}

impl Def {
    pub fn mono(name: &'static str, bus: usize, samples: Vec<f32>) -> Def {
        Def {
            name,
            bus,
            channels: 1,
            rate: RATE,
            samples,
            loop_points: None,
            stream: false,
            on_demand: false,
            gain: 1.0,
        }
    }

    pub fn looped(mut self, start: u64, end: u64) -> Def {
        self.loop_points = Some((start, end));
        self
    }

    pub fn streamed(mut self) -> Def {
        self.stream = true;
        self
    }

    pub fn build(self) -> SoundDef {
        let frames = (self.samples.len() / usize::from(self.channels)) as u64;
        SoundDef {
            name: self.name.into(),
            bus: self.bus,
            gain: self.gain,
            container: Container::Wav,
            data: Arc::from(wav(
                &self.samples,
                self.channels,
                self.rate,
                WavFormat::Float32,
            )),
            loop_points: self
                .loop_points
                .map(|(start, end)| LoopPoints { start, end }),
            frames,
            stream: self.stream,
            load: if self.on_demand {
                Load::OnDemand
            } else {
                Load::Bank
            },
            enter_s: None,
            leave_s: None,
        }
    }
}

pub const MUSIC: usize = 0;
pub const VOICE: usize = 1;
pub const EFFECTS: usize = 2;
pub const AMBIENCE: usize = 3;

pub fn quiet_settings() -> MixerSettings {
    MixerSettings {
        ducking: Vec::new(),
        ..MixerSettings::default()
    }
}

pub fn setup(settings: MixerSettings, defs: Vec<Def>) -> (Audio, Mixer) {
    let sounds = defs.into_iter().map(Def::build).collect();
    Audio::new(settings, sounds, Feeding::Manual).expect("valid settings")
}

/// Renders `frames` stereo frames in callbacks of `buffer` frames, pumping
/// streams and collecting garbage between callbacks.
pub fn render(audio: &mut Audio, mixer: &mut Mixer, frames: usize, buffer: usize) -> Vec<f32> {
    let mut out = vec![0.0; 2 * frames];
    let mut pos = 0;
    while pos < frames {
        let n = buffer.min(frames - pos);
        audio.pump().expect("streams decode");
        mixer.render(&mut out[2 * pos..2 * (pos + n)]);
        audio.collect();
        pos += n;
    }
    out
}

pub fn left(stereo: &[f32]) -> Vec<f32> {
    stereo.iter().step_by(2).copied().collect()
}

pub fn rms_db(samples: &[f32]) -> f32 {
    let ms = samples.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / samples.len() as f64;
    gain_to_db(ms.sqrt() as f32)
}

/// Largest difference between consecutive samples of one channel.
pub fn max_jump(channel: &[f32]) -> f32 {
    channel
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}
