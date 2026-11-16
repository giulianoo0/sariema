// SPDX-License-Identifier: Apache-2.0

//! The sound stage of a step: the game's sound and music requests as mixer
//! commands stamped with the step's mixer frame, sounds tied to an entity
//! heard from the camera as it moves, and, without a device, the step's
//! samples mixed at once.

use glam::Vec3;
use sariema_audio::{
    Audio, AudioError, Feeding, Mixer, MixerSettings, MusicFades, PlayParams, SoundId, VoiceId,
    gain_to_db,
};
use sariema_core::{STEP_HZ, STEP_SECONDS};
use sariema_format::script::{Bus, Command, Entity, Event, name_id};
use sariema_format::sound::SoundBank;
use sariema_runtime::camera::CameraPose;
use sariema_runtime::outbox::MusicRequest;
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

pub struct Sound {
    pub audio: Audio,
    /// Kept to mix offline; the host takes it to feed a device.
    mixer: Option<Mixer>,
    ids: Vec<u32>,
    settings: SoundSettings,
    /// Oldest first.
    emitters: Vec<Emitter>,
    scratch: Vec<f32>,
    hash: u64,
    /// Requests the mixer refused, for the game at the next step.
    pub rejected: Vec<Event<'static>>,
}

impl Sound {
    pub fn new(
        bank: SoundBank,
        base: MixerSettings,
        settings: SoundSettings,
    ) -> Result<Sound, AudioError> {
        let (mixer_settings, defs) = sariema_audio::runtime(bank, base);
        let ids = defs.iter().map(|d| name_id(&d.name)).collect();
        let rate = mixer_settings.sample_rate;
        let (audio, mixer) = Audio::new(mixer_settings, defs, Feeding::Manual)?;
        Ok(Sound {
            audio,
            mixer: Some(mixer),
            ids,
            emitters: Vec::with_capacity(settings.emitters),
            settings,
            scratch: vec![0.0; 2 * (frame(1, rate) as usize + 1)],
            hash: FNV_OFFSET,
            rejected: Vec::new(),
        })
    }

    /// The mixer, for a device to pull from; the engine then stops mixing
    /// offline.
    pub fn take_mixer(&mut self) -> Option<Mixer> {
        self.mixer.take()
    }

    /// Whether the steps mix the sound themselves, no device having taken
    /// the mixer.
    pub fn offline(&self) -> bool {
        self.mixer.is_some()
    }

    /// The mixer frame step `n` starts at.
    pub fn frame_of(&self, n: u64) -> u64 {
        frame(n, self.audio.settings().sample_rate)
    }

    /// Carries out step `n`'s requests at mixer frame `at`.
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
            if let Err(e) = self.audio.preload(id) {
                reject(&mut self.rejected, &PLAY_SOUND, e, request.emitter);
                continue;
            }
            let gain = self.audio.sounds()[id.0 as usize].gain * request.volume;
            let heard = self.heard(s, listener, request.emitter);
            let params = PlayParams {
                gain: request.volume * heard.0,
                pan: heard.1,
                ..PlayParams::default()
            };
            match self.audio.play(at, id, params) {
                Ok(voice) if !request.emitter.is_none() => {
                    if self.emitters.len() == self.settings.emitters {
                        self.emitters.remove(0);
                    }
                    if self.settings.emitters > 0 {
                        self.emitters.push(Emitter {
                            entity: request.emitter,
                            voice,
                            gain,
                            sent: heard,
                        });
                    }
                }
                Ok(_) => {}
                Err(e) => reject(&mut self.rejected, &PLAY_SOUND, e, request.emitter),
            }
        }
        for &request in &s.outbox.music {
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
                let command = match request {
                    MusicRequest::Play { .. } => &PLAY_MUSIC,
                    MusicRequest::Stop { .. } => &STOP_MUSIC,
                    MusicRequest::BusVolume { .. } => &SET_BUS_VOLUME,
                };
                reject(&mut self.rejected, command, e, Entity::NONE);
            }
        }
        self.follow(s, listener, at);
        if let Err(e) = self.audio.pump() {
            reject(&mut self.rejected, &PLAY_SOUND, e, Entity::NONE);
        }
        self.audio.collect();
    }

    /// Mixes step `n`'s samples when no device took the mixer, folding them
    /// into [`Sound::hash`].
    pub fn mix(&mut self, n: u64) {
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
