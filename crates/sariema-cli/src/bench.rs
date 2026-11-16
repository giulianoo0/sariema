// SPDX-License-Identifier: Apache-2.0

//! The `bench` command: loads a pack offscreen, draws it many times and
//! reports where the time goes, by load stage, frame stage and GPU pass.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use clap::{Args, ValueEnum, value_parser};
use sariema_core::room_camera;
use sariema_render::{
    FrameTimings, PassTime, Quality, RenderConfig, RenderError, Renderer, request_device,
};
use serde_json::{Value, json};

use crate::Tier;
use crate::output::{Done, Failure};

mod alloc;
mod stats;

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Args)]
pub struct BenchArgs {
    pack: PathBuf,
    /// Frames timed in each run.
    #[arg(long, default_value_t = 300, value_parser = value_parser!(u32).range(1..))]
    frames: u32,
    /// Frames drawn untimed before the first run.
    #[arg(long, default_value_t = 60)]
    warmup: u32,
    /// Runs of --frames frames; the GPU passes are timed in one more run.
    #[arg(long, default_value_t = 3, value_parser = value_parser!(u32).range(1..))]
    runs: u32,
    #[arg(long, default_value_t = 1920, value_parser = value_parser!(u32).range(1..))]
    width: u32,
    #[arg(long, default_value_t = 1080, value_parser = value_parser!(u32).range(1..))]
    height: u32,
    #[arg(long, value_enum, default_value_t = Tier::Medium)]
    quality: Tier,
    /// Samples per pixel instead of the tier's.
    #[arg(long, value_parser = value_parser!(u32).range(1..))]
    msaa: Option<u32>,
    /// Neither the sun nor any light casts a shadow, so no shadow map is
    /// drawn or sampled.
    #[arg(long)]
    no_shadows: bool,
    /// No ambient occlusion.
    #[arg(long)]
    no_ao: bool,
}

impl BenchArgs {
    fn settings(&self) -> Quality {
        let tier = self.quality.settings();
        Quality {
            msaa: self.msaa.unwrap_or(tier.msaa),
            ao_samples: if self.no_ao { 0 } else { tier.ao_samples },
            ..tier
        }
    }
}

/// CPU samples, one per timed frame.
#[derive(Default)]
struct Samples {
    frame: Vec<f64>,
    stages: Vec<FrameTimings>,
    allocations: Vec<u64>,
    allocated_bytes: Vec<u64>,
}

impl Samples {
    fn stage(&self, of: fn(&FrameTimings) -> Duration) -> Value {
        let values: Vec<f64> = self.stages.iter().map(|t| ms(of(t))).collect();
        stats::summary(&values)
    }
}

pub fn bench(args: &BenchArgs) -> Result<Done, Failure> {
    let mut last = Instant::now();
    let mut lap = || {
        let now = Instant::now();
        let elapsed = ms(now - last);
        last = now;
        elapsed
    };
    let path = &args.pack;
    let bytes = std::fs::read(path).map_err(|e| Failure::io(path, e))?;
    let read_ms = lap();
    let mut room = sariema_format::decode(&bytes)
        .map_err(|e| Failure::new(e.code(), format!("{}: {e}", path.display())))?;
    if args.no_shadows {
        if let Some(sun) = &mut room.sun {
            sun.casts_shadow = false;
        }
        for light in &mut room.lights {
            light.casts_shadow = false;
        }
    }
    let decode_ms = lap();
    let (adapter, device, queue) = device()?;
    let device_ms = lap();
    let config = RenderConfig {
        width: args.width,
        height: args.height,
        output_format: TARGET_FORMAT,
        quality: args.settings(),
    };
    let mut renderer = Renderer::new(device.clone(), queue, config)?;
    let pipelines_ms = lap();
    renderer.time_cpu_stages(Some(since_start));
    renderer.load_room(&room)?;
    wait(&device)?;
    let upload_ms = lap();

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench target"),
        size: wgpu::Extent3d {
            width: args.width,
            height: args.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let camera = room_camera(&room.camera, args.width as f32 / args.height as f32);
    let mut frame = 0u32;
    let mut draw = |renderer: &mut Renderer| {
        renderer.render(&camera, frame as f32 / 60.0, &view);
        frame += 1;
    };

    lap();
    draw(&mut renderer);
    let first_frame_cpu_ms = lap();
    wait(&device)?;
    let first_frame_wall_ms = first_frame_cpu_ms + lap();
    for _ in 0..args.warmup {
        draw(&mut renderer);
    }
    wait(&device)?;

    let mut samples = Samples::default();
    let mut walls = Vec::new();
    for _ in 0..args.runs {
        let run = Instant::now();
        for _ in 0..args.frames {
            let before = allocations_so_far();
            let start = Instant::now();
            draw(&mut renderer);
            let cpu = start.elapsed();
            let after = allocations_so_far();
            samples.frame.push(ms(cpu));
            samples.stages.push(renderer.last_frame_timings());
            if let (Some((count, bytes)), Some((count_after, bytes_after))) = (before, after) {
                samples.allocations.push(count_after - count);
                samples.allocated_bytes.push(bytes_after - bytes);
            }
        }
        wait(&device)?;
        walls.push(ms(run.elapsed()) / args.frames as f64);
    }
    let frame_stats = renderer.last_frame_stats();

    let gpu = if renderer.time_gpu_passes(args.frames) {
        for _ in 0..args.frames {
            draw(&mut renderer);
        }
        gpu_json(&renderer.gpu_pass_times()?)
    } else {
        json!({ "timestamps": "unsupported", "passes": null })
    };

    let effective = renderer.effective_quality();
    let info = adapter.get_info();
    let cpu_median = stats::median(&samples.frame);
    let wall_median = stats::median(&walls);
    let text = format!(
        "{} at {}×{}: {cpu_median:.3} ms CPU and {wall_median:.3} ms wall per frame (medians); \
         {} passes, {} draws, {} bytes uploaded per frame",
        room.name,
        args.width,
        args.height,
        frame_stats.passes,
        frame_stats.draws,
        frame_stats.bytes_uploaded
    );
    Ok(Done {
        json: json!({
            "pack": path.display().to_string(),
            "room": room.name,
            "adapter": { "name": info.name, "backend": format!("{:?}", info.backend) },
            "load_average": load_average(),
            "settings": {
                "width": args.width,
                "height": args.height,
                "quality": args.quality.to_possible_value().map(|v| v.get_name().to_owned()),
                "msaa": effective.msaa,
                "shadows": !args.no_shadows,
                "ao": effective.ao_samples > 0,
                "frames": args.frames,
                "warmup": args.warmup,
                "runs": args.runs,
            },
            "load_ms": {
                "read": read_ms,
                "decode": decode_ms,
                "device": device_ms,
                "pipelines": pipelines_ms,
                "upload": upload_ms,
                "first_frame_cpu": first_frame_cpu_ms,
                "first_frame_wall": first_frame_wall_ms,
            },
            "cpu_ms": {
                "frame": stats::summary(&samples.frame),
                "cull": samples.stage(|t| t.cull),
                "bin": samples.stage(|t| t.bin),
                "record": samples.stage(|t| t.record),
                "post": samples.stage(|t| t.post),
                "finish": samples.stage(|t| t.finish),
                "submit": samples.stage(|t| t.submit),
            },
            "wall_ms_per_frame": { "runs": walls, "median": wall_median },
            "per_frame": {
                "passes": frame_stats.passes,
                "post_passes": frame_stats.post_passes,
                "draws": frame_stats.draws,
                "bytes_uploaded": frame_stats.bytes_uploaded,
                "allocations": counted(&samples.allocations),
                "allocated_bytes": counted(&samples.allocated_bytes),
            },
            "frame_stats": crate::run::frame_stats_json(&frame_stats),
            "shadow_slots_drawn": renderer.shadow_passes_rendered(),
            "gpu": gpu,
        }),
        text,
    })
}

/// A tile-based GPU starts a pass's vertex stage early and runs passes, and
/// frames, side by side, so a pass's own span overstates it. A frame starts
/// at its first pass's start or at the previous frame's end, whichever is
/// later, and a pass costs how far it moves the end of its frame. Gives, per
/// frame, its start to its end; per pass position, the label and the median
/// and 90th percentile of its cost; and the raw spans of the last frame.
fn gpu_json(frames: &[Vec<PassTime>]) -> Value {
    let end = |frame: &[PassTime]| frame.iter().map(|p| p.end_ms).fold(0.0, f64::max);
    let starts: Vec<f64> = (frames.iter().enumerate())
        .map(|(i, f)| {
            let own = f.first().map_or(0.0, |p| p.start_ms);
            i.checked_sub(1).map_or(own, |j| own.max(end(&frames[j])))
        })
        .collect();
    let span: Vec<f64> = (frames.iter().zip(&starts))
        .map(|(f, start)| end(f) - start)
        .collect();
    let cost = |i: usize, k: usize| {
        let before = k.checked_sub(1).map_or(starts[i], |j| frames[i][j].end_ms);
        (frames[i][k].end_ms - before).max(0.0)
    };
    let first = frames.first().map_or(&[][..], Vec::as_slice);
    let passes: Vec<Value> = (first.iter().enumerate())
        .map(|(k, pass)| {
            let durations: Vec<f64> = (0..frames.len())
                .filter(|&i| frames[i].get(k).is_some_and(|p| p.label == pass.label))
                .map(|i| cost(i, k))
                .collect();
            let summary = stats::summary(&durations);
            json!({ "label": pass.label, "median_ms": summary["median"], "p90_ms": summary["p90"] })
        })
        .collect();
    let origin = starts.last().copied().unwrap_or(0.0);
    let last: Vec<Value> = (frames.last().into_iter().flatten())
        .map(|p| {
            let (start, end) = (p.start_ms - origin, p.end_ms - origin);
            json!({ "label": p.label, "start_ms": start, "end_ms": end })
        })
        .collect();
    json!({
        "timestamps": "supported",
        "frame_ms": stats::summary(&span),
        "passes": passes,
        "last_frame": last,
    })
}

fn device() -> Result<(wgpu::Adapter, Arc<wgpu::Device>, Arc<wgpu::Queue>), Failure> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .map_err(|_| RenderError::NoAdapter)?;
    let (device, queue) = request_device(&adapter, "sariema bench")?;
    Ok((adapter, device, queue))
}

fn wait(device: &wgpu::Device) -> Result<(), Failure> {
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map(drop)
        .map_err(|e| Failure::new("E_GPU", e.to_string()))
}

/// Null in builds without the `alloc-count` feature, which count nothing.
fn counted(values: &[u64]) -> Value {
    if values.is_empty() {
        Value::Null
    } else {
        stats::summary(values)
    }
}

#[cfg(feature = "alloc-count")]
fn allocations_so_far() -> Option<(u64, u64)> {
    Some(alloc::so_far())
}

#[cfg(not(feature = "alloc-count"))]
fn allocations_so_far() -> Option<(u64, u64)> {
    None
}

/// The time since its first call, the clock of the frame's stages.
fn since_start() -> Duration {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// The system's 1, 5 and 15 minute load averages.
#[cfg(unix)]
fn load_average() -> Option<[f64; 3]> {
    unsafe extern "C" {
        fn getloadavg(loadavg: *mut f64, nelem: std::ffi::c_int) -> std::ffi::c_int;
    }
    let mut averages = [0.0; 3];
    // SAFETY: the pointer is to three writable f64s, as many as asked for.
    (unsafe { getloadavg(averages.as_mut_ptr(), 3) } == 3).then_some(averages)
}

#[cfg(not(unix))]
fn load_average() -> Option<[f64; 3]> {
    None
}
