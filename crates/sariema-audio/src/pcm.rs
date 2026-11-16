// SPDX-License-Identifier: Apache-2.0

//! A sound decoded at the mixer's rate, ready to be played from memory.

use sariema_format::sound::LoopPoints;

use crate::decode::Decoded;
use crate::resample::resample;

#[derive(Debug, PartialEq)]
pub struct Pcm {
    channels: u16,
    samples: Box<[f32]>,
    loop_frames: Option<(usize, usize)>,
}

impl Pcm {
    /// Mono or stereo interleaved samples at the mixer's rate. Panics on
    /// another channel count, on a partial frame or on a loop region that is
    /// empty or past the end.
    pub fn new(channels: u16, samples: Vec<f32>, loop_frames: Option<(usize, usize)>) -> Pcm {
        assert!(matches!(channels, 1 | 2), "{channels} channels");
        assert_eq!(samples.len() % usize::from(channels), 0, "partial frame");
        let frames = samples.len() / usize::from(channels);
        if let Some((start, end)) = loop_frames {
            assert!(
                start < end && end <= frames,
                "loop {start}..{end} of {frames}"
            );
        }
        Pcm {
            channels,
            samples: samples.into_boxed_slice(),
            loop_frames,
        }
    }

    /// Converts `decoded` to `rate`, keeping the first two channels of a
    /// sound that has more.
    pub fn from_decoded(
        decoded: &Decoded,
        rate: u32,
        half_taps: u32,
        loop_points: Option<LoopPoints>,
    ) -> Pcm {
        let source = usize::from(decoded.channels);
        let channels = source.min(2);
        let kept: Vec<f32> = if source == channels {
            decoded.samples.clone()
        } else {
            decoded
                .samples
                .chunks_exact(source)
                .flat_map(|f| [f[0], f[1]])
                .collect()
        };
        let region = loop_points.map(|l| (l.start, l.end));
        let (samples, region) = resample(
            &kept,
            channels,
            decoded.sample_rate,
            rate,
            half_taps,
            region,
        );
        let frames = samples.len() / channels;
        let region = region
            .map(|(s, e)| (s as usize, (e as usize).min(frames)))
            .filter(|(s, e)| s < e);
        Pcm::new(channels as u16, samples, region)
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn loop_frames(&self) -> Option<(usize, usize)> {
        self.loop_frames
    }

    pub fn bytes(&self) -> usize {
        size_of_val(&*self.samples)
    }
}
