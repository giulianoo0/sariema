// SPDX-License-Identifier: Apache-2.0

//! A game pack in the simulation: its WebAssembly logic in the sandboxed
//! machine, its cutscenes and its settings.

use sariema_format::script::{BATCH_VERSION, GamePack, decode_game};
use sariema_vm::{LogLine, Vm, VmError};

use crate::settings::Settings;
use crate::sim::{Logic, Simulation, Stop};

/// A game's logic running in the machine.
pub struct WasmLogic {
    vm: Vm,
}

impl WasmLogic {
    pub fn new(vm: Vm) -> WasmLogic {
        WasmLogic { vm }
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    /// Lines the last tick logged.
    pub fn log(&self) -> &[LogLine] {
        self.vm.log()
    }
}

impl Logic for WasmLogic {
    fn tick(&mut self, events: &[u8], ends: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop> {
        self.vm.tick(events, ends, commands).map_err(|e| Stop {
            code: e.code(),
            message: e.to_string(),
        })
    }

    fn regions(&self, each: &mut dyn FnMut(u32, &[u8])) {
        for (region, bytes) in self.vm.regions() {
            each(region.kind, bytes);
        }
    }

    fn memory(&self) -> Option<&[u8]> {
        Some(self.vm.memory())
    }
}

#[derive(Debug)]
pub struct LoadError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LoadError {}

impl From<VmError> for LoadError {
    fn from(e: VmError) -> LoadError {
        LoadError {
            code: e.code(),
            message: e.to_string(),
        }
    }
}

/// The runtime settings a pack carries; an empty text is the defaults.
pub fn pack_settings(pack: &GamePack) -> Result<Settings, LoadError> {
    parse_settings(&pack.settings).map_err(|e| LoadError {
        code: e.code,
        message: format!("game pack {}: settings: {}", pack.name, e.message),
    })
}

/// Runtime settings from their JSON text, checked; an empty text is the
/// defaults.
pub fn parse_settings(text: &str) -> Result<Settings, LoadError> {
    let settings = if text.trim().is_empty() {
        Settings::default()
    } else {
        serde_json::from_str(text).map_err(|e| LoadError {
            code: "E_SCHEMA",
            message: e.to_string(),
        })?
    };
    settings.validate().map_err(|message| LoadError {
        code: "E_SETTINGS",
        message,
    })?;
    Ok(settings)
}

/// Loads a game pack into a simulation seeded with `seed`.
pub fn load(bytes: &[u8], seed: u64) -> Result<Simulation<WasmLogic>, LoadError> {
    let pack = decode_game(bytes).map_err(|e| LoadError {
        code: e.code(),
        message: e.to_string(),
    })?;
    start(pack, seed)
}

pub fn start(pack: GamePack, seed: u64) -> Result<Simulation<WasmLogic>, LoadError> {
    let settings = pack_settings(&pack)?;
    let mut vm = Vm::load(&pack.wasm, settings.vm)?;
    match vm.batch_version()? {
        Some(v) if v != BATCH_VERSION => {
            return Err(LoadError {
                code: "E_BATCH_VERSION",
                message: format!(
                    "game pack {}: its logic was built against batch version {v}; this engine reads version {BATCH_VERSION}",
                    pack.name
                ),
            });
        }
        _ => {}
    }
    Ok(Simulation::new(
        settings,
        pack.cutscenes,
        WasmLogic::new(vm),
        seed,
    ))
}
