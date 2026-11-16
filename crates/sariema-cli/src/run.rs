// SPDX-License-Identifier: Apache-2.0

//! The `run` command; [`crate::player`] has its window.

use std::path::Path;
use std::time::{Duration, Instant};

use sariema_format::RoomPack;
use sariema_render::{FrameStats, Headless, render_coverage};
use serde_json::{Value, json};

use crate::RunArgs;
use crate::output::{Done, Failure};
use crate::stage::{self, Stage};
use crate::{filmstrip, game_player, pack, player, simulate};

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
    fn mode(&self) -> Result<Mode<'_>, Failure> {
        match (self.headless, self.frames, &self.screenshot) {
            (true, Some(frames), Some(screenshot)) => Ok(Mode::Headless {
                frames,
                screenshot,
                max_background: self.max_background,
            }),
            (true, ..) => Err(Failure::new(
                "E_USAGE",
                "--headless needs --frames and --screenshot to draw a room pack",
            )),
            // clap accepts --screenshot and --max-background only with --headless.
            _ => Ok(Mode::Window {
                frames: self.frames,
            }),
        }
    }
}

pub fn run(args: &RunArgs) -> Result<Done, Failure> {
    if let Some(game) = &args.game.game {
        return match (args.headless, args.game.steps, args.frames) {
            (true, Some(_), None) => simulate::run(game, args),
            (true, ..) => Err(Failure::new(
                "E_USAGE",
                "--headless --game needs --steps, and counts steps, not --frames",
            )),
            (false, ..) => game_player::play(game, args, args.frames),
        };
    }
    // clap requires the pack without --game.
    let path = args.pack.as_deref().expect("a room pack");
    let mode = args.mode()?;
    let room = pack::read(path)?;
    match mode {
        Mode::Window { frames } => player::play(&room, args, frames),
        Mode::Headless {
            frames,
            screenshot,
            max_background,
        } => headless(&room, path, args, frames, screenshot, max_background),
    }
}

/// Draws `frames` frames at `time_s = frame / 60` and saves the last one.
fn headless(
    room: &RoomPack,
    path: &Path,
    args: &RunArgs,
    frames: u32,
    screenshot: &Path,
    max_background: Option<f32>,
) -> Result<Done, Failure> {
    let camera = stage::camera(
        &args.stage,
        &room.camera,
        args.width as f32 / args.height as f32,
    );
    let mut headless = Headless::new(args.width, args.height, args.quality.settings())?;
    let loading = Instant::now();
    headless.renderer().load_room(room)?;
    let load_room_ms = loading.elapsed().as_secs_f64() * 1000.0;
    let loading = Instant::now();
    let mut stage = Stage::load(headless.renderer(), &args.stage)?;
    let load_actors_ms = loading.elapsed().as_secs_f64() * 1000.0;
    let pipelines = headless.renderer().pipelines_created();
    let (mut cpu, mut posing) = (Duration::ZERO, Duration::ZERO);
    let started = Instant::now();
    for frame in 0..frames {
        let start = Instant::now();
        // The last frame shows `--time`; each one before it, a step earlier.
        let time = args.stage.time - (frames - 1 - frame) as f32 / 60.0;
        stage.pose(headless.renderer(), time)?;
        posing += start.elapsed();
        headless.draw(&camera, frame as f32 / 60.0);
        cpu += start.elapsed();
    }
    headless.finish()?;
    let wall = started.elapsed();
    let pipelines_created = headless.renderer().pipelines_created() - pipelines;
    let stats = headless.renderer().last_frame_stats();
    let (image, strip) = match args.stage.filmstrip {
        Some(count) => {
            let span = (args.stage.span).or(stage.first_clip_s()).unwrap_or(0.0);
            let times = filmstrip::times(args.stage.time, span, count);
            let mut frames = Vec::with_capacity(times.len());
            for &time in &times {
                stage.pose(headless.renderer(), time)?;
                frames.push(headless.render(&camera, time));
            }
            let strip = json!({
                "frames": count,
                "columns": args.stage.columns.min(count),
                "times_s": times,
            });
            (filmstrip::tile(&frames, args.stage.columns), Some(strip))
        }
        None => (headless.image(), None),
    };
    image
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
                path.display(),
                background * 100.0,
                max * 100.0
            ),
        ));
    }
    let per_frame_ms = |d: Duration| d.as_secs_f64() * 1000.0 / frames as f64;
    let (cpu_ms, wall_ms) = (per_frame_ms(cpu), per_frame_ms(wall));
    let actors_ms = per_frame_ms(posing);
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
            "load_actors_ms": load_actors_ms,
            "actors_ms_per_frame": actors_ms,
            "actors": stage.json(),
            "filmstrip": strip,
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
        "actors_drawn": stats.actors_drawn,
        "actors_culled": stats.actors_culled,
    })
}
