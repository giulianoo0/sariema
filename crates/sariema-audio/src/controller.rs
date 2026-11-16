// SPDX-License-Identifier: Apache-2.0

//! The control side, used from the simulation thread: it decodes, opens
//! streams and sends timed commands. Everything that allocates or frees
//! happens here.

use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};
use sariema_format::sound::{Container, Load, LoopPoints};

use crate::AudioError;
use crate::command::{Op, Play, Src, Timed, VoiceId};
use crate::decode::decode_all;
use crate::duck::Duck;
use crate::mixer::{MUSIC_SLOTS, MixStats, Mixer, SharedStats};
use crate::pcm::Pcm;
use crate::ramp::{Curve, db_to_gain};
use crate::settings::{MixerSettings, seconds_to_frames};
use crate::stream::{Feeder, Pump, StreamReader};

/// A sound the mixer can play.
#[derive(Clone, Debug)]
pub struct SoundDef {
    pub name: String,
    /// Index into [`MixerSettings::buses`].
    pub bus: usize,
    /// Linear gain the sound always plays at.
    pub gain: f32,
    pub container: Container,
    pub data: Arc<[u8]>,
    /// In source frames.
    pub loop_points: Option<LoopPoints>,
    /// Source frames.
    pub frames: u64,
    pub stream: bool,
    pub load: Load,
    pub enter_s: Option<f32>,
    pub leave_s: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SoundId(pub u32);

/// Who decodes streams ahead of the mixer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feeding {
    /// A thread of its own, started with the first stream.
    Thread,
    /// The caller, through [`Audio::pump`]; deterministic, for offline work.
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayParams {
    /// Linear, on top of the sound's own gain. Default 1.
    pub gain: f32,
    /// −1 left to 1 right. Default 0.
    pub pan: f32,
    /// Default: when the sound has loop points.
    pub looped: Option<bool>,
    /// Never shorter than the declick fade. Default 0.
    pub fade_in_s: f32,
    /// A play only steals voices of a priority at most its own. Default 128.
    pub priority: u8,
}

impl Default for PlayParams {
    fn default() -> PlayParams {
        PlayParams {
            gain: 1.0,
            pan: 0.0,
            looped: None,
            fade_in_s: 0.0,
            priority: 128,
        }
    }
}

/// Overrides for one music change; `None` takes the sound's `enter_s` or
/// `leave_s`, then the mixer's default.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MusicFades {
    pub fade_in_s: Option<f32>,
    pub fade_out_s: Option<f32>,
    pub curve: Option<Curve>,
}

pub struct Audio {
    settings: MixerSettings,
    commands: Producer<Timed>,
    retired: Consumer<StreamReader>,
    sounds: Vec<SoundDef>,
    loaded: Vec<Option<Arc<Pcm>>>,
    /// Unloaded sounds a voice may still be playing; freed once only this
    /// list holds them, so the audio thread never frees one.
    dying: Vec<(u32, Arc<Pcm>)>,
    next_voice: u64,
    music: Option<u32>,
    feeders: Vec<Feeder>,
    /// Dropping the sender lets the thread end once its streams are abandoned.
    thread: Option<(Sender<Feeder>, JoinHandle<()>)>,
    stream_errors: Arc<Mutex<Vec<String>>>,
    shared: Arc<SharedStats>,
    feeding: Feeding,
}

impl Audio {
    /// Builds the two sides and decodes every sound loaded with the bank.
    pub fn new(
        settings: MixerSettings,
        sounds: Vec<SoundDef>,
        feeding: Feeding,
    ) -> Result<(Audio, Mixer), AudioError> {
        let ducks = validate(&settings, &sounds)?;
        let (commands, consumer) = RingBuffer::new(settings.commands as usize);
        let slots = (settings.voices + settings.fading_voices) as usize + MUSIC_SLOTS;
        let (retired_producer, retired) = RingBuffer::new(slots + settings.commands as usize);
        let shared = Arc::new(SharedStats::default());
        let mixer = Mixer::new(&settings, consumer, retired_producer, ducks, shared.clone());
        let mut audio = Audio {
            settings,
            commands,
            retired,
            loaded: vec![None; sounds.len()],
            sounds,
            dying: Vec::new(),
            next_voice: 1,
            music: None,
            feeders: Vec::new(),
            thread: None,
            stream_errors: Arc::new(Mutex::new(Vec::new())),
            shared,
            feeding,
        };
        for i in 0..audio.sounds.len() {
            let def = &audio.sounds[i];
            if !def.stream && def.load == Load::Bank {
                audio.preload(SoundId(i as u32))?;
            }
        }
        Ok((audio, mixer))
    }

    pub fn settings(&self) -> &MixerSettings {
        &self.settings
    }

    pub fn sound(&self, name: &str) -> Result<SoundId, AudioError> {
        self.sounds
            .iter()
            .position(|s| s.name == name)
            .map(|i| SoundId(i as u32))
            .ok_or_else(|| AudioError::UnknownSound(name.into()))
    }

    pub fn sounds(&self) -> &[SoundDef] {
        &self.sounds
    }

    /// Mixer frames for `seconds`, rounded.
    pub fn frames(&self, seconds: f32) -> u32 {
        seconds_to_frames(seconds, self.settings.sample_rate)
    }

    /// Counters as of the last block mixed.
    pub fn stats(&self) -> MixStats {
        self.shared.read()
    }

    pub fn is_loaded(&self, sound: SoundId) -> bool {
        self.loaded[sound.0 as usize].is_some()
    }

    /// Decodes a sound that is not streamed so it can be played. Does nothing
    /// when it is already loaded.
    pub fn preload(&mut self, sound: SoundId) -> Result<(), AudioError> {
        let i = sound.0 as usize;
        let def = self.def(sound)?;
        if def.stream || self.loaded[i].is_some() {
            return Ok(());
        }
        if let Some(at) = self.dying.iter().position(|(s, _)| *s == sound.0) {
            self.loaded[i] = Some(self.dying.swap_remove(at).1);
            return Ok(());
        }
        let decoded = decode_all(def.data.clone(), def.container)
            .map_err(|e| AudioError::Decode(format!("{}: {e}", def.name)))?;
        let pcm = Pcm::from_decoded(
            &decoded,
            self.settings.sample_rate,
            self.settings.stream.resampler_half_taps,
            def.loop_points,
        );
        self.loaded[i] = Some(Arc::new(pcm));
        Ok(())
    }

    /// Releases a sound's samples once no voice plays them any more.
    pub fn unload(&mut self, sound: SoundId) {
        if let Some(pcm) = self.loaded.get_mut(sound.0 as usize).and_then(Option::take) {
            self.dying.push((sound.0, pcm));
        }
    }

    /// Bytes of decoded samples held, voices still fading included.
    pub fn loaded_bytes(&self) -> usize {
        let live = self.loaded.iter().flatten().map(|p| p.bytes());
        live.chain(self.dying.iter().map(|(_, p)| p.bytes())).sum()
    }

    pub fn play(
        &mut self,
        at: u64,
        sound: SoundId,
        params: PlayParams,
    ) -> Result<VoiceId, AudioError> {
        let def = self.def(sound)?;
        let looped = params.looped.unwrap_or(def.loop_points.is_some());
        let (bus, gain) = (def.bus as u16, def.gain * params.gain);
        let src = self.src(sound, looped)?;
        let voice = self.voice_id();
        let play = Play {
            voice,
            sound: sound.0,
            src,
            bus,
            gain,
            pan: params.pan,
            looped,
            fade_in: self.frames(params.fade_in_s),
            curve: Curve::Linear,
            priority: params.priority,
        };
        self.send(at, Op::Play(play))?;
        Ok(voice)
    }

    pub fn stop(&mut self, at: u64, voice: VoiceId, fade_s: f32) -> Result<(), AudioError> {
        let fade = self.frames(fade_s);
        self.send(at, Op::Stop { voice, fade })
    }

    pub fn set_voice(
        &mut self,
        at: u64,
        voice: VoiceId,
        gain: f32,
        pan: f32,
        ramp_s: f32,
    ) -> Result<(), AudioError> {
        let ramp = self.frames(ramp_s);
        self.send(
            at,
            Op::SetVoice {
                voice,
                gain,
                pan,
                ramp,
            },
        )
    }

    pub fn set_bus(
        &mut self,
        at: u64,
        bus: usize,
        gain_db: f32,
        ramp_s: f32,
    ) -> Result<(), AudioError> {
        if bus >= self.settings.buses.len() {
            return Err(AudioError::Settings(format!("there is no bus {bus}")));
        }
        let ramp = self.frames(ramp_s);
        let gain = db_to_gain(gain_db);
        self.send(
            at,
            Op::SetBus {
                bus: bus as u16,
                gain,
                ramp,
            },
        )
    }

    pub fn set_master(&mut self, at: u64, gain_db: f32, ramp_s: f32) -> Result<(), AudioError> {
        let ramp = self.frames(ramp_s);
        self.send(
            at,
            Op::SetMaster {
                gain: db_to_gain(gain_db),
                ramp,
            },
        )
    }

    /// Makes `sound` the music, looping unless `looped` says otherwise. The
    /// same sound already playing is left alone.
    pub fn music(
        &mut self,
        at: u64,
        sound: SoundId,
        fades: MusicFades,
        looped: Option<bool>,
    ) -> Result<VoiceId, AudioError> {
        let def = self.def(sound)?;
        let music = &self.settings.music;
        let fade_in = fades.fade_in_s.or(def.enter_s).unwrap_or(music.fade_in_s);
        let leaving = self.music.and_then(|m| self.sounds[m as usize].leave_s);
        let fade_out = fades.fade_out_s.or(leaving).unwrap_or(music.fade_out_s);
        let curve = fades.curve.unwrap_or(music.curve);
        let gain = def.gain;
        let src = self.src(sound, looped.unwrap_or(true))?;
        let play = self.music_play(
            sound,
            src,
            gain,
            self.frames(fade_in),
            curve,
            looped.unwrap_or(true),
        );
        let voice = play.voice;
        let fade_out = self.frames(fade_out);
        self.send(at, Op::Music { play, fade_out })?;
        self.music = Some(sound.0);
        Ok(voice)
    }

    /// Plays `sound` once over the music, which drops and returns from where
    /// it was when the stinger ends.
    pub fn stinger(&mut self, at: u64, sound: SoundId) -> Result<VoiceId, AudioError> {
        let gain = self.def(sound)?.gain;
        let src = self.src(sound, false)?;
        let play = self.music_play(sound, src, gain, 0, self.settings.music.curve, false);
        let voice = play.voice;
        self.send(at, Op::Stinger(play))?;
        Ok(voice)
    }

    pub fn stop_music(&mut self, at: u64, fade_s: Option<f32>) -> Result<(), AudioError> {
        let leaving = self.music.and_then(|m| self.sounds[m as usize].leave_s);
        let fade = fade_s.or(leaving).unwrap_or(self.settings.music.fade_out_s);
        let fade = self.frames(fade);
        self.send(at, Op::MusicStop { fade })?;
        self.music = None;
        Ok(())
    }

    /// Fades everything out and freezes every voice where it is, for the app
    /// going to the background.
    pub fn pause(&mut self, at: u64) -> Result<(), AudioError> {
        self.send(at, Op::Pause)
    }

    pub fn resume(&mut self, at: u64) -> Result<(), AudioError> {
        self.send(at, Op::Resume)
    }

    /// Decodes every manually fed stream until its buffer is full. Returns
    /// the first decoding error; that stream ends where it failed.
    pub fn pump(&mut self) -> Result<(), AudioError> {
        let mut first = Ok(());
        self.feeders.retain_mut(|f| match f.pump() {
            Ok(Pump::Abandoned) => false,
            Ok(_) => true,
            Err(e) => {
                if first.is_ok() {
                    first = Err(e);
                }
                true
            }
        });
        first
    }

    /// Frees what the mixer has let go of. Call once per simulation step.
    pub fn collect(&mut self) {
        while let Ok(reader) = self.retired.pop() {
            drop(reader);
        }
        self.dying.retain(|(_, pcm)| Arc::strong_count(pcm) > 1);
    }

    /// Decoding errors of threaded streams, oldest first, emptied by reading.
    pub fn take_stream_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.stream_errors.lock().expect("never poisoned"))
    }

    /// Bytes held by open streams besides their compressed sound.
    pub fn stream_bytes(&self) -> usize {
        self.feeders.iter().map(Feeder::buffer_bytes).sum()
    }

    fn def(&self, sound: SoundId) -> Result<&SoundDef, AudioError> {
        self.sounds
            .get(sound.0 as usize)
            .ok_or_else(|| AudioError::UnknownSound(format!("sound {}", sound.0)))
    }

    fn voice_id(&mut self) -> VoiceId {
        let id = VoiceId(self.next_voice);
        self.next_voice += 1;
        id
    }

    fn music_play(
        &mut self,
        sound: SoundId,
        src: Src,
        gain: f32,
        fade_in: u32,
        curve: Curve,
        looped: bool,
    ) -> Play {
        Play {
            voice: self.voice_id(),
            sound: sound.0,
            src,
            bus: self.sounds[sound.0 as usize].bus as u16,
            gain,
            pan: 0.0,
            looped,
            fade_in,
            curve,
            priority: u8::MAX,
        }
    }

    fn src(&mut self, sound: SoundId, looped: bool) -> Result<Src, AudioError> {
        let def = self.def(sound)?;
        if !def.stream {
            return self.loaded[sound.0 as usize]
                .clone()
                .map(Src::Static)
                .ok_or_else(|| AudioError::NotLoaded(def.name.clone()));
        }
        let loop_points = looped.then(|| {
            def.loop_points.unwrap_or(LoopPoints {
                start: 0,
                end: def.frames,
            })
        });
        let (mut feeder, reader) = Feeder::open(
            def.data.clone(),
            def.container,
            loop_points,
            self.settings.sample_rate,
            &self.settings.stream,
        )
        .map_err(|e| AudioError::Decode(format!("{}: {e}", def.name)))?;
        match self.feeding {
            Feeding::Manual => {
                feeder.pump()?;
                self.feeders.push(feeder);
            }
            Feeding::Thread => self.feed_on_thread(feeder),
        }
        Ok(Src::Stream(reader))
    }

    fn feed_on_thread(&mut self, feeder: Feeder) {
        let feeder = match &self.thread {
            Some((tx, _)) => match tx.send(feeder) {
                Ok(()) => return,
                Err(mpsc::SendError(feeder)) => feeder,
            },
            None => feeder,
        };
        let (tx, rx) = mpsc::channel::<Feeder>();
        let poll = Duration::from_millis(u64::from(self.settings.stream.poll_ms.max(1)));
        let errors = self.stream_errors.clone();
        let handle = thread::Builder::new()
            .name("sariema-audio-decode".into())
            .spawn(move || {
                let mut feeders = vec![feeder];
                loop {
                    match rx.try_recv() {
                        Ok(f) => feeders.push(f),
                        Err(TryRecvError::Disconnected) if feeders.is_empty() => return,
                        Err(_) => {}
                    }
                    feeders.retain_mut(|f| match f.pump() {
                        Ok(Pump::Abandoned) => false,
                        Ok(_) => true,
                        Err(e) => {
                            errors.lock().expect("never poisoned").push(e.to_string());
                            true
                        }
                    });
                    thread::sleep(poll);
                }
            })
            .expect("spawning the decoding thread");
        self.thread = Some((tx, handle));
    }

    fn send(&mut self, at: u64, op: Op) -> Result<(), AudioError> {
        self.collect();
        self.commands
            .push(Timed { at, op })
            .map_err(|_| AudioError::QueueFull)
    }
}

fn validate(settings: &MixerSettings, sounds: &[SoundDef]) -> Result<Vec<Duck>, AudioError> {
    let bad = |why: String| Err(AudioError::Settings(why));
    if settings.sample_rate == 0 || settings.block_frames == 0 || settings.commands == 0 {
        return bad("sample_rate, block_frames and commands must be positive".into());
    }
    if settings.buses.is_empty() || settings.buses.len() > usize::from(u16::MAX) {
        return bad(format!(
            "{} buses; between 1 and 65535 are allowed",
            settings.buses.len()
        ));
    }
    for (i, bus) in settings.buses.iter().enumerate() {
        if settings.bus(&bus.name) != Some(i) {
            return bad(format!("two buses are called {}", bus.name));
        }
    }
    let bus = |name: &str| {
        settings
            .bus(name)
            .map(|b| b as u16)
            .ok_or_else(|| AudioError::Settings(format!("there is no bus called {name}")))
    };
    let mut ducks = Vec::new();
    for rule in &settings.ducking {
        let targets = rule
            .targets
            .iter()
            .map(|t| bus(t))
            .collect::<Result<_, _>>()?;
        ducks.push(Duck::new(
            rule,
            bus(&rule.key)?,
            targets,
            settings.sample_rate,
        ));
    }
    if let Some(s) = sounds.iter().find(|s| s.bus >= settings.buses.len()) {
        return bad(format!(
            "sound {} uses bus {} of {}",
            s.name,
            s.bus,
            settings.buses.len()
        ));
    }
    Ok(ducks)
}
