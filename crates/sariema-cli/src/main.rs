// SPDX-License-Identifier: Apache-2.0

//! `sariema`: builds, inspects, runs and tests room packs.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum, value_parser};
use sariema_import::TextureMode;
use sariema_render::Quality;

mod actor;
mod golden;
mod output;
mod pack;
mod player;
mod project;
mod run;

#[derive(Parser)]
#[command(
    name = "sariema",
    version,
    about = "Builds, inspects, runs and tests Sariema room packs."
)]
struct Cli {
    /// Print one JSON object on stdout, errors included.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build one room or one actor of a project into a pack.
    Build {
        /// Directory holding sariema.json.
        project: PathBuf,
        /// Room id from the project's `rooms`.
        #[arg(long, required_unless_present = "actor", conflicts_with = "actor")]
        room: Option<String>,
        /// Actor id from the project's `actors`.
        #[arg(long)]
        actor: Option<String>,
        /// Pack to write, conventionally `.srma` for a room and `.srmk` for an actor.
        #[arg(short, long)]
        output: PathBuf,
        /// How to store colour textures, overriding the room file's `textures`.
        #[arg(long, value_enum)]
        textures: Option<Textures>,
    },
    /// Show a room pack's stats, lights, materials and loose draws, or an
    /// actor pack's joints, clips, parts, sockets and faces.
    Inspect { pack: PathBuf },
    /// Draw a pack's room in a window, or offscreen with --headless.
    ///
    /// In the window, dragging with the left button moves the camera over the
    /// floor, the wheel changes its distance, R resets it, T toggles the
    /// tilt-shift blur and Esc closes. Its report's ms_per_frame is the average
    /// time between presented frames, paced by the display, and null when
    /// fewer than two frames were presented.
    ///
    /// Offscreen, the report's cpu_ms_per_frame is the CPU time to encode and
    /// submit a frame and excludes GPU execution; wall_ms_per_frame waits for
    /// the GPU to finish every frame.
    Run(RunArgs),
    /// Render each X.srma of a directory and compare it with X.golden.png beside it.
    Test {
        dir: PathBuf,
        /// Rewrite the references instead of comparing.
        #[arg(long)]
        update: bool,
    },
}

#[derive(Args)]
struct RunArgs {
    pack: PathBuf,
    /// Draw offscreen instead of in a window.
    #[arg(long, requires_all = ["frames", "screenshot"])]
    headless: bool,
    /// Frames to draw before closing; without it the window stays open until closed.
    #[arg(long, value_parser = value_parser!(u32).range(1..))]
    frames: Option<u32>,
    /// PNG to save the last frame into.
    #[arg(long, requires = "headless")]
    screenshot: Option<PathBuf>,
    /// In pixels; for a window, its initial inner size.
    #[arg(long, default_value_t = 1920, value_parser = value_parser!(u32).range(1..))]
    width: u32,
    #[arg(long, default_value_t = 1080, value_parser = value_parser!(u32).range(1..))]
    height: u32,
    #[arg(long, value_enum, default_value_t = Tier::Medium)]
    quality: Tier,
    /// Fail with E_ROOM_VOID when a larger fraction of the frame shows no geometry.
    #[arg(long, value_parser = fraction, requires = "headless")]
    max_background: Option<f32>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Textures {
    Rgba8,
    Astc,
}

impl Textures {
    fn mode(self) -> TextureMode {
        match self {
            Textures::Rgba8 => TextureMode::Rgba8,
            Textures::Astc => TextureMode::Astc4x4,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Tier {
    Low,
    Medium,
    High,
}

impl Tier {
    fn settings(self) -> Quality {
        match self {
            Tier::Low => Quality::low(),
            Tier::Medium => Quality::medium(),
            Tier::High => Quality::high(),
        }
    }
}

fn fraction(s: &str) -> Result<f32, String> {
    let value = s.parse::<f32>().map_err(|e| e.to_string())?;
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err("must be between 0 and 1".into())
    }
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return output::parse_error(e, output::json_flag(std::env::args_os())),
    };
    let outcome = output::guarded(|| match &cli.command {
        Command::Build {
            project,
            room,
            actor,
            output,
            textures,
        } => match (room, actor) {
            (Some(room), _) => project::build(project, room, output, textures.map(Textures::mode)),
            (None, Some(actor)) => {
                actor::build(project, actor, output, textures.map(Textures::mode))
            }
            (None, None) => unreachable!("clap requires --room or --actor"),
        },
        Command::Inspect { pack } => pack::inspect(pack),
        Command::Run(args) => run::run(args),
        Command::Test { dir, update } => golden::test(dir, *update),
    });
    output::emit(cli.json, outcome)
}
