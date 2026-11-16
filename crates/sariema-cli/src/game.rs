// SPDX-License-Identifier: Apache-2.0

//! A project's game: its logic crate, cutscenes and runtime settings, built
//! into a game pack by `build`; and the cutscene rules `check` enforces.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sariema_format::script::cutscene::Cutscene;
use sariema_format::script::{BATCH_VERSION, GamePack, encode_game};
use sariema_import::TextureMode;
use sariema_runtime::cutscene::author::{Source, compile};
use sariema_runtime::cutscene::check::check as check_rules;
use sariema_runtime::game::{parse_settings, start};
use sariema_runtime::settings::Settings;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::output::{Done, Failure};
use crate::project;

const CUTSCENE_SCHEMA: &str = include_str!("../../../schemas/cutscene.schema.json");
const WASM_TARGET: &str = "wasm32-unknown-unknown";

#[derive(Deserialize)]
struct GameProject {
    name: String,
    game: Option<GameSection>,
}

/// The `game` object of `sariema.json`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GameSection {
    #[serde(default = "GameSection::logic")]
    logic: PathBuf,
    #[serde(default = "GameSection::cutscenes")]
    cutscenes: PathBuf,
    #[serde(default)]
    settings: Option<Value>,
    #[serde(default)]
    toolchain: Option<String>,
    /// The room the game starts in; the project's first without it.
    #[serde(default)]
    room: Option<String>,
    /// The other rooms the game plays in, with their neighbours and
    /// ambience.
    #[serde(default)]
    rooms: BTreeMap<String, crate::bundle::RoomEntry>,
    /// The HUD's UI description.
    #[serde(default)]
    hud: Option<PathBuf>,
}

impl GameSection {
    fn logic() -> PathBuf {
        "game".into()
    }

    fn cutscenes() -> PathBuf {
        "cutscenes".into()
    }
}

struct Compiled {
    cutscenes: Vec<Cutscene>,
    files: Vec<PathBuf>,
    names: Vec<String>,
}

/// Reads, validates and compiles every `*.json` of the cutscene directory,
/// in file name order.
fn compile_cutscenes(dir: &Path) -> Result<Compiled, Failure> {
    let mut compiled = Compiled {
        cutscenes: Vec::new(),
        files: Vec::new(),
        names: Vec::new(),
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(compiled),
        Err(e) => return Err(Failure::io(dir, e)),
    };
    let mut files: Vec<PathBuf> = entries
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| Failure::io(dir, e))?;
    files.retain(|f| f.extension().is_some_and(|x| x == "json"));
    files.sort();
    let schema: Value =
        serde_json::from_str(CUTSCENE_SCHEMA).expect("cutscene.schema.json is JSON");
    for file in files {
        let text = fs::read_to_string(&file).map_err(|e| Failure::io(&file, e))?;
        let fail = |why: String| Failure::new("E_SCHEMA", format!("{}: {why}", file.display()));
        let value: Value = serde_json::from_str(&text).map_err(|e| fail(e.to_string()))?;
        jsonschema::validate(&schema, &value)
            .map_err(|e| fail(format!("{}: {e}", e.instance_path())))?;
        let source: Source = serde_json::from_value(value).map_err(|e| fail(e.to_string()))?;
        let (cutscene, names) = compile(&source)
            .map_err(|e| Failure::new(e.code, format!("{}: {e}", file.display())))?;
        if let Some(i) = compiled
            .cutscenes
            .iter()
            .position(|c| c.name == cutscene.name)
        {
            return Err(fail(format!(
                "a cutscene named {} is already in {}",
                cutscene.name,
                compiled.files[i].display()
            )));
        }
        for name in names {
            if !compiled.names.contains(&name) {
                compiled.names.push(name);
            }
        }
        compiled.cutscenes.push(cutscene);
        compiled.files.push(file);
    }
    Ok(compiled)
}

/// Fails with every broken rule, the first one's code.
fn enforce_rules(compiled: &Compiled, settings: &Settings) -> Result<(), Failure> {
    let broken: Vec<(String, &'static str)> = compiled
        .cutscenes
        .iter()
        .zip(&compiled.files)
        .flat_map(|(c, file)| {
            check_rules(c, settings)
                .into_iter()
                .map(move |e| (format!("{}: {e} ({})", file.display(), e.code), e.code))
        })
        .collect();
    let Some(&(_, code)) = broken.first() else {
        return Ok(());
    };
    let lines: Vec<String> = broken.into_iter().map(|(line, _)| line).collect();
    Err(Failure::new(code, lines.join("\n")))
}

fn read_project(dir: &Path) -> Result<(String, Option<GameSection>), Failure> {
    let project: GameProject = project::load(dir)?;
    Ok((project.name, project.game))
}

/// The settings `game.settings` gives, as a game pack carries them.
fn game_settings(dir: &Path, game: &GameSection) -> Result<(String, Settings), Failure> {
    let text = game
        .settings
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_default();
    let settings = parse_settings(&text).map_err(|e| {
        Failure::new(
            e.code,
            format!("{}: game.settings: {e}", dir.join("sariema.json").display()),
        )
    })?;
    Ok((text, settings))
}

/// The cutscene rules `sariema check` enforces on a project's game: the names
/// of the cutscenes that pass, none without a `game` section.
pub fn check(dir: &Path) -> Result<Vec<String>, Failure> {
    let (_, game) = read_project(dir)?;
    let Some(game) = game else {
        return Ok(Vec::new());
    };
    let (_, settings) = game_settings(dir, &game)?;
    let compiled = compile_cutscenes(&dir.join(&game.cutscenes))?;
    enforce_rules(&compiled, &settings)?;
    Ok(compiled.cutscenes.into_iter().map(|c| c.name).collect())
}

/// `sariema build` without `--room`: compiles the logic and writes the game pack.
pub fn build(dir: &Path, output: &Path, textures: Option<TextureMode>) -> Result<Done, Failure> {
    let (name, game) = read_project(dir)?;
    let game = game.ok_or_else(|| {
        Failure::new(
            "E_SCHEMA",
            format!(
                "{}: no game section; pass --room to build a room pack",
                dir.join("sariema.json").display()
            ),
        )
    })?;
    let (text, settings) = game_settings(dir, &game)?;
    let compiled = compile_cutscenes(&dir.join(&game.cutscenes))?;
    enforce_rules(&compiled, &settings)?;
    let mut pack = GamePack {
        name,
        batch_version: BATCH_VERSION,
        wasm: Vec::new(),
        cutscenes: compiled.cutscenes,
        names: compiled.names,
        settings: text,
        assets: crate::bundle::assets(
            dir,
            game.room.as_deref(),
            &game.rooms,
            game.hud.as_deref(),
            textures,
        )?,
    };
    pack.wasm = compile_logic(&dir.join(&game.logic), game.toolchain.as_deref())?;
    start(pack.clone(), 0).map_err(|e| Failure::new(e.code, format!("{}: {e}", dir.display())))?;
    let bytes = encode_game(&pack);
    fs::write(output, &bytes).map_err(|e| Failure::io(output, e))?;
    let cutscenes: Vec<&str> = pack.cutscenes.iter().map(|c| c.name.as_str()).collect();
    Ok(Done {
        json: json!({
            "game": pack.name,
            "output": output.display().to_string(),
            "bytes": bytes.len(),
            "wasm_bytes": pack.wasm.len(),
            "batch_version": pack.batch_version,
            "cutscenes": cutscenes,
            "room": !pack.assets.rooms.is_empty(),
            "rooms": pack.assets.rooms.iter().map(|r| &r.name).collect::<Vec<_>>(),
            "collision": pack.assets.rooms.first().is_some_and(|r| r.collision.is_some()),
            "actors": pack.assets.actors.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            "sounds": pack.assets.sounds.is_some(),
            "hud": pack.assets.hud.is_some(),
        }),
        text: format!(
            "built {} into {}: {} bytes, {} of logic, {} cutscenes",
            pack.name,
            output.display(),
            bytes.len(),
            pack.wasm.len(),
            cutscenes.len()
        ),
    })
}

fn build_error(message: impl Into<String>) -> Failure {
    Failure::new("E_GAME_BUILD", message)
}

/// `cargo build --release --target wasm32-unknown-unknown` of the logic
/// crate, with the project's rustup toolchain when it names one, and the
/// bytes of the `cdylib` it produced.
fn compile_logic(crate_dir: &Path, toolchain: Option<&str>) -> Result<Vec<u8>, Failure> {
    let manifest = crate_dir.join("Cargo.toml");
    if !manifest.is_file() {
        return Err(Failure::io(
            &manifest,
            "the game's logic crate has no Cargo.toml",
        ));
    }
    // Cargo relinks the .wasm on every build, even a fresh one, so a
    // concurrent build of the same crate could remove it under our read.
    let lock = fs::File::open(&manifest).map_err(|e| Failure::io(&manifest, e))?;
    lock.lock().map_err(|e| Failure::io(&manifest, e))?;
    let mut cargo = match toolchain {
        Some(toolchain) => {
            let mut c = Command::new("rustup");
            c.args(["run", toolchain, "cargo"]);
            // rustup may not put the toolchain's rustc first on PATH; name it.
            c.env("RUSTC", toolchain_rustc(toolchain)?);
            c
        }
        None => Command::new("cargo"),
    };
    cargo
        .args(["build", "--release", "--target", WASM_TARGET])
        .args([
            "--message-format",
            "json-render-diagnostics",
            "--manifest-path",
        ])
        .arg(&manifest)
        .stdin(Stdio::null());
    let out = cargo
        .output()
        .map_err(|e| build_error(format!("could not run cargo: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(30).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(build_error(format!(
            "cargo could not build {}:\n{}",
            manifest.display(),
            tail.join("\n")
        )));
    }
    let wasm = String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .filter(|m| {
            m["target"]["kind"]
                .as_array()
                .is_some_and(|k| k.iter().any(|k| k == "cdylib"))
        })
        .filter_map(|m| {
            m["filenames"].as_array()?.iter().find_map(|f| {
                let f = f.as_str()?;
                f.ends_with(".wasm").then(|| PathBuf::from(f))
            })
        })
        .next()
        .ok_or_else(|| {
            build_error(format!(
                "{} built no .wasm; its [lib] needs crate-type = [\"cdylib\"]",
                manifest.display()
            ))
        })?;
    fs::read(&wasm).map_err(|e| Failure::io(&wasm, e))
}

fn toolchain_rustc(toolchain: &str) -> Result<PathBuf, Failure> {
    let out = Command::new("rustup")
        .args(["run", toolchain, "rustc", "--print", "sysroot"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| build_error(format!("could not run rustup: {e}")))?;
    if !out.status.success() {
        return Err(build_error(format!(
            "rustup has no toolchain {toolchain}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(Path::new(&sysroot).join("bin").join("rustc"))
}
