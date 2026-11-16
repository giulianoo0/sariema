// SPDX-License-Identifier: Apache-2.0

//! Compressed sound to samples through symphonia, from bytes in memory.
//! Encoder delay and padding are trimmed, so frame 0 is the first audible
//! frame and loop points mean the same as in an editor.

use std::io::Cursor;
use std::sync::Arc;

use sariema_format::sound::Container;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Timestamp;

use crate::AudioError;

/// A whole sound, decoded.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved.
    pub samples: Vec<f32>,
}

impl Decoded {
    pub fn frames(&self) -> u64 {
        (self.samples.len() / usize::from(self.channels)) as u64
    }
}

pub fn container_of(extension: &str) -> Option<Container> {
    match extension.to_ascii_lowercase().as_str() {
        "mp3" => Some(Container::Mp3),
        "ogg" | "oga" => Some(Container::Ogg),
        "wav" | "wave" => Some(Container::Wav),
        _ => None,
    }
}

/// Decodes all of `bytes`.
pub fn decode_all(bytes: Arc<[u8]>, container: Container) -> Result<Decoded, AudioError> {
    let mut source = Source::open(bytes, container)?;
    let mut samples = Vec::new();
    while let Some((ts, channels, chunk)) = source.next()? {
        let at = (samples.len() / channels) as i64;
        let rest = placed(at, ts, chunk, channels, &mut samples);
        samples.extend_from_slice(rest);
    }
    Ok(Decoded {
        sample_rate: source.rate,
        channels: source.channels as u16,
        samples,
    })
}

/// A decoded packet: the frame index of its first frame, its channel count
/// and its interleaved samples.
pub(crate) type Packet<'a> = (i64, usize, &'a [f32]);

pub(crate) struct Source {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    pub rate: u32,
    pub channels: usize,
    samples: Vec<f32>,
}

impl Source {
    pub fn open(bytes: Arc<[u8]>, container: Container) -> Result<Source, AudioError> {
        let failed = |e: Error| AudioError::Decode(e.to_string());
        let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
        let mut hint = Hint::new();
        hint.with_extension(match container {
            Container::Mp3 => "mp3",
            Container::Ogg => "ogg",
            Container::Wav => "wav",
        });
        let reader = symphonia::default::get_probe()
            .probe(
                &hint,
                mss,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(failed)?;
        let track = reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| AudioError::Decode("no audio track".into()))?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| AudioError::Decode("the audio track has no codec parameters".into()))?;
        let (rate, channels) = (
            params.sample_rate,
            params.channels.as_ref().map(|c| c.count()),
        );
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(failed)?;
        let id = track.id;
        let mut source = Source {
            reader,
            decoder,
            track: id,
            rate: rate.unwrap_or(0),
            channels: channels.unwrap_or(0),
            samples: Vec::new(),
        };
        // Some containers only tell once a packet is decoded.
        if (source.rate == 0 || source.channels == 0) && source.next()?.is_some() {
            source.seek(0)?;
        }
        if source.rate == 0 || source.channels == 0 {
            return Err(AudioError::Decode(
                "the sample rate or channel count is unknown".into(),
            ));
        }
        Ok(source)
    }

    /// `None` at the end.
    pub fn next(&mut self) -> Result<Option<Packet<'_>>, AudioError> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                Err(Error::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => return Err(AudioError::Decode(e.to_string())),
            };
            if packet.track_id != self.track {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(buf) => {
                    if buf.frames() == 0 {
                        continue;
                    }
                    self.rate = buf.spec().rate();
                    self.channels = buf.spec().channels().count();
                    buf.copy_to_vec_interleaved(&mut self.samples);
                    let ts = packet.pts.get() + packet.trim_start.get() as i64;
                    return Ok(Some((ts, self.channels, &self.samples)));
                }
                // A damaged packet is skipped, like players do.
                Err(Error::DecodeError(_)) => continue,
                Err(e) => return Err(AudioError::Decode(e.to_string())),
            }
        }
    }

    /// Positions the reader at or before `frame`; the next packets report
    /// where they really start.
    pub fn seek(&mut self, frame: u64) -> Result<(), AudioError> {
        let ts = Timestamp::new(frame as i64);
        self.reader
            .seek(
                SeekMode::Accurate,
                SeekTo::Timestamp {
                    ts,
                    track_id: self.track,
                },
            )
            .map_err(|e| AudioError::Decode(format!("seeking to frame {frame}: {e}")))?;
        self.decoder.reset();
        Ok(())
    }
}

/// The part of `chunk`, starting at frame `ts`, that follows frame `at`:
/// frames before `at` are dropped and a gap up to `ts` is filled with
/// silence in `out`.
pub(crate) fn placed<'a>(
    at: i64,
    ts: i64,
    chunk: &'a [f32],
    channels: usize,
    out: &mut Vec<f32>,
) -> &'a [f32] {
    if ts > at {
        out.resize(out.len() + (ts - at) as usize * channels, 0.0);
    }
    let skip = ((at - ts).max(0) as usize * channels).min(chunk.len());
    &chunk[skip..]
}
