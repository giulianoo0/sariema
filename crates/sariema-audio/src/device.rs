// SPDX-License-Identifier: Apache-2.0

//! The platform's default output device through cpal (feature `device`).
//!
//! The host app owns what cpal cannot: on iOS the `AVAudioSession` category
//! and interruptions, on Android initialising `ndk-context` with the JVM
//! and activity before [`Output::start`].

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::AudioError;
use crate::mixer::Mixer;

/// Frames rendered per piece when the device asks for more channels than two.
const SCRATCH_FRAMES: usize = 4096;

pub struct Output {
    _stream: cpal::Stream,
    pub sample_rate: u32,
    pub channels: u16,
}

/// The default device's own rate; build the mixer at it.
pub fn default_rate() -> Result<u32, AudioError> {
    let device = default_device()?;
    let config = device
        .default_output_config()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    Ok(config.sample_rate())
}

fn default_device() -> Result<cpal::Device, AudioError> {
    cpal::default_host()
        .default_output_device()
        .ok_or_else(|| AudioError::Device("no output device".into()))
}

impl Output {
    /// Starts the default device at `sample_rate`, calling the mixer from its
    /// callback. Channels past the first two are silent.
    pub fn start(mut mixer: Mixer, sample_rate: u32) -> Result<Output, AudioError> {
        let failed = |e: &dyn std::fmt::Display| AudioError::Device(e.to_string());
        let device = default_device()?;
        let channels = device
            .default_output_config()
            .map_err(|e| failed(&e))?
            .channels()
            .max(2);
        let config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let mut scratch = vec![0.0f32; 2 * SCRATCH_FRAMES];
        let callback = move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
            if channels == 2 {
                mixer.render(out);
                return;
            }
            let c = usize::from(channels);
            for piece in out.chunks_mut(c * SCRATCH_FRAMES) {
                let frames = piece.len() / c;
                let stereo = &mut scratch[..2 * frames];
                mixer.render(stereo);
                for (frame, lr) in piece
                    .chunks_exact_mut(c)
                    .zip(stereo.as_chunks::<2>().0.iter())
                {
                    frame.fill(0.0);
                    frame[..2].copy_from_slice(lr);
                }
            }
        };
        let stream = device
            .build_output_stream(config, callback, |e| log_error(&e), None)
            .map_err(|e| failed(&e))?;
        stream.play().map_err(|e| failed(&e))?;
        Ok(Output {
            _stream: stream,
            sample_rate,
            channels,
        })
    }
}

fn log_error(e: &cpal::Error) {
    eprintln!("sariema-audio: output device error: {e}");
}
