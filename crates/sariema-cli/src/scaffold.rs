// SPDX-License-Identifier: Apache-2.0

//! `sariema new`: a game project that builds, checks and runs as it is.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::output::{Done, Failure};

/// Where the SDK was when this tool was built.
const SDK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../sariema-sdk");

const FILES: &[(&str, &[u8])] = &[
    (
        "rooms/start.json",
        include_bytes!("../templates/new/rooms/start.json"),
    ),
    (
        "montagem.json",
        include_bytes!("../../../examples/demo/montagem.json"),
    ),
    (
        "luz.json",
        include_bytes!("../../../examples/demo/luz.json"),
    ),
    (
        "kit.glb",
        include_bytes!("../../sariema-import/tests/fixtures/kit_min.opt.glb"),
    ),
    (
        "cutscenes/intro.json",
        include_bytes!("../templates/new/cutscenes/intro.json"),
    ),
    (
        "input/walk.json",
        include_bytes!("../templates/new/input/walk.json"),
    ),
    (
        "game/src/lib.rs",
        include_bytes!("../templates/new/game/src/lib.rs"),
    ),
    ("game/.gitignore", b"/target\n"),
];

/// A crate name from the directory's: lowercase ASCII letters, digits and
/// dashes, starting with a letter.
fn crate_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c @ ('a'..='z' | '0'..='9') => c,
            _ => '-',
        })
        .collect();
    if !out.starts_with(|c: char| c.is_ascii_lowercase()) {
        out.insert_str(0, "game-");
    }
    out
}

fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn new(dir: &Path, sdk: Option<&Path>, toolchain: Option<&str>) -> Result<Done, Failure> {
    if fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some()) {
        return Err(Failure::new(
            "E_PROJECT_EXISTS",
            format!(
                "{} is not empty; new writes into an empty or missing directory",
                dir.display()
            ),
        ));
    }
    let sdk = sdk.map_or_else(|| PathBuf::from(SDK), Path::to_path_buf);
    let sdk = sdk.canonicalize().map_err(|e| {
        Failure::new(
            "E_USAGE",
            format!(
                "the SDK is not at {} ({e}); pass --sdk <path to crates/sariema-sdk>",
                sdk.display()
            ),
        )
    })?;
    let name = dir
        .canonicalize()
        .unwrap_or_else(|_| dir.to_path_buf())
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("game")
        .to_string();
    let mut game = json!({ "logic": "game", "cutscenes": "cutscenes" });
    if let Some(toolchain) = toolchain {
        game["toolchain"] = json!(toolchain);
    }
    let project = json!({
        "name": name,
        "assets_root": ".",
        "rooms": { "start": "rooms/start.json" },
        "game": game,
    });
    let manifest = format!(
        "[package]\nname = {}\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n\
         # Its own workspace: the game builds apart from any other.\n[workspace]\n\n\
         [lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n\n\
         [dependencies]\nsariema-sdk = {{ path = {} }}\n\n\
         [profile.release]\nopt-level = \"s\"\npanic = \"abort\"\n",
        toml_string(&crate_name(&name)),
        toml_string(&sdk.display().to_string())
    );
    let project = serde_json::to_string_pretty(&project).expect("JSON") + "\n";
    let generated = [
        ("sariema.json", project.as_bytes()),
        ("game/Cargo.toml", manifest.as_bytes()),
    ];
    let mut written = Vec::new();
    for (path, bytes) in generated.into_iter().chain(FILES.iter().copied()) {
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(|e| Failure::io(parent, e))?;
        }
        fs::write(&file, bytes).map_err(|e| Failure::io(&file, e))?;
        written.push(path);
    }
    Ok(Done {
        json: json!({ "project": dir.display().to_string(), "name": name, "files": written }),
        text: format!(
            "made {name} in {}; next: sariema check {0}, sariema build {0} -o {name}.srmg",
            dir.display()
        ),
    })
}
