// SPDX-License-Identifier: Apache-2.0

//! Streamed sounds: a [`Feeder`] decodes, loops and resamples into a ring
//! that the mixer reads through a [`StreamReader`]. The compressed bytes are
//! the only full copy of the sound in memory.
//!
//! The feeder drops its end of the ring only after the mixer abandoned its
//! own, so the ring is never freed on the audio thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};
use sariema_format::sound::{Container, LoopPoints};

use crate::AudioError;
use crate::decode::{Source, placed};
use crate::resample::StreamResampler;
use crate::settings::{StreamSettings, seconds_to_frames};

#[derive(Debug, Default)]
pub(crate) struct StreamShared {
    /// Every frame the stream will ever have is in the ring.
    finished: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pump {
    /// The ring is full; call again once the mixer has read some.
    Full,
    /// The sound ended and all of it is in the ring.
    Finished,
    /// The mixer let go of the stream; drop the feeder.
    Abandoned,
}

/// The decoding side of one streamed sound.
pub struct Feeder {
    source: Source,
    loop_points: Option<(u64, u64)>,
    preroll: u64,
    /// Source frame the next decoded frame must be.
    pos: i64,
    resampler: Option<StreamResampler>,
    stereo: Vec<f32>,
    staged: Vec<f32>,
    ready: Vec<f32>,
    ready_at: usize,
    producer: Producer<f32>,
    shared: Arc<StreamShared>,
    done: bool,
}

pub(crate) struct StreamReader {
    consumer: Consumer<f32>,
    shared: Arc<StreamShared>,
    capacity_frames: usize,
    /// Of the source: a mono sound arrives as two equal channels but is
    /// panned as mono.
    pub channels: u16,
}

impl Feeder {
    pub(crate) fn open(
        bytes: Arc<[u8]>,
        container: Container,
        loop_points: Option<LoopPoints>,
        rate: u32,
        settings: &StreamSettings,
    ) -> Result<(Feeder, StreamReader), AudioError> {
        let source = Source::open(bytes, container)?;
        let channels = source.channels.min(2) as u16;
        let resampler = (source.rate != rate)
            .then(|| StreamResampler::new(settings.resampler_half_taps, source.rate, rate));
        let capacity_frames = seconds_to_frames(settings.buffer_s, rate).max(1) as usize;
        let (producer, consumer) = RingBuffer::new(2 * capacity_frames);
        let shared = Arc::new(StreamShared::default());
        let feeder = Feeder {
            source,
            loop_points: loop_points.map(|l| (l.start, l.end)),
            preroll: u64::from(settings.seek_preroll_frames),
            pos: 0,
            resampler,
            stereo: Vec::new(),
            staged: Vec::new(),
            ready: Vec::new(),
            ready_at: 0,
            producer,
            shared: shared.clone(),
            done: false,
        };
        let reader = StreamReader {
            consumer,
            shared,
            capacity_frames,
            channels,
        };
        Ok((feeder, reader))
    }

    /// Decodes until the ring is full or the sound ended. A decoding error
    /// ends the sound where it happened and is returned.
    pub fn pump(&mut self) -> Result<Pump, AudioError> {
        loop {
            if self.producer.is_abandoned() {
                return Ok(Pump::Abandoned);
            }
            let pending = &self.ready[self.ready_at..];
            let room = (self.producer.slots() / 2) * 2;
            let (pushed, _) = self
                .producer
                .push_partial_slice(&pending[..room.min(pending.len())]);
            self.ready_at += pushed.len();
            if self.ready_at < self.ready.len() {
                return Ok(Pump::Full);
            }
            self.ready.clear();
            self.ready_at = 0;
            if self.done {
                self.shared.finished.store(true, Ordering::Release);
                return Ok(Pump::Finished);
            }
            if let Err(e) = self.decode_more() {
                self.done = true;
                self.shared.finished.store(true, Ordering::Release);
                return Err(e);
            }
        }
    }

    fn decode_more(&mut self) -> Result<(), AudioError> {
        let decoded = self.source.next()?;
        let Some((ts, channels, chunk)) = decoded else {
            match self.loop_points {
                Some((start, _)) => self.restart_loop(start)?,
                None => {
                    if let Some(r) = &mut self.resampler {
                        r.finish(&mut self.ready);
                    }
                    self.done = true;
                }
            }
            return Ok(());
        };
        self.stereo.clear();
        self.stereo
            .extend(chunk.chunks_exact(channels).flat_map(|f| match f {
                [mono] => [*mono, *mono],
                [l, r, ..] => [*l, *r],
                [] => unreachable!("chunks_exact never yields an empty chunk"),
            }));
        self.staged.clear();
        let rest = placed(self.pos, ts, &self.stereo, 2, &mut self.staged);
        self.pos += (self.staged.len() / 2) as i64;
        let mut take = rest.len() / 2;
        let mut wrap = None;
        if let Some((start, end)) = self.loop_points {
            let left = (end as i64 - self.pos).max(0) as usize;
            if take >= left {
                take = left;
                wrap = Some(start);
            }
        }
        self.staged.extend_from_slice(&rest[..2 * take]);
        self.pos += take as i64;
        match &mut self.resampler {
            Some(r) => r.push(&self.staged, &mut self.ready),
            None => self.ready.extend_from_slice(&self.staged),
        }
        if let Some(start) = wrap {
            self.restart_loop(start)?;
        }
        Ok(())
    }

    fn restart_loop(&mut self, start: u64) -> Result<(), AudioError> {
        self.source.seek(start.saturating_sub(self.preroll))?;
        self.pos = start as i64;
        Ok(())
    }

    /// Bytes this stream holds besides the compressed sound: the ring and the
    /// decoding buffers.
    pub fn buffer_bytes(&self) -> usize {
        let floats = self.producer.buffer().capacity()
            + self.stereo.capacity()
            + self.staged.capacity()
            + self.ready.capacity();
        floats * size_of::<f32>()
    }
}

impl StreamReader {
    /// Frames ready to read and whether no more will come; read in that
    /// order so a finished stream is never seen with frames still missing.
    pub fn state(&self) -> (usize, bool) {
        let finished = self.shared.finished.load(Ordering::Acquire);
        (self.consumer.slots() / 2, finished)
    }

    pub fn capacity_frames(&self) -> usize {
        self.capacity_frames
    }

    /// Reads exactly `frames` stereo frames, which must be available, calling
    /// `f` with each frame's index and samples.
    pub fn read(&mut self, frames: usize, mut f: impl FnMut(usize, f32, f32)) {
        let chunk = self
            .consumer
            .read_chunk(2 * frames)
            .expect("the caller checked the frames are available");
        let (a, b) = chunk.as_slices();
        let mut i = 0;
        for half in [a, b] {
            for frame in half.as_chunks::<2>().0.iter() {
                f(i, frame[0], frame[1]);
                i += 1;
            }
        }
        chunk.commit_all();
    }
}
