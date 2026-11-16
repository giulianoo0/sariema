// SPDX-License-Identifier: Apache-2.0

//! `sariema audio`: builds, inspects and checks sound banks, and renders
//! cue lists through the real mixer without a device.

use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, value_parser};
use sariema_audio::offline::{CueList, RenderOptions, render};
use sariema_audio::{
    AudioError, AudioProject, MixerSettings, Warning, WavFormat, check, effective_lufs, wav,
};
use sariema_format::sound::{self, Load, SoundBank};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::output::{Done, Failure, list};

const PROJECT_SCHEMA: &str = include_str!("../../../schemas/project.schema.json");
const AUDIO_SCHEMA: &str = include_str!("../../../schemas/audio.schema.json");
const CUES_SCHEMA: &str = include_str!("../../../schemas/cues.schema.json");

#[derive(Args)]
pub struct AudioArgs {
    #[command(subcommand)]
    command: AudioCommand,
}

#[derive(Subcommand)]
enum AudioCommand {
    /// Decode and measure a project's sounds into a sound bank, reporting the
    /// loudness checks' warnings.
    Build {
        /// Directory holding sariema.json, whose `audio` names the sound description.
        project: PathBuf,
        /// Bank to write, conventionally `.srms`.
        #[arg(short, long)]
        output: PathBuf,
        /// Fail with E_AUDIO_CHECK, writing nothing, when a check warns.
        #[arg(long)]
        strict: bool,
    },
    /// Show a bank's buses, sounds, measured and effective loudness, and mixes.
    Inspect { bank: PathBuf },
    /// Run the loudness checks on a project's sounds without writing a bank.
    Check {
        project: PathBuf,
        #[arg(long)]
        strict: bool,
    },
    /// Play a cue list (schemas/cues.schema.json) through the mixer into a
    /// 24-bit WAV and a JSON timeline of bus levels.
    Render {
        bank: PathBuf,
        #[arg(long)]
        cues: PathBuf,
        /// WAV to write.
        #[arg(short, long)]
        output: PathBuf,
        /// Timeline to write. Default: the output with `.timeline.json`.
        #[arg(long)]
        timeline: Option<PathBuf>,
        /// Frames per mixer call, like a device buffer; the output does not
        /// depend on it.
        #[arg(long, default_value_t = 480, value_parser = value_parser!(u32).range(1..))]
        buffer: u32,
        #[arg(long, default_value_t = 48_000, value_parser = value_parser!(u32).range(8_000..=192_000))]
        rate: u32,
        /// Seconds per timeline window.
        #[arg(long, default_value_t = 0.1, value_parser = positive)]
        window: f32,
    },
}

fn positive(s: &str) -> Result<f32, String> {
    match s.parse::<f32>() {
        Ok(v) if v > 0.0 && v.is_finite() => Ok(v),
        Ok(_) => Err("must be positive".into()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn run(args: &AudioArgs) -> Result<Done, Failure> {
    match &args.command {
        AudioCommand::Build {
            project,
            output,
            strict,
        } => build(project, output, *strict),
        AudioCommand::Inspect { bank } => inspect(bank),
        AudioCommand::Check { project, strict } => check_project(project, *strict),
        AudioCommand::Render {
            bank,
            cues,
            output,
            timeline,
            buffer,
            rate,
            window,
        } => render_cues(
            bank,
            cues,
            output,
            timeline.as_deref(),
            *buffer,
            *rate,
            *window,
        ),
    }
}

impl From<AudioError> for Failure {
    fn from(e: AudioError) -> Failure {
        Failure::new(e.code(), e.to_string())
    }
}

/// `text` checked against `schema`, then deserialized.
fn validated<T: for<'de> Deserialize<'de>>(path: &Path, schema: &str) -> Result<T, Failure> {
    let text = fs::read_to_string(path).map_err(|e| Failure::io(path, e))?;
    let schema_error = |why: String| Failure::new("E_SCHEMA", format!("{}: {why}", path.display()));
    let value: Value = serde_json::from_str(&text).map_err(|e| schema_error(e.to_string()))?;
    let schema: Value = serde_json::from_str(schema).expect("embedded schemas are JSON");
    jsonschema::validate(&schema, &value).map_err(|e| {
        let at = e.instance_path().to_string();
        schema_error(if at.is_empty() {
            e.to_string()
        } else {
            format!("{at}: {e}")
        })
    })?;
    serde_json::from_value(value).map_err(|e| schema_error(e.to_string()))
}

#[derive(Deserialize)]
struct ProjectAudio {
    audio: Option<PathBuf>,
}

fn load_project(dir: &Path) -> Result<(AudioProject, PathBuf), Failure> {
    let file = dir.join("sariema.json");
    let project: ProjectAudio = validated(&file, PROJECT_SCHEMA)?;
    let Some(audio) = project.audio else {
        return Err(Failure::new(
            "E_SCHEMA",
            format!(
                "{}: has no audio key naming the sound description",
                file.display()
            ),
        ));
    };
    let description = dir.join(audio);
    let parsed: AudioProject = validated(&description, AUDIO_SCHEMA)?;
    let base = description.parent().unwrap_or(dir).to_path_buf();
    Ok((parsed, base))
}

fn warnings_json(warnings: &[Warning]) -> Value {
    serde_json::to_value(warnings).expect("warnings serialize")
}

fn warnings_text(warnings: &[Warning]) -> String {
    warnings
        .iter()
        .map(|w| format!("\nwarning[{}]: {}", w.code, w.message))
        .collect()
}

fn strict_failure(warnings: &[Warning]) -> Failure {
    let messages: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
    Failure::new("E_AUDIO_CHECK", messages.join("; "))
}

fn build(dir: &Path, output: &Path, strict: bool) -> Result<Done, Failure> {
    let (project, base) = load_project(dir)?;
    let bank = sariema_audio::build(&project, &base)?;
    let warnings = check(&bank, &project.checks);
    if strict && !warnings.is_empty() {
        return Err(strict_failure(&warnings));
    }
    let bytes = sound::encode(&bank);
    fs::write(output, &bytes).map_err(|e| Failure::io(output, e))?;
    Ok(Done {
        json: json!({
            "output": output.display().to_string(),
            "bytes": bytes.len(),
            "sounds": bank.sounds.len(),
            "warnings": warnings_json(&warnings),
        }),
        text: format!(
            "built {} sounds into {}: {} bytes{}",
            bank.sounds.len(),
            output.display(),
            bytes.len(),
            warnings_text(&warnings)
        ),
    })
}

fn check_project(dir: &Path, strict: bool) -> Result<Done, Failure> {
    let (project, base) = load_project(dir)?;
    let bank = sariema_audio::build(&project, &base)?;
    let warnings = check(&bank, &project.checks);
    if strict && !warnings.is_empty() {
        return Err(strict_failure(&warnings));
    }
    let sounds: Vec<Value> = bank.sounds.iter().map(|s| sound_json(&bank, s)).collect();
    Ok(Done {
        json: json!({ "sounds": sounds, "warnings": warnings_json(&warnings) }),
        text: if warnings.is_empty() {
            format!("{} sounds, no warnings", bank.sounds.len())
        } else {
            format!("{} sounds{}", bank.sounds.len(), warnings_text(&warnings))
        },
    })
}

fn read_bank(path: &Path) -> Result<SoundBank, Failure> {
    let bytes = fs::read(path).map_err(|e| Failure::io(path, e))?;
    sound::decode(&bytes).map_err(|e| Failure::new(e.code(), format!("{}: {e}", path.display())))
}

fn sound_json(bank: &SoundBank, s: &sound::Sound) -> Value {
    json!({
        "name": s.name,
        "bus": bank.buses[s.bus as usize].name,
        "container": format!("{:?}", s.container).to_lowercase(),
        "sample_rate": s.sample_rate,
        "channels": s.channels,
        "seconds": s.frames as f64 / f64::from(s.sample_rate),
        "loop": s.loop_points.map(|l| json!({ "start": l.start, "end": l.end })),
        "stream": s.stream,
        "on_demand": s.load == Load::OnDemand,
        "integrated_lufs": s.loudness.integrated_lufs,
        "effective_lufs": effective_lufs(bank, s),
        "peak_dbfs": s.loudness.peak_dbfs,
        "seam_step_db": s.loudness.seam_step_db,
        "bytes": s.data.len(),
    })
}

fn inspect(path: &Path) -> Result<Done, Failure> {
    let bank = read_bank(path)?;
    let sounds: Vec<Value> = bank.sounds.iter().map(|s| sound_json(&bank, s)).collect();
    let mixes: serde_json::Map<String, Value> = bank
        .mixes
        .iter()
        .map(|m| {
            let names: Vec<&str> = m
                .sounds
                .iter()
                .map(|&i| bank.sounds[i as usize].name.as_str())
                .collect();
            (m.name.clone(), json!(names))
        })
        .collect();
    let bus_names: Vec<&str> = bank.buses.iter().map(|b| b.name.as_str()).collect();
    let lines: String = bank
        .sounds
        .iter()
        .map(|s| {
            format!(
                "\n{} on {}: {:.1} s, {:.1} LUFS measured, {:.1} effective{}{}",
                s.name,
                bank.buses[s.bus as usize].name,
                s.frames as f64 / f64::from(s.sample_rate),
                s.loudness.integrated_lufs,
                effective_lufs(&bank, s),
                if s.stream { ", streamed" } else { "" },
                if s.loop_points.is_some() {
                    ", loops"
                } else {
                    ""
                },
            )
        })
        .collect();
    Ok(Done {
        json: json!({
            "buses": bank.buses.iter().map(|b| json!({
                "name": b.name, "gain_db": b.gain_db, "target_lufs": b.target_lufs
            })).collect::<Vec<_>>(),
            "sounds": sounds,
            "mixes": mixes,
        }),
        text: format!("buses: {}{lines}", list(&bus_names)),
    })
}

fn render_cues(
    bank_path: &Path,
    cues_path: &Path,
    output: &Path,
    timeline_path: Option<&Path>,
    buffer: u32,
    rate: u32,
    window: f32,
) -> Result<Done, Failure> {
    let bank = read_bank(bank_path)?;
    let cues: CueList = validated(cues_path, CUES_SCHEMA)?;
    let settings = MixerSettings {
        sample_rate: rate,
        ..MixerSettings::default()
    };
    let options = RenderOptions {
        buffer_frames: buffer as usize,
        window_s: window,
    };
    let rendered = render(bank, settings, &cues, options)?;
    let bytes = wav(&rendered.samples, 2, rate, WavFormat::Pcm24);
    fs::write(output, bytes).map_err(|e| Failure::io(output, e))?;
    let timeline_path = timeline_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| output.with_extension("timeline.json"));
    let t = &rendered.timeline;
    let timeline = serde_json::to_vec_pretty(t).expect("the timeline serializes");
    fs::write(&timeline_path, timeline).map_err(|e| Failure::io(&timeline_path, e))?;
    let bus_lufs: serde_json::Map<String, Value> = t
        .buses
        .iter()
        .zip(&t.bus_lufs)
        .map(|(name, lufs)| (name.clone(), json!(lufs)))
        .collect();
    let per_bus: Vec<String> = t
        .buses
        .iter()
        .zip(&t.bus_lufs)
        .map(|(name, lufs)| format!("{name} {lufs:.1}"))
        .collect();
    Ok(Done {
        json: json!({
            "output": output.display().to_string(),
            "timeline": timeline_path.display().to_string(),
            "seconds": cues.duration_s,
            "master_lufs": t.master_lufs,
            "master_peak_dbfs": t.master_peak_dbfs,
            "bus_lufs": bus_lufs,
            "stats": t.stats,
        }),
        text: format!(
            "rendered {:.1} s into {} ({:.1} LUFS, peak {:.1} dBFS; {}); timeline in {}",
            cues.duration_s,
            output.display(),
            t.master_lufs,
            t.master_peak_dbfs,
            list(&per_bus),
            timeline_path.display()
        ),
    })
}
