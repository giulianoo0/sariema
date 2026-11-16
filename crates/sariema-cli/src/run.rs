// SPDX-License-Identifier: Apache-2.0

//! The `run --headless` command.

use std::time::{Duration, Instant};

use sariema_core::room_camera;
use sariema_render::{Headless, render_coverage};
use serde_json::json;

use crate::RunArgs;
use crate::output::{Done, Failure};
use crate::pack;

/// Draws `frames` frames at `time_s = frame / 60` and saves the last one.
pub fn run(args: &RunArgs) -> Result<Done, Failure> {
    let room = pack::read(&args.pack)?;
    let camera = room_camera(&room.camera, args.width as f32 / args.height as f32);
    let mut headless = Headless::new(args.width, args.height, args.quality.settings())?;
    headless.renderer().load_room(&room)?;
    let pipelines = headless.renderer().pipelines_created();
    let mut cpu = Duration::ZERO;
    for frame in 0..args.frames {
        let start = Instant::now();
        headless.draw(&camera, frame as f32 / 60.0);
        cpu += start.elapsed();
    }
    let pipelines_created = headless.renderer().pipelines_created() - pipelines;
    let stats = headless.renderer().last_frame_stats();
    headless
        .image()
        .save(&args.screenshot)
        .map_err(|e| Failure::io(&args.screenshot, e))?;
    let background = render_coverage(&room, &camera, args.width, args.height)?;
    if let Some(max) = args.max_background
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
    let cpu_ms = cpu.as_secs_f64() * 1000.0 / args.frames as f64;
    Ok(Done {
        json: json!({
            "frames": args.frames,
            "screenshot": args.screenshot.display().to_string(),
            "cpu_ms_per_frame": cpu_ms,
            "pipelines_created_during_frames": pipelines_created,
            "background_ratio": background,
            "frame_stats": {
                "lights_in_frustum": stats.lights_in_frustum,
                "lights_dropped": stats.lights_dropped,
                "clusters": stats.clusters,
                "max_lights_overlapping_cluster": stats.max_lights_overlapping_cluster,
                "cluster_overflows": stats.cluster_overflows,
                "shadows_skipped": stats.shadows_skipped,
            },
        }),
        text: format!(
            "{} frames into {}: {cpu_ms:.2} ms CPU per frame, {:.1}% background\n\
             lights: {} in view, {} dropped, {} of {} clusters overflowed, {} shadows skipped",
            args.frames,
            args.screenshot.display(),
            background * 100.0,
            stats.lights_in_frustum,
            stats.lights_dropped,
            stats.cluster_overflows,
            stats.clusters,
            stats.shadows_skipped
        ),
    })
}
