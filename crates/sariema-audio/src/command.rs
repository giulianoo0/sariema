// SPDX-License-Identifier: Apache-2.0

//! What the control side sends the mixer. Every command carries the mixer
//! frame it takes effect on.

use std::sync::Arc;

use crate::pcm::Pcm;
use crate::ramp::Curve;
use crate::stream::StreamReader;

/// A playing sound, as the control side knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoiceId(pub u64);

pub(crate) enum Src {
    Static(Arc<Pcm>),
    Stream(StreamReader),
}

pub(crate) struct Play {
    pub voice: VoiceId,
    pub sound: u32,
    pub src: Src,
    pub bus: u16,
    pub gain: f32,
    pub pan: f32,
    pub looped: bool,
    pub fade_in: u32,
    pub curve: Curve,
    pub priority: u8,
}

pub(crate) enum Op {
    Play(Play),
    Stop {
        voice: VoiceId,
        fade: u32,
    },
    SetVoice {
        voice: VoiceId,
        gain: f32,
        pan: f32,
        ramp: u32,
    },
    SetBus {
        bus: u16,
        gain: f32,
        ramp: u32,
    },
    SetMaster {
        gain: f32,
        ramp: u32,
    },
    /// `play.fade_in` brings the track in; the previous one leaves over `fade_out`.
    Music {
        play: Play,
        fade_out: u32,
    },
    Stinger(Play),
    MusicStop {
        fade: u32,
    },
    Pause,
    Resume,
}

pub(crate) struct Timed {
    pub at: u64,
    pub op: Op,
}
