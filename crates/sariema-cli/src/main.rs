// SPDX-License-Identifier: Apache-2.0

//! `sariema`: builds, inspects, checks, runs and tests Sariema packs.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum, value_parser};
use sariema_import::TextureMode;
use sariema_render::Quality;

mod actor;
mod api;
mod audio;
mod bench;
mod bundle;
mod check;
mod collision;
mod filmstrip;
mod game;
mod game_frames;
mod game_player;
mod game_view;
mod golden;
mod output;
mod pack;
mod physics_map;
mod player;
mod project;
mod run;
mod scaffold;
mod simulate;
mod spot_check;
mod stage;
mod ui;
mod window;

#[derive(Parser)]
#[command(
    name = "sariema",
    version,
    about = "Builds, inspects, checks, runs and tests Sariema packs."
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
    /// Build one room or one actor of a project into a pack, or with neither
    /// the project's game into a game pack.
    #[command(group(clap::ArgGroup::new("source").args(["room", "actor"])))]
    Build {
        /// Directory holding sariema.json.
        project: PathBuf,
        /// Room id from the project's `rooms`.
        #[arg(long, conflicts_with = "actor")]
        room: Option<String>,
        /// Actor id from the project's `actors`.
        #[arg(long)]
        actor: Option<String>,
        /// Pack to write, conventionally `.srma` for a room, `.srmk` for an
        /// actor and `.srmg` for a game.
        #[arg(short, long)]
        output: PathBuf,
        /// How to store colour textures, overriding the room file's `textures`.
        #[arg(long, value_enum, requires = "source")]
        textures: Option<Textures>,
    },
    /// Show a room pack's stats, lights, materials and loose draws, an actor
    /// pack's joints, clips, parts, sockets and faces, or a collision pack's
    /// colliders, triggers, spots and routes.
    Inspect { pack: PathBuf },
    /// Draw a room pack in a window, or offscreen with --headless; play a game
    /// pack (--game) in a window, or step it headless.
    ///
    /// In a room's window, dragging with the left button moves the camera over
    /// the floor, the wheel changes its distance, R resets it, T toggles the
    /// tilt-shift blur and Esc closes. A game's window takes the keys its
    /// input settings bind and the mouse as the HUD's pointer, and plays its
    /// sound on the default output device, silent without one. Either
    /// report's ms_per_frame is the average time between presented frames,
    /// paced by the display, and null when fewer than two were presented.
    ///
    /// Offscreen, the report's cpu_ms_per_frame is the CPU time to encode and
    /// submit a frame and excludes GPU execution; wall_ms_per_frame waits for
    /// the GPU to finish every frame.
    Run(Box<RunArgs>),
    /// Render each X.srma of a directory and compare it with X.golden.png beside it.
    Test {
        dir: PathBuf,
        /// Rewrite the references instead of comparing.
        #[arg(long)]
        update: bool,
    },
    /// Check a project's rooms (every action spot clear and on ground, every
    /// route walkable) and its cutscenes (the no-teleport and start rules).
    Check(check::CheckArgs),
    /// Draw a collision pack from above into a PNG: colliders, walkable cells, triggers, spots, a path.
    PhysicsMap(physics_map::MapArgs),
    /// Draw a room pack offscreen many times, or step and draw a game pack, and
    /// report the cost of each load stage, frame stage and GPU pass, with
    /// draws, passes, uploads and allocations per frame and the HUD pass's own.
    Bench(bench::BenchArgs),
    /// Build, inspect and check sound banks; render cue lists offline.
    Audio(audio::AudioArgs),
    /// Draw UI descriptions headless at a screen size and a moment.
    Ui(ui::UiArgs),
    /// Make a game project that builds, checks and runs as it is.
    New {
        /// Directory to create; it must be missing or empty.
        dir: PathBuf,
        /// The engine's crates/sariema-sdk, which the game depends on.
        #[arg(long)]
        sdk: Option<PathBuf>,
        /// rustup toolchain the project builds its logic with, such as 1.100.0.
        #[arg(long)]
        toolchain: Option<String>,
    },
    /// Describe every command, event, enum, setting and error code of the game API.
    Api,
}

#[derive(Args)]
struct RunArgs {
    /// Room pack to draw.
    #[arg(required_unless_present = "game")]
    pack: Option<PathBuf>,
    /// Draw offscreen instead of in a window; a room pack needs --frames and --screenshot.
    #[arg(long)]
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
    #[command(flatten)]
    stage: stage::StageArgs,
    #[command(flatten)]
    game: simulate::GameRun,
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
            (None, None) => game::build(project, output),
        },
        Command::Inspect { pack } => pack::inspect(pack),
        Command::Run(args) => run::run(args),
        Command::Test { dir, update } => golden::test(dir, *update),
        Command::Check(args) => check::run(args),
        Command::PhysicsMap(args) => physics_map::run(args),
        Command::Bench(args) => bench::bench(args),
        Command::Audio(args) => audio::run(args),
        Command::Ui(args) => ui::run(args),
        Command::New {
            dir,
            sdk,
            toolchain,
        } => scaffold::new(dir, sdk.as_deref(), toolchain.as_deref()),
        Command::Api => api::api(),
    });
    output::emit(cli.json, outcome)
}
