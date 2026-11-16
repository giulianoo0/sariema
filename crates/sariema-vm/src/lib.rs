// SPDX-License-Identifier: Apache-2.0

//! The sandboxed WebAssembly machine that runs a game's logic.
//!
//! A game module exports `memory` and `sariema_tick`, optionally
//! `sariema_batch_version() -> i32`, and imports only the four functions
//! in [`host`]. No file, network or system access exists. Every tick runs
//! under a fuel budget and the memory ceiling; exceeding either, or a trap,
//! stops the game for good with a [`VmError`]; the host is never hung or
//! panicked, and no step is resumed half-way.

use serde::{Deserialize, Serialize};

pub mod host;
mod wasmi_backend;

pub use host::{LogLine, Region};
pub use wasmi_backend::Wasmi;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VmSettings {
    /// Ceiling of the game's linear memory, in bytes.
    pub memory_bytes: u64,
    /// Fuel (about one unit per WebAssembly instruction) a tick may use.
    pub fuel_per_step: u64,
    /// Fuel of the first tick, which usually builds the world.
    pub fuel_first_step: u64,
    /// Elements a table may grow to.
    pub table_elements: u32,
    /// Nested calls before the module traps.
    pub max_call_depth: u32,
    /// Bulk regions a game may register.
    pub max_regions: u32,
    /// Bytes of log text kept per tick; the rest is dropped.
    pub log_bytes_per_step: u32,
}

impl Default for VmSettings {
    fn default() -> VmSettings {
        VmSettings {
            memory_bytes: 64 << 20,
            fuel_per_step: 20_000_000,
            fuel_first_step: 200_000_000,
            table_elements: 10_000,
            max_call_depth: 1000,
            max_regions: 16,
            log_bytes_per_step: 4096,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmError {
    /// A tick used all of its fuel.
    Budget,
    /// The module asked for more memory than the ceiling.
    Memory,
    Trap(String),
    /// The module is not valid WebAssembly, imports something the engine
    /// does not provide, or lacks a required export.
    Module(String),
}

impl VmError {
    pub const CODES: [&'static str; 4] = [
        "E_GAME_BUDGET",
        "E_GAME_MEMORY",
        "E_GAME_TRAP",
        "E_GAME_MODULE",
    ];

    pub fn code(&self) -> &'static str {
        match self {
            VmError::Budget => "E_GAME_BUDGET",
            VmError::Memory => "E_GAME_MEMORY",
            VmError::Trap(_) => "E_GAME_TRAP",
            VmError::Module(_) => "E_GAME_MODULE",
        }
    }
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::Budget => write!(f, "the game used its whole execution budget in one step"),
            VmError::Memory => write!(f, "the game asked for more memory than its ceiling"),
            VmError::Trap(why) => write!(f, "the game trapped: {why}"),
            VmError::Module(why) => write!(f, "the game module cannot run: {why}"),
        }
    }
}

impl std::error::Error for VmError {}

/// What an interpreter backend runs: the small interface the engine needs,
/// so interpreters can be swapped and measured against each other.
pub trait Machine {
    /// Runs `sariema_tick` with `fuel`.
    fn tick(&mut self, fuel: u64) -> Result<(), VmError>;
    /// The value of `sariema_batch_version`, if exported.
    fn batch_version(&mut self, fuel: u64) -> Result<Option<u32>, VmError>;
    fn host(&mut self) -> &mut host::HostState;
    fn host_ref(&self) -> &host::HostState;
    fn memory(&self) -> &[u8];
}

pub trait Interpreter {
    const NAME: &'static str;
    fn load(wasm: &[u8], settings: &VmSettings) -> Result<Box<dyn Machine>, VmError>;
}

pub struct Vm {
    machine: Box<dyn Machine>,
    settings: VmSettings,
    interpreter: &'static str,
    ticks: u64,
    stopped: Option<VmError>,
}

impl Vm {
    /// Loads with the default interpreter, [`Wasmi`].
    pub fn load(wasm: &[u8], settings: VmSettings) -> Result<Vm, VmError> {
        Vm::load_with::<Wasmi>(wasm, settings)
    }

    pub fn load_with<I: Interpreter>(wasm: &[u8], settings: VmSettings) -> Result<Vm, VmError> {
        Ok(Vm {
            machine: I::load(wasm, &settings)?,
            settings,
            interpreter: I::NAME,
            ticks: 0,
            stopped: None,
        })
    }

    pub fn interpreter(&self) -> &'static str {
        self.interpreter
    }

    pub fn batch_version(&mut self) -> Result<Option<u32>, VmError> {
        self.machine.batch_version(self.settings.fuel_per_step)
    }

    /// Runs one tick. `events` is this step's event batch and `ends` the end
    /// offset of each of its records. On success `commands` holds the batch
    /// the game submitted; on failure it is empty and the game stays stopped.
    pub fn tick(
        &mut self,
        events: &[u8],
        ends: &[u32],
        commands: &mut Vec<u8>,
    ) -> Result<(), VmError> {
        commands.clear();
        if let Some(e) = &self.stopped {
            return Err(e.clone());
        }
        self.machine.host().begin(events, ends);
        let fuel = if self.ticks == 0 {
            self.settings.fuel_first_step
        } else {
            self.settings.fuel_per_step
        };
        self.ticks += 1;
        match self.machine.tick(fuel) {
            Ok(()) => {
                std::mem::swap(&mut self.machine.host().commands, commands);
                Ok(())
            }
            Err(e) => {
                self.stopped = Some(e.clone());
                Err(e)
            }
        }
    }

    pub fn stopped(&self) -> Option<&VmError> {
        self.stopped.as_ref()
    }

    /// The registered regions, as slices of memory, in registration order.
    pub fn regions(&self) -> impl Iterator<Item = (Region, &[u8])> {
        let memory = self.machine.memory();
        self.machine.host_ref().regions.iter().map(move |r| {
            let start = r.offset as usize;
            (*r, &memory[start..start + r.len as usize])
        })
    }

    /// Lines the last tick logged.
    pub fn log(&self) -> &[LogLine] {
        &self.machine.host_ref().log
    }

    pub fn memory(&self) -> &[u8] {
        self.machine.memory()
    }
}
