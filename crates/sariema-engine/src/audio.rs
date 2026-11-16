// SPDX-License-Identifier: Apache-2.0

//! The sound stage: the game's sound and music requests as mixer commands
//! stamped with the step's mixer frame, sounds tied to an entity heard from
//! the camera as it moves, and the feed between steps that opens and decodes
//! streams and, without a device, mixes the steps' samples. The step itself
//! never decodes: a decoder allocates as it reads.

use glam::Vec3;
use sariema_audio::{
    Audio, AudioError, Feeding, Mixer, MixerSettings, MusicFades, PlayParams, SoundId, VoiceId,
    gain_to_db,
};
use sariema_core::{STEP_HZ, STEP_SECONDS};
use sariema_format::script::{Bus, Command, Entity, Event, name_id};
use sariema_format::sound::{Load, SoundBank};
use sariema_runtime::camera::CameraPose;
use sariema_runtime::outbox::{MusicRequest, SoundRequest};
use sariema_runtime::state::State;

use crate::settings::SoundSettings;

#[derive(Clone, Copy)]
struct Emitter {
    entity: Entity,
    voice: VoiceId,
    /// The sound's own gain times the request's volume.
    gain: f32,
    /// Gain and pan last sent.
    sent: (f32, f32),
}

/// A request the step left to the feed: a sound still to be decoded or
/// opened as a stream, or any music request, kept in order.
#[derive(Clone, Copy)]
enum Pending {
    Sound {
        at: u64,
        id: SoundId,
        request: SoundRequest,
        heard: (f32, f32),
    },
    Music {
        at: u64,
        request: MusicRequest,
    },
    Ambience {
        at: u64,
        id: SoundId,
        fade: f32,
    },
}

pub struct Sound {
    pub audio: Audio,
    /// Kept to mix offline; the host takes it to feed a device.
    mixer: Option<Mixer>,
    ids: Vec<u32>,
    settings: SoundSettings,
    /// Oldest first.
    emitters: Vec<Emitter>,
    pending: Vec<Pending>,
    /// The next step to mix offline.
    mixed: u64,
    scratch: Vec<f32>,
    hash: u64,
    /// Requests the mixer refused, for the game at the next step.
    pub rejected: Vec<Event<'static>>,
    /// By room, its ambience.
    ambiences: Vec<Option<SoundId>>,
    /// The room whose ambience plays, and its voice once playing.
    ambience: Option<(usize, Option<VoiceId>)>,
    /// The last feed played what the steps left.
    played: bool,
}

impl Sound {
    pub fn new(
        bank: SoundBank,
        base: MixerSettings,
        settings: SoundSettings,
    ) -> Result<Sound, AudioError> {
        let (mixer_settings, mut defs) = sariema_audio::runtime(bank, base);
        if settings.predecode_streams {
            for def in defs.iter_mut().filter(|d| d.stream) {
                def.stream = false;
                def.load = Load::Bank;
            }
        }
        let ids = defs.iter().map(|d| name_id(&d.name)).collect();
        let rate = mixer_settings.sample_rate;
        let (audio, mixer) = Audio::new(mixer_settings, defs, feeding(settings.stream_thread))?;
        Ok(Sound {
            audio,
            mixer: Some(mixer),
            ids,
            emitters: Vec::with_capacity(settings.emitters),
            pending: Vec::with_capacity(settings.pending),
            mixed: 0,
            settings,
            scratch: vec![0.0; 2 * (frame(1, rate) as usize + 1)],
            hash: FNV_OFFSET,
            rejected: Vec::with_capacity(settings.pending),
            ambiences: Vec::new(),
            ambience: None,
            played: false,
        })
    }

    /// Each room's ambience by sound name; an unknown name is an error.
    pub fn set_ambiences(&mut self, names: &[Option<&str>]) -> Result<(), AudioError> {
        self.ambiences = (names.iter())
            .map(|name| match name {
                Some(name) => (self.id(name_id(name)))
                    .map(Some)
                    .ok_or_else(|| AudioError::UnknownSound(name.to_string())),
                None => Ok(None),
            })
            .collect::<Result<_, _>>()?;
        Ok(())
    }

    /// The active room's ambience in place of the last room's: the old one
    /// fades out as the new one fades in, over the switch's duration.
    fn ambience(&mut self, s: &State, at: u64) {
        let room = s.scenery.active();
        if self.ambience.is_some_and(|(r, _)| r == room) || self.ambiences.is_empty() {
            return;
        }
        let fade = match (s.scenery.switch(), self.ambience) {
            (_, None) => 0.0,
            (Some(switch), _) => switch.steps as f32 * STEP_SECONDS,
            (None, _) => self.settings.ambience_fade_s,
        };
        if let Some((_, Some(voice))) = self.ambience
            && let Err(e) = self.audio.stop(at, voice, fade)
        {
            reject(&mut self.rejected, &PLAY_SOUND, e, Entity::NONE);
        }
        self.ambience = Some((room, None));
        if let Some(Some(id)) = self.ambiences.get(room) {
            self.defer(Pending::Ambience { at, id: *id, fade });
        }
    }

    /// The room whose ambience plays, and whether its voice started.
    pub fn ambience_room(&self) -> Option<(usize, bool)> {
        self.ambience.map(|(r, v)| (r, v.is_some()))
    }

    /// Whether room `room`'s ambience stream is open ahead of its play.
    pub fn ambience_prepared(&self, room: usize) -> bool {
        (self.ambiences.get(room).copied().flatten())
            .is_some_and(|id| self.audio.is_prepared(id, true))
    }

    pub(crate) fn has_ambiences(&self) -> bool {
        self.ambiences.iter().any(Option::is_some)
    }

    /// Opens ahead the ambience of the first room of `wanted` but the active
    /// one whose ambience is not open yet, and closes those of the rooms no
    /// longer wanted. Opens nothing while a switch lasts or in a feed that
    /// played something, so no feed opens more than one stream and none
    /// during a switch.
    pub(crate) fn prepare_ambiences(&mut self, wanted: &[usize], switching: bool) {
        let active = self.ambience.map(|(r, _)| r);
        let ambiences = &self.ambiences;
        let next = |r: &usize| Some(*r) != active;
        let ahead = |id: SoundId| {
            (wanted.iter().filter(|r| next(r))).any(|&r| ambiences.get(r) == Some(&Some(id)))
        };
        self.audio.release_prepared(ahead);
        if self.played || switching {
            return;
        }
        let closed = (wanted.iter().filter(|r| next(r)))
            .filter_map(|&r| self.ambiences.get(r).copied().flatten())
            .find(|&id| !self.audio.is_prepared(id, true));
        if let Some(id) = closed
            && let Err(e) = self.audio.prepare(id, true)
        {
            reject(&mut self.rejected, &PLAY_SOUND, e, Entity::NONE);
        }
    }

    /// The mixer, for a device to pull from; the engine then stops mixing
    /// offline.
    pub fn take_mixer(&mut self) -> Option<Mixer> {
        self.mixer.take()
    }

    /// Whether the feed mixes the sound itself, no device having taken the
    /// mixer.
    pub fn offline(&self) -> bool {
        self.mixer.is_some()
    }

    /// The mixer frame step `n` starts at.
    pub fn frame_of(&self, n: u64) -> u64 {
        frame(n, self.audio.settings().sample_rate)
    }

    /// Carries out step `n`'s requests at mixer frame `at` where that needs
    /// no decoding, leaving the rest to [`Sound::feed`].
    pub fn step(&mut self, s: &State, at: u64) {
        let listener = s.camera.pose();
        for &request in &s.outbox.sounds {
            let Some(id) = self.id(request.sound) else {
                reject(
                    &mut self.rejected,
                    &PLAY_SOUND,
                    unknown(request.sound),
                    request.emitter,
                );
                continue;
            };
            let heard = self.heard(s, listener, request.emitter);
            let decoded = !self.audio.sounds()[id.0 as usize].stream && self.audio.is_loaded(id);
            if decoded {
                self.play(at, id, request, heard);
            } else {
                self.defer(Pending::Sound {
                    at,
                    id,
                    request,
                    heard,
                });
            }
        }
        for &request in &s.outbox.music {
            self.defer(Pending::Music { at, request });
        }
        self.ambience(s, at);
        self.follow(s, listener, at);
    }

    /// Between steps: plays what the steps left (decoding a sound loaded on
    /// demand, opening a stream), decodes the streams ahead, and without a
    /// device mixes every step up to `steps`. It allocates; the steps do not.
    pub fn feed(&mut self, steps: u64) {
        self.played = !self.pending.is_empty();
        for k in 0..self.pending.len() {
            match self.pending[k] {
                Pending::Sound {
                    at,
                    id,
                    request,
                    heard,
                } => match self.audio.preload(id) {
                    Ok(()) => self.play(at, id, request, heard),
                    Err(e) => reject(&mut self.rejected, &PLAY_SOUND, e, request.emitter),
                },
                Pending::Music { at, request } => self.music(at, request),
                Pending::Ambience { at, id, fade } => {
                    let params = PlayParams {
                        looped: Some(true),
                        fade_in_s: fade,
                        ..PlayParams::default()
                    };
                    let played = self
                        .audio
                        .preload(id)
                        .and_then(|()| self.audio.play(at, id, params));
                    match played {
                        Ok(voice) => {
                            if let Some((_, v)) = &mut self.ambience {
                                *v = Some(voice);
                            }
                        }
                        Err(e) => reject(&mut self.rejected, &PLAY_SOUND, e, Entity::NONE),
                    }
                }
            }
        }
        self.pending.clear();
        if let Err(e) = self.audio.pump() {
            reject(&mut self.rejected, &PLAY_SOUND, e, Entity::NONE);
        }
        self.audio.collect();
        while self.mixed < steps {
            self.mix(self.mixed);
            self.mixed += 1;
        }
    }

    fn defer(&mut self, pending: Pending) {
        if self.pending.len() < self.pending.capacity() {
            self.pending.push(pending);
            return;
        }
        let (command, entity) = match pending {
            Pending::Sound { request, .. } => (&PLAY_SOUND, request.emitter),
            Pending::Music { request, .. } => (music_command(request), Entity::NONE),
            Pending::Ambience { .. } => (&PLAY_SOUND, Entity::NONE),
        };
        reject(&mut self.rejected, command, AudioError::QueueFull, entity);
    }

    fn play(&mut self, at: u64, id: SoundId, request: SoundRequest, heard: (f32, f32)) {
        let gain = self.audio.sounds()[id.0 as usize].gain * request.volume;
        let params = PlayParams {
            gain: request.volume * heard.0,
            pan: heard.1,
            ..PlayParams::default()
        };
        match self.audio.play(at, id, params) {
            Ok(voice) if !request.emitter.is_none() && self.settings.emitters > 0 => {
                if self.emitters.len() == self.settings.emitters {
                    self.emitters.remove(0);
                }
                self.emitters.push(Emitter {
                    entity: request.emitter,
                    voice,
                    gain,
                    sent: heard,
                });
            }
            Ok(_) => {}
            Err(e) => reject(&mut self.rejected, &PLAY_SOUND, e, request.emitter),
        }
    }

    fn music(&mut self, at: u64, request: MusicRequest) {
        let result = match request {
            MusicRequest::Play { music, fade } => match self.id(music) {
                Some(id) => {
                    let fades = MusicFades {
                        fade_in_s: Some(fade),
                        fade_out_s: Some(fade),
                        curve: None,
                    };
                    self.audio.music(at, id, fades, None).map(|_| ())
                }
                None => Err(unknown(music)),
            },
            MusicRequest::Stop { fade } => self.audio.stop_music(at, Some(fade)),
            MusicRequest::BusVolume { bus, volume, fade } => {
                match self.audio.settings().bus(bus.name()) {
                    Some(b) => self.audio.set_bus(at, b, gain_to_db(volume), fade),
                    None => Ok(()),
                }
            }
        };
        if let Err(e) = result {
            reject(&mut self.rejected, music_command(request), e, Entity::NONE);
        }
    }

    /// Mixes step `n`'s samples, folding them into [`Sound::hash`].
    fn mix(&mut self, n: u64) {
        let frames = (self.frame_of(n + 1) - self.frame_of(n)) as usize;
        let Some(mixer) = self.mixer.as_mut() else {
            return;
        };
        let out = &mut self.scratch[..2 * frames];
        mixer.render(out);
        for x in out.iter() {
            self.hash = (self.hash ^ x.to_bits() as u64).wrapping_mul(FNV_PRIME);
        }
    }

    /// FNV-1a over every sample mixed offline.
    pub fn hash(&self) -> u64 {
        self.hash
    }

    fn id(&self, name: u32) -> Option<SoundId> {
        self.ids
            .iter()
            .position(|&n| n == name)
            .map(|i| SoundId(i as u32))
    }

    /// Gain and pan of a sound at `emitter` heard from the camera: full gain
    /// within the reference distance, then the inverse-distance model; pan
    /// along the camera's right.
    fn heard(&self, s: &State, listener: Option<CameraPose>, emitter: Entity) -> (f32, f32) {
        let (Some(camera), Some(pose)) = (listener, s.world.current_pose(emitter)) else {
            return (1.0, 0.0);
        };
        let to = pose.position - camera.position;
        let reference = self.settings.reference_distance.max(f32::EPSILON);
        let d = to.length().max(reference);
        let gain = reference / (reference + self.settings.rolloff * (d - reference));
        let right = camera.rotation * Vec3::X;
        let flat = Vec3::new(to.x, 0.0, to.z);
        let pan = if flat.length_squared() > 0.0 {
            flat.normalize()
                .dot(Vec3::new(right.x, 0.0, right.z).normalize_or_zero())
        } else {
            0.0
        };
        (gain, pan.clamp(-1.0, 1.0))
    }

    fn follow(&mut self, s: &State, listener: Option<CameraPose>, at: u64) {
        let mut k = 0;
        while k < self.emitters.len() {
            let e = self.emitters[k];
            if !s.world.is_alive(e.entity) {
                self.emitters.remove(k);
                continue;
            }
            k += 1;
            let heard = self.heard(s, listener, e.entity);
            let change = (heard.0 - e.sent.0).abs().max((heard.1 - e.sent.1).abs());
            if change < self.settings.min_change {
                continue;
            }
            self.emitters[k - 1].sent = heard;
            if let Err(err) =
                self.audio
                    .set_voice(at, e.voice, e.gain * heard.0, heard.1, STEP_SECONDS)
            {
                reject(&mut self.rejected, &PLAY_SOUND, err, e.entity);
            }
        }
    }

    /// Gain and pan last sent for each sound following an entity.
    pub fn emitters(&self) -> impl Iterator<Item = (Entity, f32, f32)> + '_ {
        self.emitters.iter().map(|e| (e.entity, e.sent.0, e.sent.1))
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[cfg(not(target_arch = "wasm32"))]
fn feeding(thread: bool) -> Feeding {
    if thread {
        Feeding::Thread
    } else {
        Feeding::Manual
    }
}

#[cfg(target_arch = "wasm32")]
fn feeding(_: bool) -> Feeding {
    Feeding::Manual
}

/// Whole frames, so that the mixer clock never drifts from the steps.
fn frame(step: u64, rate: u32) -> u64 {
    step * rate as u64 / STEP_HZ as u64
}

fn unknown(sound: u32) -> AudioError {
    AudioError::UnknownSound(format!("{sound:#010x}"))
}

fn reject(out: &mut Vec<Event<'static>>, command: &Command<'_>, e: AudioError, entity: Entity) {
    out.push(Event::Rejected {
        opcode: command.opcode(),
        code: e.code(),
        entity,
    });
}

const PLAY_SOUND: Command<'static> = Command::PlaySound {
    sound: 0,
    bus: Bus::Effects,
    emitter: Entity::NONE,
    volume: 0.0,
};
fn music_command(request: MusicRequest) -> &'static Command<'static> {
    match request {
        MusicRequest::Play { .. } => &PLAY_MUSIC,
        MusicRequest::Stop { .. } => &STOP_MUSIC,
        MusicRequest::BusVolume { .. } => &SET_BUS_VOLUME,
    }
}

const PLAY_MUSIC: Command<'static> = Command::PlayMusic {
    music: 0,
    fade: 0.0,
};
const STOP_MUSIC: Command<'static> = Command::StopMusic { fade: 0.0 };
const SET_BUS_VOLUME: Command<'static> = Command::SetBusVolume {
    bus: Bus::Music,
    volume: 0.0,
    fade: 0.0,
};
