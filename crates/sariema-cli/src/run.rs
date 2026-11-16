// SPDX-License-Identifier: Apache-2.0

//! The `run` command; [`crate::player`] has its window.

use std::path::Path;
use std::time::{Duration, Instant};

use sariema_core::room_camera;
use sariema_format::RoomPack;
use sariema_render::{FrameStats, Headless, render_coverage};
use serde_json::{Value, json};

use crate::RunArgs;
use crate::output::{Done, Failure};
use crate::{pack, player};

/// What `run` was asked for, with what each way of drawing needs.
enum Mode<'a> {
    /// Until the window is closed, or for `frames` frames.
    Window { frames: Option<u32> },
    Headless {
        frames: u32,
        screenshot: &'a Path,
        max_background: Option<f32>,
    },
}

impl RunArgs {
    fn mode(&self) -> Mode<'_> {
        match (self.headless, self.frames, &self.screenshot) {
            (true, Some(frames), Some(screenshot)) => Mode::Headless {
                frames,
                screenshot,
                max_background: self.max_background,
            },
            // clap accepts --headless only with --frames and --screenshot.
            _ => Mode::Window {
                frames: self.frames,
            },
        }
    }
}

pub fn run(args: &RunArgs) -> Result<Done, Failure> {
    let room = pack::read(&args.pack)?;
    match args.mode() {
        Mode::Window { frames } => player::play(&room, args, frames),
        Mode::Headless {
            frames,
            screenshot,
            max_background,
        } => headless(&room, args, frames, screenshot, max_background),
    }
}

/// Draws `frames` frames at `time_s = frame / 60` and saves the last one.
fn headless(
    room: &RoomPack,
    args: &RunArgs,
    frames: u32,
    screenshot: &Path,
    max_background: Option<f32>,
) -> Result<Done, Failure> {
    let camera = room_camera(&room.camera, args.width as f32 / args.height as f32);
    let mut headless = Headless::new(args.width, args.height, args.quality.settings())?;
    let loading = Instant::now();
    headless.renderer().load_room(room)?;
    let load_room_ms = loading.elapsed().as_secs_f64() * 1000.0;
    let pipelines = headless.renderer().pipelines_created();
    let mut cpu = Duration::ZERO;
    let started = Instant::now();
    for frame in 0..frames {
        let start = Instant::now();
        headless.draw(&camera, frame as f32 / 60.0);
        cpu += start.elapsed();
    }
    headless.finish()?;
    let wall = started.elapsed();
    let pipelines_created = headless.renderer().pipelines_created() - pipelines;
    let stats = headless.renderer().last_frame_stats();
    headless
        .image()
        .save(screenshot)
        .map_err(|e| Failure::io(screenshot, e))?;
    let background = render_coverage(room, &camera, args.width, args.height)?;
    if let Some(max) = max_background
        && background > max
    {
        return Err(Failure::new(
            "E_ROOM_VOID",
            format!(
                "{}: {:.1}% of the frame shows no geometry, more than the {:.1}% allowed",
                args.pack.display(),
                background * 100.0,
                max * 100.0
            ),
        ));
    }
    let per_frame_ms = |d: Duration| d.as_secs_f64() * 1000.0 / frames as f64;
    let (cpu_ms, wall_ms) = (per_frame_ms(cpu), per_frame_ms(wall));
    Ok(Done {
        json: json!({
            "frames": frames,
            "load_room_ms": load_room_ms,
            "screenshot": screenshot.display().to_string(),
            "cpu_ms_per_frame": cpu_ms,
            "wall_ms_per_frame": wall_ms,
            "pipelines_created_during_frames": pipelines_created,
            "background_ratio": background,
            "frame_stats": frame_stats_json(&stats),
        }),
        text: format!(
            "{} frames into {}: {cpu_ms:.2} ms CPU and {wall_ms:.2} ms wall per frame, \
             {:.1}% background, room loaded in {load_room_ms:.1} ms\n\
             lights: {} in view, {} dropped, {} of {} clusters overflowed, {} shadows skipped; \
             {} post passes",
            frames,
            screenshot.display(),
            background * 100.0,
            stats.lights_in_frustum,
            stats.lights_dropped,
            stats.cluster_overflows,
            stats.clusters,
            stats.shadows_skipped,
            stats.post_passes
        ),
    })
}

/// The `frame_stats` object of both ways' JSON reports.
pub fn frame_stats_json(stats: &FrameStats) -> Value {
    json!({
        "lights_in_frustum": stats.lights_in_frustum,
        "lights_dropped": stats.lights_dropped,
        "clusters": stats.clusters,
        "max_lights_overlapping_cluster": stats.max_lights_overlapping_cluster,
        "cluster_overflows": stats.cluster_overflows,
        "shadows_skipped": stats.shadows_skipped,
        "post_passes": stats.post_passes,
        "draws_culled": stats.draws_culled,
    })
}
