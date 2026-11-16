// SPDX-License-Identifier: Apache-2.0

//! Packaging: a game's sound description (`schemas/audio.schema.json`) and
//! its files become a [`SoundBank`], measured on the way.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sariema_format::sound::{Bus, Load, LoopPoints, Loudness, Mix, Sound, SoundBank};
use serde::Deserialize;

use crate::AudioError;
use crate::check::CheckSettings;
use crate::decode::{container_of, decode_all};
use crate::loudness::{integrated_lufs, seam_step_db};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioProject {
    /// Directory the sound files are relative to, from the project
    /// directory. Default: the project directory.
    #[serde(default)]
    pub root: Option<PathBuf>,
    /// Default: [`default_buses`].
    #[serde(default)]
    pub buses: Option<Vec<BusDesc>>,
    pub sounds: BTreeMap<String, SoundDesc>,
    #[serde(default)]
    pub mixes: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub checks: CheckSettings,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusDesc {
    pub name: String,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub target_lufs: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundDesc {
    pub file: PathBuf,
    pub bus: String,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub on_demand: bool,
    /// `true` loops the whole sound; a region loops `[start, end)` frames.
    #[serde(default, rename = "loop")]
    pub looping: Option<LoopDesc>,
    #[serde(default)]
    pub enter_s: Option<f32>,
    #[serde(default)]
    pub leave_s: Option<f32>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum LoopDesc {
    Whole(bool),
    Region { start: u64, end: u64 },
}

/// Music, voice, effects and ambience, each brought to a loudness that
/// leaves voice clear over music and ambience under it: −24, −21, −24 and
/// −30 LUFS.
pub fn default_buses() -> Vec<BusDesc> {
    [
        ("music", -24.0),
        ("voice", -21.0),
        ("effects", -24.0),
        ("ambience", -30.0),
    ]
    .into_iter()
    .map(|(name, target)| BusDesc {
        name: name.into(),
        gain_db: 0.0,
        target_lufs: Some(target),
    })
    .collect()
}

/// Reads, decodes and measures every sound, relative to `dir`.
pub fn build(project: &AudioProject, dir: &Path) -> Result<SoundBank, AudioError> {
    let root = dir.join(project.root.as_deref().unwrap_or(Path::new("")));
    let buses: Vec<Bus> = project
        .buses
        .clone()
        .unwrap_or_else(default_buses)
        .into_iter()
        .map(|b| Bus {
            name: b.name,
            gain_db: b.gain_db,
            target_lufs: b.target_lufs,
        })
        .collect();
    let mut sounds = Vec::with_capacity(project.sounds.len());
    for (name, desc) in &project.sounds {
        sounds.push(sound(name, desc, &root, &buses, &project.checks)?);
    }
    let index = |name: &str| {
        project
            .sounds
            .keys()
            .position(|k| k == name)
            .map(|i| i as u32)
            .ok_or_else(|| AudioError::UnknownSound(name.into()))
    };
    let mixes = project
        .mixes
        .iter()
        .map(|(name, members)| {
            let sounds = members.iter().map(|m| index(m)).collect::<Result<_, _>>()?;
            Ok(Mix {
                name: name.clone(),
                sounds,
            })
        })
        .collect::<Result<_, AudioError>>()?;
    Ok(SoundBank {
        buses,
        sounds,
        mixes,
    })
}

fn sound(
    name: &str,
    desc: &SoundDesc,
    root: &Path,
    buses: &[Bus],
    checks: &CheckSettings,
) -> Result<Sound, AudioError> {
    let path = root.join(&desc.file);
    let at = |why: String| AudioError::Decode(format!("{name} ({}): {why}", path.display()));
    let bus = buses
        .iter()
        .position(|b| b.name == desc.bus)
        .ok_or_else(|| {
            AudioError::Settings(format!(
                "{name} uses bus {}, which is not declared",
                desc.bus
            ))
        })?;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let container = container_of(extension).ok_or_else(|| {
        at(format!(
            "files ending in .{extension} are not supported; use .mp3, .ogg or .wav"
        ))
    })?;
    let data = fs::read(&path).map_err(|e| AudioError::Io(format!("{}: {e}", path.display())))?;
    let decoded =
        decode_all(Arc::from(data.as_slice()), container).map_err(|e| at(e.to_string()))?;
    let frames = decoded.frames();
    let channels = usize::from(decoded.channels);
    let loop_points = match desc.looping {
        None | Some(LoopDesc::Whole(false)) => None,
        Some(LoopDesc::Whole(true)) => Some(LoopPoints {
            start: 0,
            end: frames,
        }),
        Some(LoopDesc::Region { start, end }) => {
            if !(start < end && end <= frames) {
                return Err(AudioError::Settings(format!(
                    "{name} loops over {start}..{end} but has {frames} frames"
                )));
            }
            Some(LoopPoints { start, end })
        }
    };
    let (integrated_lufs, peak_dbfs) =
        integrated_lufs(&decoded.samples, channels, decoded.sample_rate);
    let window =
        crate::settings::seconds_to_frames(checks.seam_window_s, decoded.sample_rate) as usize;
    let seam_step_db = loop_points.map(|l| {
        seam_step_db(
            &decoded.samples,
            channels,
            l.start as usize,
            l.end as usize,
            window,
        )
    });
    Ok(Sound {
        name: name.into(),
        bus: bus as u32,
        gain_db: desc.gain_db,
        container,
        sample_rate: decoded.sample_rate,
        channels: decoded.channels,
        frames,
        loop_points,
        stream: desc.stream,
        load: if desc.on_demand {
            Load::OnDemand
        } else {
            Load::Bank
        },
        enter_s: desc.enter_s,
        leave_s: desc.leave_s,
        loudness: Loudness {
            integrated_lufs,
            peak_dbfs,
            seam_step_db,
        },
        data,
    })
}
