// SPDX-License-Identifier: Apache-2.0

//! `sariema`: builds, inspects, runs and tests room packs.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum, value_parser};
use sariema_render::Quality;

mod golden;
mod output;
mod pack;
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
    /// Build one room of a project into a pack.
    Build {
        /// Directory holding sariema.json.
        project: PathBuf,
        /// Room id from the project's `rooms`.
        #[arg(long)]
        room: String,
        /// Pack to write, conventionally `.srma`.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Show a pack's stats, lights, materials and loose draws.
    Inspect { pack: PathBuf },
    /// Draw a pack's room.
    ///
    /// The report's cpu_ms_per_frame is the CPU time to encode and submit a
    /// frame and excludes GPU execution; wall_ms_per_frame waits for the GPU
    /// to finish every frame.
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
    /// Draw offscreen. Required: there is no window yet.
    #[arg(long, required = true)]
    headless: bool,
    #[arg(long, value_parser = value_parser!(u32).range(1..))]
    frames: u32,
    /// PNG to save the last frame into.
    #[arg(long)]
    screenshot: PathBuf,
    #[arg(long, default_value_t = 1920, value_parser = value_parser!(u32).range(1..))]
    width: u32,
    #[arg(long, default_value_t = 1080, value_parser = value_parser!(u32).range(1..))]
    height: u32,
    #[arg(long, value_enum, default_value_t = Tier::Medium)]
    quality: Tier,
    /// Fail with E_ROOM_VOID when a larger fraction of the frame shows no geometry.
    #[arg(long, value_parser = fraction)]
    max_background: Option<f32>,
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
            output,
        } => project::build(project, room, output),
        Command::Inspect { pack } => pack::inspect(pack),
        Command::Run(args) => run::run(args),
        Command::Test { dir, update } => golden::test(dir, *update),
    });
    output::emit(cli.json, outcome)
}
