// SPDX-License-Identifier: Apache-2.0

//! The audio-thread side: a pure function of the commands received and the
//! frames asked for. It never allocates, locks or touches a file.
//!
//! Frames are mixed in blocks of `block_frames` aligned on the mixer clock,
//! whatever the device's buffer size, so the same commands give the same
//! samples with any buffer size. Within a block, work is split at every
//! frame where something happens: a command, the end of a pause fade, a duck
//! release, a held track going silent.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rtrb::{Consumer, Producer};

use crate::command::{Op, Play, Src, Timed, VoiceId};
use crate::duck::Duck;
use crate::ramp::{Curve, Ramp};
use crate::settings::{MixerSettings, Steal, seconds_to_frames};
use crate::stream::StreamReader;
use crate::voice::{Counters, StreamGate, Voice};

/// Counters the mixer publishes after every block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MixStats {
    /// Mixer clock: frames mixed so far.
    pub now: u64,
    /// Times a stream ran dry and faded out.
    pub underruns: u64,
    /// Voices faded out to make room for a new play.
    pub steals: u64,
    /// Plays dropped: no voice could be stolen or no slot was free.
    pub refused: u64,
    /// Commands that arrived after their frame.
    pub late: u64,
    /// Fading voices cut short to make room for music.
    pub cuts: u64,
    /// Voices with a source, fading ones included.
    pub voices: u64,
}

#[derive(Default)]
pub(crate) struct SharedStats {
    values: [AtomicU64; 7],
}

impl SharedStats {
    fn publish(&self, s: &MixStats) {
        let all = [
            s.now,
            s.underruns,
            s.steals,
            s.refused,
            s.late,
            s.cuts,
            s.voices,
        ];
        for (slot, v) in self.values.iter().zip(all) {
            slot.store(v, Ordering::Release);
        }
    }

    pub fn read(&self) -> MixStats {
        let v = |i: usize| self.values[i].load(Ordering::Acquire);
        MixStats {
            now: v(0),
            underruns: v(1),
            steals: v(2),
            refused: v(3),
            late: v(4),
            cuts: v(5),
            voices: v(6),
        }
    }
}

/// One bus, or the master with `bus: None`, after its gain and ducking, for
/// one block. Meant for meters and offline analysis.
pub struct Tap<'a> {
    pub bus: Option<usize>,
    /// Interleaved stereo.
    pub samples: &'a [f32],
    /// Ducking applied at the end of the block, in dB.
    pub duck_db: f32,
}

struct Bus {
    gain: Ramp,
    buf: Vec<f32>,
    ducks: Vec<u16>,
}

struct MusicRoles {
    current: Option<VoiceId>,
    held: Option<VoiceId>,
    stinger: Option<VoiceId>,
    curve: Curve,
    stinger_out: u32,
    stinger_return: u32,
}

pub struct Mixer {
    block: usize,
    declick: u32,
    gate: StreamGate,
    now: u64,
    commands: Consumer<Timed>,
    /// Sorted by frame, latest first, so the next one due is last.
    pending: Vec<Timed>,
    retired: Producer<StreamReader>,
    zombies: Vec<StreamReader>,
    voices: Vec<Voice>,
    audible: usize,
    steal: Steal,
    buses: Vec<Bus>,
    ducks: Vec<Duck>,
    master: Ramp,
    pause: Ramp,
    music: MusicRoles,
    mix: Vec<f32>,
    out: Vec<f32>,
    out_at: usize,
    stats: MixStats,
    counters: Counters,
    shared: Arc<SharedStats>,
}

/// Room for the music roles besides the voice limits: current, held, stinger.
pub(crate) const MUSIC_SLOTS: usize = 3;

impl Mixer {
    pub(crate) fn new(
        settings: &MixerSettings,
        commands: Consumer<Timed>,
        retired: Producer<StreamReader>,
        ducks: Vec<Duck>,
        shared: Arc<SharedStats>,
    ) -> Mixer {
        let block = settings.block_frames.max(1) as usize;
        let slots = (settings.voices + settings.fading_voices) as usize + MUSIC_SLOTS;
        let mut buses: Vec<Bus> = settings
            .buses
            .iter()
            .map(|b| Bus {
                gain: Ramp::constant(crate::ramp::db_to_gain(b.gain_db)),
                buf: vec![0.0; 2 * block],
                ducks: Vec::new(),
            })
            .collect();
        for (i, duck) in ducks.iter().enumerate() {
            for &t in &duck.targets {
                buses[t as usize].ducks.push(i as u16);
            }
        }
        let rate = settings.sample_rate;
        let declick = settings.declick_frames();
        Mixer {
            block,
            declick,
            gate: StreamGate {
                declick,
                resume_fill: settings.stream.resume_fill,
            },
            now: 0,
            commands,
            pending: Vec::with_capacity(settings.commands as usize),
            retired,
            zombies: Vec::with_capacity(slots + settings.commands as usize),
            voices: (0..slots).map(|_| Voice::empty()).collect(),
            audible: settings.voices as usize,
            steal: settings.steal,
            buses,
            ducks,
            master: Ramp::constant(1.0),
            pause: Ramp::constant(1.0),
            music: MusicRoles {
                current: None,
                held: None,
                stinger: None,
                curve: settings.music.curve,
                stinger_out: seconds_to_frames(settings.music.stinger_out_s, rate),
                stinger_return: seconds_to_frames(settings.music.stinger_return_s, rate),
            },
            mix: vec![0.0; 2 * block],
            out: vec![0.0; 2 * block],
            out_at: 2 * block,
            stats: MixStats::default(),
            counters: Counters { underruns: 0 },
            shared,
        }
    }

    /// Fills interleaved stereo `out`, any length.
    pub fn render(&mut self, out: &mut [f32]) {
        self.render_tapped(out, &mut |_| {});
    }

    /// [`Mixer::render`], calling `tap` with every bus and the master of each
    /// block as it is mixed.
    pub fn render_tapped(&mut self, out: &mut [f32], tap: &mut impl FnMut(Tap<'_>)) {
        let mut written = 0;
        while written < out.len() {
            if self.out_at == self.out.len() {
                self.mix_block(tap);
                self.out_at = 0;
            }
            let k = (out.len() - written).min(self.out.len() - self.out_at);
            out[written..written + k].copy_from_slice(&self.out[self.out_at..self.out_at + k]);
            written += k;
            self.out_at += k;
        }
    }

    pub fn stats(&self) -> MixStats {
        self.stats
    }

    fn mix_block(&mut self, tap: &mut impl FnMut(Tap<'_>)) {
        self.receive();
        for bus in &mut self.buses {
            bus.buf.fill(0.0);
        }
        self.mix.fill(0.0);
        let mut o = 0;
        while o < self.block {
            let t = self.now;
            self.apply_due(t);
            self.update_ducks(t);
            let n = (self.next_event(t) - t).min((self.block - o) as u64) as usize;
            self.render_span(o, n);
            self.now += n as u64;
            o += n;
        }
        for (i, bus) in self.buses.iter().enumerate() {
            let duck_db = bus
                .ducks
                .iter()
                .map(|&d| self.ducks[d as usize].db(self.now))
                .sum();
            tap(Tap {
                bus: Some(i),
                samples: &bus.buf,
                duck_db,
            });
        }
        tap(Tap {
            bus: None,
            samples: &self.out,
            duck_db: 0.0,
        });
        self.flush_zombies();
        self.stats.now = self.now;
        self.stats.underruns = self.counters.underruns;
        self.stats.voices = self.voices.iter().filter(|v| !v.is_free()).count() as u64;
        self.shared.publish(&self.stats);
    }

    fn receive(&mut self) {
        while self.pending.len() < self.pending.capacity() {
            let Ok(command) = self.commands.pop() else {
                break;
            };
            let at = command.at;
            let i = self.pending.partition_point(|c| c.at > at);
            self.pending.insert(i, command);
        }
    }

    fn apply_due(&mut self, t: u64) {
        while self.pending.last().is_some_and(|c| c.at <= t) {
            let command = self.pending.pop().expect("checked above");
            if command.at != 0 && command.at < t {
                self.stats.late += 1;
            }
            self.apply(command.op, t);
        }
    }

    fn next_event(&self, t: u64) -> u64 {
        let mut next = u64::MAX;
        if let Some(c) = self.pending.last() {
            next = next.min(c.at.max(t + 1));
        }
        for duck in &self.ducks {
            if let Some(r) = duck.next_event(t) {
                next = next.min(r);
            }
        }
        if self.pause.target() == 0.0 && self.pause.end() > t {
            next = next.min(self.pause.end());
        }
        for v in &self.voices {
            if !v.is_free()
                && !v.stopping
                && let Some(end) = v.silent_at()
                && end > t
            {
                next = next.min(end);
            }
        }
        next
    }

    fn update_ducks(&mut self, t: u64) {
        for duck in &mut self.ducks {
            let key = duck.key;
            let active = self
                .voices
                .iter()
                .any(|v| !v.is_free() && !v.frozen && v.bus == key);
            duck.update(t, active);
        }
    }

    fn render_span(&mut self, o: usize, n: usize) {
        let t = self.now;
        let frozen = self.pause.target() == 0.0 && t >= self.pause.end();
        if !frozen {
            for i in 0..self.voices.len() {
                let v = &mut self.voices[i];
                if v.is_free() {
                    continue;
                }
                let buf = &mut self.buses[v.bus as usize].buf[2 * o..2 * (o + n)];
                if !v.render(t, n, &self.gate, buf, &mut self.counters) {
                    self.release(i, t + n as u64);
                }
            }
        }
        let end = 2 * (o + n);
        for bus in &mut self.buses {
            let flat = bus.gain.is_flat(t, n as u32)
                && bus
                    .ducks
                    .iter()
                    .all(|&d| self.ducks[d as usize].is_flat(t, n as u32));
            let gain_at = |t: u64| {
                bus.ducks.iter().fold(bus.gain.value(t), |g, &d| {
                    g * self.ducks[d as usize].gain(t)
                })
            };
            let constant = gain_at(t);
            for (k, (frame, mix)) in bus.buf[2 * o..end]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .zip(self.mix[2 * o..end].as_chunks_mut::<2>().0.iter_mut())
                .enumerate()
            {
                let g = if flat {
                    constant
                } else {
                    gain_at(t + k as u64)
                };
                frame[0] *= g;
                frame[1] *= g;
                mix[0] += frame[0];
                mix[1] += frame[1];
            }
        }
        let flat = self.master.is_flat(t, n as u32) && self.pause.is_flat(t, n as u32);
        let constant = self.master.value(t) * self.pause.value(t);
        for (k, (out, mix)) in self.out[2 * o..end]
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(self.mix[2 * o..end].as_chunks::<2>().0.iter())
            .enumerate()
        {
            let at = t + k as u64;
            let g = if flat {
                constant
            } else {
                self.master.value(at) * self.pause.value(at)
            };
            out[0] = mix[0] * g;
            out[1] = mix[1] * g;
        }
    }

    fn apply(&mut self, op: Op, t: u64) {
        match op {
            Op::Play(play) => self.start(play, t),
            Op::Stop { voice, fade } => {
                let fade = fade.max(self.declick);
                if let Some(v) = self.find(voice) {
                    v.stop(t, fade, Curve::Linear);
                }
            }
            Op::SetVoice {
                voice,
                gain,
                pan,
                ramp,
            } => {
                if let Some(v) = self.find(voice) {
                    v.set(t, gain, pan, ramp);
                }
            }
            Op::SetBus { bus, gain, ramp } => {
                if let Some(b) = self.buses.get_mut(bus as usize) {
                    b.gain.retarget(t, gain, ramp, Curve::Linear);
                }
            }
            Op::SetMaster { gain, ramp } => self.master.retarget(t, gain, ramp, Curve::Linear),
            Op::Music { play, fade_out } => self.music(play, fade_out, t),
            Op::Stinger(play) => self.stinger(play, t),
            Op::MusicStop { fade } => {
                let fade = fade.max(self.declick);
                let curve = self.music.curve;
                for id in [self.music.current, self.music.stinger, self.music.held] {
                    if let Some(v) = id.and_then(|id| self.find(id)) {
                        v.stop(t, fade, curve);
                    }
                }
                self.music.current = None;
                self.music.stinger = None;
                self.music.held = None;
            }
            Op::Pause => self.pause.retarget(t, 0.0, self.declick, Curve::Linear),
            Op::Resume => self.pause.retarget(t, 1.0, self.declick, Curve::Linear),
        }
    }

    fn music(&mut self, play: Play, fade_out: u32, t: u64) {
        let same = self
            .music
            .current
            .and_then(|id| self.find(id))
            .is_some_and(|v| v.sound == play.sound && !v.stopping);
        if same {
            self.discard(play.src);
            return;
        }
        let (curve, declick) = (self.music.curve, self.declick);
        if let Some(v) = self.music.current.and_then(|id| self.find(id)) {
            v.stop(t, fade_out.max(declick), curve);
        }
        if let Some(v) = self.music.stinger.and_then(|id| self.find(id)) {
            v.stop(t, fade_out.max(declick), curve);
        }
        if let Some(v) = self.music.held.and_then(|id| self.find(id)) {
            v.stop(t, declick, curve);
        }
        self.music.held = None;
        self.music.stinger = None;
        self.music.current = self.start_protected(play, t);
    }

    fn stinger(&mut self, play: Play, t: u64) {
        let (curve, declick, out) = (self.music.curve, self.declick, self.music.stinger_out);
        if let Some(v) = self.music.stinger.and_then(|id| self.find(id)) {
            v.stop(t, declick, curve);
        }
        if let Some(id) = self.music.current.take() {
            if let Some(v) = self.find(id) {
                v.hold(t, out.max(declick), curve);
            }
            self.music.held = Some(id);
        }
        self.music.stinger = self.start_protected(play, t);
    }

    fn start(&mut self, play: Play, t: u64) {
        let audible = self
            .voices
            .iter()
            .filter(|v| !v.is_free() && !v.stopping && !v.protected)
            .count();
        let Some(slot) = self.voices.iter().position(Voice::is_free) else {
            self.refuse(play.src);
            return;
        };
        if audible >= self.audible {
            let candidates = self.voices.iter().enumerate().filter(|(_, v)| {
                !v.is_free() && !v.stopping && !v.protected && v.priority <= play.priority
            });
            let victim = match self.steal {
                Steal::Refuse => None,
                Steal::Oldest => candidates.min_by_key(|(_, v)| (v.priority, v.started)),
                Steal::Quietest => candidates.min_by(|(_, a), (_, b)| {
                    (a.priority, a.level(t))
                        .partial_cmp(&(b.priority, b.level(t)))
                        .expect("levels are finite")
                }),
            };
            let Some((victim, _)) = victim else {
                self.refuse(play.src);
                return;
            };
            self.voices[victim].stop(t, self.declick, Curve::Linear);
            self.stats.steals += 1;
        }
        self.voices[slot].start(t, play, self.declick, false);
    }

    fn start_protected(&mut self, play: Play, t: u64) -> Option<VoiceId> {
        let slot = match self.voices.iter().position(Voice::is_free) {
            Some(slot) => slot,
            None => {
                let quietest = self
                    .voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.stopping)
                    .min_by(|(_, a), (_, b)| a.level(t).partial_cmp(&b.level(t)).expect("finite"))
                    .map(|(i, _)| i);
                let Some(slot) = quietest else {
                    self.refuse(play.src);
                    return None;
                };
                self.stats.cuts += 1;
                self.release(slot, t);
                slot
            }
        };
        let id = play.voice;
        self.voices[slot].start(t, play, self.declick, true);
        Some(id)
    }

    fn find(&mut self, id: VoiceId) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id && !v.is_free())
    }

    fn refuse(&mut self, src: Src) {
        self.stats.refused += 1;
        self.discard(src);
    }

    /// Drops a static sound's reference, which the control side always
    /// outlives, or hands a stream back to the control side to be freed.
    fn discard(&mut self, src: Src) {
        match src {
            Src::Static(pcm) => drop(pcm),
            Src::Stream(reader) => {
                if let Err(rtrb::PushError::Full(reader)) = self.retired.push(reader) {
                    self.zombies.push(reader);
                }
            }
        }
    }

    fn flush_zombies(&mut self) {
        while let Some(reader) = self.zombies.pop() {
            if let Err(rtrb::PushError::Full(reader)) = self.retired.push(reader) {
                self.zombies.push(reader);
                break;
            }
        }
    }

    fn release(&mut self, i: usize, t: u64) {
        let id = self.voices[i].id;
        if let Some(src) = self.voices[i].free() {
            self.discard(src);
        }
        let roles = &mut self.music;
        if roles.current == Some(id) {
            roles.current = None;
        }
        if roles.held == Some(id) {
            roles.held = None;
        }
        if roles.stinger == Some(id) {
            roles.stinger = None;
            if let Some(held) = roles.held.take() {
                let (len, curve) = (roles.stinger_return.max(self.declick), roles.curve);
                roles.current = Some(held);
                if let Some(v) = self.find(held) {
                    v.unhold(t, len, curve);
                }
            }
        }
    }
}
