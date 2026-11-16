// SPDX-License-Identifier: Apache-2.0

//! From a packaged [`SoundBank`] to what the mixer plays.

use std::sync::Arc;

use sariema_format::sound::{Sound, SoundBank};

use crate::controller::SoundDef;
use crate::ramp::db_to_gain;
use crate::settings::{BusSettings, MixerSettings};

/// Gain, in dB, a sound plays at before its bus: its own gain plus what
/// brings it to its bus's loudness target.
pub fn sound_gain_db(bank: &SoundBank, sound: &Sound) -> f32 {
    let bus = &bank.buses[sound.bus as usize];
    let measured = sound.loudness.integrated_lufs;
    let normalise = match bus.target_lufs {
        Some(target) if measured.is_finite() => target - measured,
        _ => 0.0,
    };
    sound.gain_db + normalise
}

/// Loudness, in LUFS, the sound reaches at the master when played at
/// unity: measured level, sound gain, normalisation and bus gain.
pub fn effective_lufs(bank: &SoundBank, sound: &Sound) -> f32 {
    let bus = &bank.buses[sound.bus as usize];
    sound.loudness.integrated_lufs + sound_gain_db(bank, sound) + bus.gain_db
}

/// The bank's buses replace `base`'s; a ducking rule whose key bus the bank
/// lacks is left out, and targets it lacks are skipped.
pub fn runtime(bank: SoundBank, base: MixerSettings) -> (MixerSettings, Vec<SoundDef>) {
    let mut settings = base;
    settings.buses = bank
        .buses
        .iter()
        .map(|b| BusSettings {
            name: b.name.clone(),
            gain_db: b.gain_db,
        })
        .collect();
    let has = |name: &str| bank.buses.iter().any(|b| b.name == name);
    settings.ducking.retain(|rule| has(&rule.key));
    for rule in &mut settings.ducking {
        rule.targets.retain(|t| has(t));
    }
    let gains: Vec<f32> = bank
        .sounds
        .iter()
        .map(|s| sound_gain_db(&bank, s))
        .collect();
    let sounds = bank
        .sounds
        .into_iter()
        .zip(gains)
        .map(|(s, gain_db)| SoundDef {
            name: s.name,
            bus: s.bus as usize,
            gain: db_to_gain(gain_db),
            container: s.container,
            data: Arc::from(s.data),
            loop_points: s.loop_points,
            frames: s.frames,
            stream: s.stream,
            load: s.load,
            enter_s: s.enter_s,
            leave_s: s.leave_s,
        })
        .collect();
    (settings, sounds)
}
