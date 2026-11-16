// SPDX-License-Identifier: Apache-2.0

//! RIFF WAVE writing, for offline renders and generated test sounds.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavFormat {
    Pcm16,
    Pcm24,
    Float32,
}

/// Interleaved `samples` as a WAV file. Integer formats clip to [−1, 1]
/// and round to the nearest step.
pub fn wav(samples: &[f32], channels: u16, rate: u32, format: WavFormat) -> Vec<u8> {
    let (tag, bytes): (u16, u16) = match format {
        WavFormat::Pcm16 => (1, 2),
        WavFormat::Pcm24 => (1, 3),
        WavFormat::Float32 => (3, 4),
    };
    let data_len = samples.len() as u32 * u32::from(bytes);
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    let block = channels * bytes;
    out.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&(bytes * 8).to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        match format {
            WavFormat::Float32 => out.extend_from_slice(&s.to_le_bytes()),
            WavFormat::Pcm16 => {
                let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                out.extend_from_slice(&v.to_le_bytes());
            }
            WavFormat::Pcm24 => {
                let v = (s.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                out.extend_from_slice(&v.to_le_bytes()[..3]);
            }
        }
    }
    out
}
