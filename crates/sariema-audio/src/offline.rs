// SPDX-License-Identifier: Apache-2.0

//! Rendering a timed list of commands through the real mixer, without a
//! device, into samples and a timeline of what each bus did.

use std::collections::BTreeMap;

use sariema_format::sound::SoundBank;
use serde::{Deserialize, Serialize};

use crate::AudioError;
use crate::bank::runtime;
use crate::command::VoiceId;
use crate::controller::{Audio, Feeding, MusicFades, PlayParams};
use crate::loudness::LoudnessMeter;
use crate::mixer::{MixStats, Tap};
use crate::ramp::{db_to_gain, gain_to_db};
use crate::settings::MixerSettings;

/// `schemas/cues.schema.json`.
#[derive(Clone, Debug, Deserialize)]
pub struct CueList {
    pub duration_s: f32,
    pub cues: Vec<Cue>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Cue {
    /// Seconds on the mixer clock.
    pub t: f32,
    #[serde(flatten)]
    pub action: Action,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Action {
    Play {
        sound: String,
        /// Name later cues use to stop or change this voice.
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        gain_db: f32,
        #[serde(default)]
        pan: f32,
        #[serde(default, rename = "loop")]
        looped: Option<bool>,
        #[serde(default)]
        fade_in_s: f32,
        #[serde(default = "default_priority")]
        priority: u8,
    },
    Stop {
        id: String,
        #[serde(default)]
        fade_s: f32,
    },
    Voice {
        id: String,
        #[serde(default)]
        gain_db: f32,
        #[serde(default)]
        pan: f32,
        #[serde(default)]
        ramp_s: f32,
    },
    Music {
        sound: String,
        #[serde(default)]
        fade_in_s: Option<f32>,
        #[serde(default)]
        fade_out_s: Option<f32>,
        #[serde(default, rename = "loop")]
        looped: Option<bool>,
    },
    Stinger {
        sound: String,
    },
    MusicStop {
        #[serde(default)]
        fade_s: Option<f32>,
    },
    Bus {
        bus: String,
        gain_db: f32,
        #[serde(default)]
        ramp_s: f32,
    },
    Master {
        gain_db: f32,
        #[serde(default)]
        ramp_s: f32,
    },
    Pause,
    Resume,
    Preload {
        sound: String,
    },
    Unload {
        sound: String,
    },
}

fn default_priority() -> u8 {
    PlayParams::default().priority
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOptions {
    /// Frames asked of the mixer at a time, like a device callback. Default 480.
    pub buffer_frames: usize,
    /// Seconds per timeline window. Default 0.1.
    pub window_s: f32,
}

impl Default for RenderOptions {
    fn default() -> RenderOptions {
        RenderOptions {
            buffer_frames: 480,
            window_s: 0.1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Timeline {
    pub sample_rate: u32,
    pub window_s: f32,
    pub buses: Vec<String>,
    /// Integrated loudness of each bus over the render, after its gain and
    /// ducking.
    pub bus_lufs: Vec<f32>,
    pub master_lufs: f32,
    pub master_peak_dbfs: f32,
    pub windows: Vec<Window>,
    pub events: Vec<Event>,
    pub stats: Stats,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Window {
    pub t: f32,
    /// RMS per bus, in dBFS.
    pub rms_db: Vec<f32>,
    pub master_rms_db: f32,
    pub master_peak_db: f32,
    /// Ducking per bus at the end of the window, in dB.
    pub duck_db: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Event {
    pub t: f32,
    pub what: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Stats {
    pub underruns: u64,
    pub steals: u64,
    pub refused: u64,
    pub late: u64,
    pub cuts: u64,
}

pub struct Render {
    /// Interleaved stereo at `timeline.sample_rate`.
    pub samples: Vec<f32>,
    pub timeline: Timeline,
}

pub fn render(
    bank: SoundBank,
    settings: MixerSettings,
    cues: &CueList,
    options: RenderOptions,
) -> Result<Render, AudioError> {
    let (settings, sounds) = runtime(bank, settings);
    let rate = settings.sample_rate;
    let buses: Vec<String> = settings.buses.iter().map(|b| b.name.clone()).collect();
    let (mut audio, mut mixer) = Audio::new(settings, sounds, Feeding::Manual)?;
    let total = audio.frames(cues.duration_s) as usize;
    let mut order: Vec<&Cue> = cues.cues.iter().collect();
    order.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut next = 0;
    let mut ids = BTreeMap::new();
    let mut events = Vec::new();
    let window = audio.frames(options.window_s).max(1) as usize;
    let mut meters = Meters::new(rate, buses.len(), window);
    let mut samples = vec![0.0; 2 * total];
    let buffer = options.buffer_frames.max(1);
    let mut pos = 0;
    let mut last = MixStats::default();
    while pos < total {
        let n = buffer.min(total - pos);
        while next < order.len() && (audio.frames(order[next].t) as usize) < pos + n {
            let cue = order[next];
            apply(&mut audio, cue, &mut ids, &buses)?;
            events.push(Event {
                t: cue.t,
                what: describe(&cue.action),
            });
            next += 1;
        }
        audio.pump()?;
        mixer.render_tapped(&mut samples[2 * pos..2 * (pos + n)], &mut |tap| {
            meters.tap(tap)
        });
        audio.collect();
        let now = mixer.stats();
        note(&mut events, &last, &now, (pos + n) as f32 / rate as f32);
        last = now;
        pos += n;
    }
    let stats = mixer.stats();
    let timeline = Timeline {
        sample_rate: rate,
        window_s: options.window_s,
        bus_lufs: meters.buses.iter().map(LoudnessMeter::integrated).collect(),
        master_lufs: meters.master.integrated(),
        master_peak_dbfs: meters.master.peak_dbfs(),
        windows: meters.windows(total),
        buses,
        events,
        stats: Stats {
            underruns: stats.underruns,
            steals: stats.steals,
            refused: stats.refused,
            late: stats.late,
            cuts: stats.cuts,
        },
    };
    Ok(Render { samples, timeline })
}

fn apply(
    audio: &mut Audio,
    cue: &Cue,
    ids: &mut BTreeMap<String, VoiceId>,
    buses: &[String],
) -> Result<(), AudioError> {
    let at = u64::from(audio.frames(cue.t));
    let voice = |ids: &BTreeMap<String, VoiceId>, id: &str| {
        ids.get(id)
            .copied()
            .ok_or_else(|| AudioError::UnknownSound(format!("no voice was started with id {id}")))
    };
    match &cue.action {
        Action::Play {
            sound,
            id,
            gain_db,
            pan,
            looped,
            fade_in_s,
            priority,
        } => {
            let sound = audio.sound(sound)?;
            let params = PlayParams {
                gain: db_to_gain(*gain_db),
                pan: *pan,
                looped: *looped,
                fade_in_s: *fade_in_s,
                priority: *priority,
            };
            let v = audio.play(at, sound, params)?;
            if let Some(id) = id {
                ids.insert(id.clone(), v);
            }
        }
        Action::Stop { id, fade_s } => audio.stop(at, voice(ids, id)?, *fade_s)?,
        Action::Voice {
            id,
            gain_db,
            pan,
            ramp_s,
        } => audio.set_voice(at, voice(ids, id)?, db_to_gain(*gain_db), *pan, *ramp_s)?,
        Action::Music {
            sound,
            fade_in_s,
            fade_out_s,
            looped,
        } => {
            let sound = audio.sound(sound)?;
            let fades = MusicFades {
                fade_in_s: *fade_in_s,
                fade_out_s: *fade_out_s,
                curve: None,
            };
            audio.music(at, sound, fades, *looped)?;
        }
        Action::Stinger { sound } => {
            let sound = audio.sound(sound)?;
            audio.stinger(at, sound)?;
        }
        Action::MusicStop { fade_s } => audio.stop_music(at, *fade_s)?,
        Action::Bus {
            bus,
            gain_db,
            ramp_s,
        } => {
            let index = buses
                .iter()
                .position(|b| b == bus)
                .ok_or_else(|| AudioError::Settings(format!("there is no bus called {bus}")))?;
            audio.set_bus(at, index, *gain_db, *ramp_s)?;
        }
        Action::Master { gain_db, ramp_s } => audio.set_master(at, *gain_db, *ramp_s)?,
        Action::Pause => audio.pause(at)?,
        Action::Resume => audio.resume(at)?,
        Action::Preload { sound } => {
            let sound = audio.sound(sound)?;
            audio.preload(sound)?;
        }
        Action::Unload { sound } => {
            let sound = audio.sound(sound)?;
            audio.unload(sound);
        }
    }
    Ok(())
}

fn describe(action: &Action) -> String {
    match action {
        Action::Play { sound, .. } => format!("play {sound}"),
        Action::Stop { id, .. } => format!("stop {id}"),
        Action::Voice { id, .. } => format!("change {id}"),
        Action::Music { sound, .. } => format!("music {sound}"),
        Action::Stinger { sound } => format!("stinger {sound}"),
        Action::MusicStop { .. } => "music stop".into(),
        Action::Bus { bus, gain_db, .. } => format!("bus {bus} to {gain_db} dB"),
        Action::Master { gain_db, .. } => format!("master to {gain_db} dB"),
        Action::Pause => "pause".into(),
        Action::Resume => "resume".into(),
        Action::Preload { sound } => format!("preload {sound}"),
        Action::Unload { sound } => format!("unload {sound}"),
    }
}

fn note(events: &mut Vec<Event>, before: &MixStats, after: &MixStats, t: f32) {
    let changes = [
        (after.steals - before.steals, "voice stolen"),
        (after.refused - before.refused, "play refused"),
        (after.underruns - before.underruns, "stream underrun"),
        (after.cuts - before.cuts, "fading voice cut for music"),
        (after.late - before.late, "command late"),
    ];
    for (count, what) in changes {
        if count > 0 {
            events.push(Event {
                t,
                what: format!("{count} × {what}"),
            });
        }
    }
}

struct WindowAcc {
    sum_sq: Vec<f64>,
    master_sq: f64,
    master_peak: f32,
    duck_db: Vec<f32>,
}

struct Meters {
    rate: u32,
    window: usize,
    buses: Vec<LoudnessMeter>,
    master: LoudnessMeter,
    windows: Vec<WindowAcc>,
    block_start: usize,
}

impl Meters {
    fn new(rate: u32, buses: usize, window: usize) -> Meters {
        Meters {
            rate,
            window,
            buses: (0..buses).map(|_| LoudnessMeter::new(rate, 2)).collect(),
            master: LoudnessMeter::new(rate, 2),
            windows: Vec::new(),
            block_start: 0,
        }
    }

    fn tap(&mut self, tap: Tap<'_>) {
        let frames = tap.samples.len() / 2;
        let last = (self.block_start + frames).saturating_sub(1) / self.window;
        let count = self.buses.len();
        while self.windows.len() <= last {
            self.windows.push(WindowAcc {
                sum_sq: vec![0.0; count],
                master_sq: 0.0,
                master_peak: 0.0,
                duck_db: vec![0.0; count],
            });
        }
        for (k, frame) in tap.samples.as_chunks::<2>().0.iter().enumerate() {
            let w = &mut self.windows[(self.block_start + k) / self.window];
            let sq = f64::from(frame[0]).powi(2) + f64::from(frame[1]).powi(2);
            match tap.bus {
                Some(b) => w.sum_sq[b] += sq,
                None => {
                    w.master_sq += sq;
                    w.master_peak = w.master_peak.max(frame[0].abs()).max(frame[1].abs());
                }
            }
        }
        match tap.bus {
            Some(b) => {
                self.buses[b].push(tap.samples);
                self.windows[last].duck_db[b] = tap.duck_db;
            }
            None => {
                self.master.push(tap.samples);
                self.block_start += frames;
            }
        }
    }

    /// Windows covering the first `total` frames; the mixer may have mixed
    /// a little past them to finish its last block.
    fn windows(&self, total: usize) -> Vec<Window> {
        let rms_db = |sum_sq: f64, frames: usize| {
            gain_to_db((sum_sq / (2 * frames.max(1)) as f64).sqrt() as f32)
        };
        self.windows
            .iter()
            .take(total.div_ceil(self.window))
            .enumerate()
            .map(|(i, w)| {
                let frames = self.window.min(total - i * self.window);
                Window {
                    t: (i * self.window) as f32 / self.rate as f32,
                    rms_db: w.sum_sq.iter().map(|&s| rms_db(s, frames)).collect(),
                    master_rms_db: rms_db(w.master_sq, frames),
                    master_peak_db: gain_to_db(w.master_peak),
                    duck_db: w.duck_db.clone(),
                }
            })
            .collect()
    }
}
