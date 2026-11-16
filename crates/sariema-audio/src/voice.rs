// SPDX-License-Identifier: Apache-2.0

//! One playing sound: its source, gain, pan and fade, rendered into its
//! bus a span of frames at a time.

use std::f32::consts::{FRAC_PI_4, SQRT_2};

use crate::command::{Play, Src, VoiceId};
use crate::ramp::{Curve, Ramp};
use crate::stream::StreamReader;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flow {
    /// The stream has not buffered enough to start.
    Waiting,
    Playing,
    /// The stream is running dry: `left` frames remain, the last
    /// `fade_len` of them fading out.
    Draining {
        left: usize,
        fade_len: usize,
    },
    /// The stream ran dry and faded out; it fades back in once refilled.
    Starved,
}

pub(crate) struct Voice {
    pub id: VoiceId,
    pub sound: u32,
    pub bus: u16,
    src: Option<Src>,
    pos: usize,
    looped: bool,
    gain: Ramp,
    pan: Ramp,
    fade: Ramp,
    fade_in: u32,
    curve: Curve,
    flow: Flow,
    gap: Ramp,
    pub stopping: bool,
    /// A held music track: silent and not advancing.
    pub frozen: bool,
    /// Music: never stolen and not counted against the voice limit.
    pub protected: bool,
    pub started: u64,
    pub priority: u8,
}

pub(crate) struct Counters {
    pub underruns: u64,
}

/// Frames a starved stream needs buffered before it starts or fades back in.
pub(crate) struct StreamGate {
    pub declick: u32,
    pub resume_fill: f32,
}

impl Voice {
    pub fn empty() -> Voice {
        Voice {
            id: VoiceId(0),
            sound: 0,
            bus: 0,
            src: None,
            pos: 0,
            looped: false,
            gain: Ramp::constant(0.0),
            pan: Ramp::constant(0.0),
            fade: Ramp::constant(0.0),
            fade_in: 0,
            curve: Curve::Linear,
            flow: Flow::Playing,
            gap: Ramp::constant(1.0),
            stopping: false,
            frozen: false,
            protected: false,
            started: 0,
            priority: 0,
        }
    }

    pub fn is_free(&self) -> bool {
        self.src.is_none()
    }

    pub fn start(&mut self, t: u64, play: Play, declick: u32, protected: bool) {
        let fade_in = play.fade_in.max(declick);
        let flow = match play.src {
            Src::Static(_) => Flow::Playing,
            Src::Stream(_) => Flow::Waiting,
        };
        let mut fade = Ramp::constant(0.0);
        fade.retarget(t, 1.0, fade_in, play.curve);
        *self = Voice {
            id: play.voice,
            sound: play.sound,
            bus: play.bus,
            src: Some(play.src),
            pos: 0,
            looped: play.looped,
            gain: Ramp::constant(play.gain),
            pan: Ramp::constant(play.pan.clamp(-1.0, 1.0)),
            fade,
            fade_in,
            curve: play.curve,
            flow,
            gap: Ramp::constant(1.0),
            stopping: false,
            frozen: false,
            protected,
            started: t,
            priority: play.priority,
        };
    }

    /// Takes the source out, leaving the voice free.
    pub fn free(&mut self) -> Option<Src> {
        self.id = VoiceId(0);
        self.src.take()
    }

    pub fn stop(&mut self, t: u64, len: u32, curve: Curve) {
        self.stopping = true;
        self.frozen = false;
        self.fade.retarget(t, 0.0, len, curve);
    }

    /// Fades to silence and then stops advancing until [`Voice::unhold`].
    pub fn hold(&mut self, t: u64, len: u32, curve: Curve) {
        self.fade.retarget(t, 0.0, len, curve);
    }

    pub fn unhold(&mut self, t: u64, len: u32, curve: Curve) {
        self.frozen = false;
        self.fade.retarget(t, 1.0, len, curve);
    }

    pub fn set(&mut self, t: u64, gain: f32, pan: f32, ramp: u32) {
        self.gain.retarget(t, gain, ramp, Curve::Linear);
        self.pan
            .retarget(t, pan.clamp(-1.0, 1.0), ramp, Curve::Linear);
    }

    pub fn level(&self, t: u64) -> f32 {
        self.gain.value(t) * self.fade.value(t)
    }

    /// Frame at which a fade to silence ends, when one is under way.
    pub fn silent_at(&self) -> Option<u64> {
        (self.fade.target() == 0.0 && !self.frozen).then(|| self.fade.end())
    }

    /// Adds `n` frames from frame `t0` into the stereo `out`. Returns false
    /// once the voice has finished and can be freed.
    pub fn render(
        &mut self,
        t0: u64,
        n: usize,
        gate: &StreamGate,
        out: &mut [f32],
        counters: &mut Counters,
    ) -> bool {
        if self.frozen {
            return true;
        }
        let mut limit = n;
        if self.fade.target() == 0.0 {
            let left = self.fade.end().saturating_sub(t0) as usize;
            if left == 0 {
                if self.stopping {
                    return false;
                }
                self.frozen = true;
                return true;
            }
            limit = limit.min(left);
        }
        let alive = match self.src.take() {
            Some(Src::Static(pcm)) => {
                let alive = self.render_static(&pcm, t0, limit, gate.declick, out);
                self.src = Some(Src::Static(pcm));
                alive
            }
            Some(Src::Stream(mut reader)) => {
                let alive = self.render_stream(&mut reader, t0, limit, gate, out, counters);
                self.src = Some(Src::Stream(reader));
                alive
            }
            None => false,
        };
        alive && !(self.stopping && limit < n)
    }

    fn gains_at(&self, t: u64, channels: u16) -> (f32, f32) {
        let g = self.gain.value(t) * self.fade.value(t);
        let (l, r) = pan_gains(self.pan.value(t), channels);
        (g * l, g * r)
    }

    /// Pan gains over `[t0, t0 + n)`, interpolated linearly from the exact
    /// law at both ends, which spares a sine and cosine per frame while the
    /// pan moves.
    fn pan_span(&self, t0: u64, n: usize, channels: u16) -> PanSpan {
        let (l0, r0) = pan_gains(self.pan.value(t0), channels);
        let (l1, r1) = pan_gains(self.pan.value(t0 + n as u64), channels);
        let step = 1.0 / n.max(1) as f32;
        PanSpan {
            l0,
            r0,
            dl: (l1 - l0) * step,
            dr: (r1 - r0) * step,
        }
    }

    fn gains_in(&self, span: &PanSpan, t0: u64, k: usize) -> (f32, f32) {
        let g = self.gain.value(t0 + k as u64) * self.fade.value(t0 + k as u64);
        let x = k as f32;
        (g * (span.l0 + span.dl * x), g * (span.r0 + span.dr * x))
    }

    fn render_static(
        &mut self,
        pcm: &crate::pcm::Pcm,
        t0: u64,
        n: usize,
        declick: u32,
        out: &mut [f32],
    ) -> bool {
        let channels = pcm.channels();
        let frames = pcm.frames();
        let samples = pcm.samples();
        let (start, end) = if self.looped {
            pcm.loop_frames().unwrap_or((0, frames))
        } else {
            (0, frames)
        };
        let tail = if self.looped {
            usize::MAX
        } else {
            frames.saturating_sub(declick as usize)
        };
        let flat = self.gain.is_flat(t0, n as u32)
            && self.pan.is_flat(t0, n as u32)
            && self.fade.is_flat(t0, n as u32);
        let pan = self.pan_span(t0, n, channels);
        let mut i = 0;
        while i < n {
            if self.pos >= end {
                if !self.looped {
                    return false;
                }
                self.pos = start;
            }
            let seg = (n - i).min(end - self.pos);
            let span = &mut out[2 * i..2 * (i + seg)];
            if flat && self.pos + seg <= tail {
                let (gl, gr) = self.gains_at(t0 + i as u64, channels);
                add_constant(samples, channels, self.pos, gl, gr, span);
            } else {
                for (k, frame) in span.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                    let at = self.pos + k;
                    let (mut gl, mut gr) = self.gains_in(&pan, t0, i + k);
                    if at >= tail {
                        let fade = (frames - at) as f32 / declick as f32;
                        gl *= fade.min(1.0);
                        gr *= fade.min(1.0);
                    }
                    let (l, r) = read_frame(samples, channels, at);
                    frame[0] += l * gl;
                    frame[1] += r * gr;
                }
            }
            self.pos += seg;
            i += seg;
        }
        self.looped || self.pos < frames
    }

    fn render_stream(
        &mut self,
        reader: &mut StreamReader,
        t0: u64,
        n: usize,
        gate: &StreamGate,
        out: &mut [f32],
        counters: &mut Counters,
    ) -> bool {
        let (avail, finished) = reader.state();
        let declick = gate.declick as usize;
        let refill = ((gate.resume_fill * reader.capacity_frames() as f32) as usize)
            .clamp(declick, reader.capacity_frames());
        match self.flow {
            Flow::Playing | Flow::Draining { .. } => {}
            Flow::Waiting | Flow::Starved if avail >= refill || (finished && avail > 0) => {
                if self.flow == Flow::Waiting {
                    let target = self.fade.target();
                    self.fade = Ramp::constant(0.0);
                    self.fade.retarget(t0, target, self.fade_in, self.curve);
                } else {
                    self.gap = Ramp::constant(0.0);
                    self.gap.retarget(t0, 1.0, gate.declick, Curve::Linear);
                }
                self.flow = Flow::Playing;
            }
            _ => return !finished,
        }
        // Frames left before the data runs out, and the fade that leads there.
        // A drain, once started, ends in silence even if data arrives meanwhile.
        let (read, left, fade_len, next) = match self.flow {
            Flow::Draining { left, fade_len } => {
                let next = if left <= n {
                    Flow::Starved
                } else {
                    Flow::Draining {
                        left: left - n,
                        fade_len,
                    }
                };
                (left.min(n), left, fade_len, next)
            }
            _ if finished => (avail.min(n), avail, declick, Flow::Playing),
            _ if avail >= n + declick => (n, usize::MAX, declick, Flow::Playing),
            _ => {
                let fade_len = declick.min(avail);
                let next = if avail <= n {
                    Flow::Starved
                } else {
                    Flow::Draining {
                        left: avail - n,
                        fade_len,
                    }
                };
                (avail.min(n), avail, fade_len, next)
            }
        };
        let ends = finished && avail <= n && !matches!(self.flow, Flow::Draining { .. });
        if next == Flow::Starved {
            counters.underruns += 1;
        }
        self.flow = next;
        let fade_len = fade_len.max(1);
        let channels = reader.channels;
        let flat = self.gain.is_flat(t0, n as u32)
            && self.pan.is_flat(t0, n as u32)
            && self.fade.is_flat(t0, n as u32)
            && self.gap.is_flat(t0, n as u32);
        let (cl, cr) = self.gains_at(t0, channels);
        let gap0 = self.gap.value(t0);
        let pan = self.pan_span(t0, n, channels);
        reader.read(read, |k, l, r| {
            let (mut gl, mut gr) = if flat {
                (cl * gap0, cr * gap0)
            } else {
                let (gl, gr) = self.gains_in(&pan, t0, k);
                let gap = self.gap.value(t0 + k as u64);
                (gl * gap, gr * gap)
            };
            let remaining = left - k;
            if remaining < fade_len {
                let f = remaining as f32 / fade_len as f32;
                gl *= f;
                gr *= f;
            }
            out[2 * k] += l * gl;
            out[2 * k + 1] += r * gr;
        });
        !ends
    }
}

struct PanSpan {
    l0: f32,
    r0: f32,
    dl: f32,
    dr: f32,
}

/// Equal-power pan for mono sources, so a centred sound sends half its power
/// to each side; a stereo source is balanced instead and stays at unity when
/// centred.
fn pan_gains(pan: f32, channels: u16) -> (f32, f32) {
    let angle = (pan + 1.0) * FRAC_PI_4;
    let (l, r) = (angle.cos(), angle.sin());
    if channels == 1 {
        (l, r)
    } else {
        ((l * SQRT_2).min(1.0), (r * SQRT_2).min(1.0))
    }
}

fn read_frame(samples: &[f32], channels: u16, at: usize) -> (f32, f32) {
    if channels == 1 {
        let s = samples[at];
        (s, s)
    } else {
        (samples[2 * at], samples[2 * at + 1])
    }
}

fn add_constant(samples: &[f32], channels: u16, pos: usize, gl: f32, gr: f32, out: &mut [f32]) {
    let n = out.len() / 2;
    if channels == 1 {
        for (frame, &s) in out
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(&samples[pos..pos + n])
        {
            frame[0] += s * gl;
            frame[1] += s * gr;
        }
    } else {
        let src = &samples[2 * pos..2 * (pos + n)];
        for (frame, s) in out
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(src.as_chunks::<2>().0.iter())
        {
            frame[0] += s[0] * gl;
            frame[1] += s[1] * gr;
        }
    }
}
