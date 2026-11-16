// SPDX-License-Identifier: Apache-2.0

//! Packaging checks that turn a tense, flat mix into a warning: a bus that
//! should sit under the music playing over it, and loops whose seam jumps in
//! level.

use std::collections::BTreeMap;

use sariema_format::sound::SoundBank;
use serde::{Deserialize, Serialize};

use crate::bank::effective_lufs;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CheckSettings {
    /// Bus name to how far, in dB, its sounds may play over the loudest
    /// music of a mix they share. Default ambience 0 and effects 9; the
    /// reference game this engine replaces measured its jingles 6 to 8.5 dB
    /// over the music.
    pub over_music_db: BTreeMap<String, f32>,
    /// Largest level step at a loop seam, in dB. Default 6.
    pub seam_step_db: f32,
    /// Seconds on each side of a seam that are compared. Default 0.2.
    pub seam_window_s: f32,
    /// Bus whose sounds are the music. Default "music".
    pub music_bus: String,
}

impl Default for CheckSettings {
    fn default() -> CheckSettings {
        CheckSettings {
            over_music_db: [("ambience".to_string(), 0.0), ("effects".to_string(), 9.0)].into(),
            seam_step_db: 6.0,
            seam_window_s: 0.2,
            music_bus: "music".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Warning {
    /// `W_AUDIO_OVER_MUSIC` or `W_AUDIO_LOOP_SEAM`.
    pub code: &'static str,
    pub sound: String,
    /// The mix that brings the sound and the music together.
    pub mix: Option<String>,
    pub value_db: f32,
    pub limit_db: f32,
    pub message: String,
}

pub fn check(bank: &SoundBank, settings: &CheckSettings) -> Vec<Warning> {
    let mut warnings = Vec::new();
    let bus_name = |i: u32| bank.buses[i as usize].name.as_str();
    for mix in &bank.mixes {
        let music = mix
            .sounds
            .iter()
            .map(|&i| &bank.sounds[i as usize])
            .filter(|s| bus_name(s.bus) == settings.music_bus)
            .map(|s| (effective_lufs(bank, s), s.name.as_str()))
            .filter(|(l, _)| l.is_finite())
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((music_lufs, music_name)) = music else {
            continue;
        };
        for &i in &mix.sounds {
            let sound = &bank.sounds[i as usize];
            let bus = bus_name(sound.bus);
            let Some(&limit) = settings.over_music_db.get(bus) else {
                continue;
            };
            let over = effective_lufs(bank, sound) - music_lufs;
            if over > limit {
                warnings.push(Warning {
                    code: "W_AUDIO_OVER_MUSIC",
                    sound: sound.name.clone(),
                    mix: Some(mix.name.clone()),
                    value_db: over,
                    limit_db: limit,
                    message: format!(
                        "in mix {}, {} on {bus} plays {over:.1} dB over the music {music_name}, \
                         more than {bus}'s {limit:.1} dB; lower its gain_db or its bus",
                        mix.name, sound.name
                    ),
                });
            }
        }
    }
    for sound in &bank.sounds {
        if let Some(step) = sound.loudness.seam_step_db
            && step > settings.seam_step_db
        {
            warnings.push(Warning {
                code: "W_AUDIO_LOOP_SEAM",
                sound: sound.name.clone(),
                mix: None,
                value_db: step,
                limit_db: settings.seam_step_db,
                message: format!(
                    "{}'s level jumps {step:.1} dB at its loop seam, more than {:.1} dB; \
                     move the loop points or crossfade the seam in the file",
                    sound.name, settings.seam_step_db
                ),
            });
        }
    }
    warnings
}
