// SPDX-License-Identifier: Apache-2.0

//! Recorded game ticks run under `sariema-vm` (wasmi), natively and from a
//! page's `.wasm`.

use sariema_vm::{Vm, VmSettings};

pub struct Tick {
    pub events: Vec<u8>,
    pub ends: Vec<u32>,
}

fn u32_at(b: &[u8], at: &mut usize) -> u32 {
    let v = u32::from_le_bytes(b[*at..*at + 4].try_into().unwrap());
    *at += 4;
    v
}

/// A ticks file: per tick, the events' length (`u32`), the events, the
/// count of record ends (`u32`) and the ends, little-endian.
pub fn parse_ticks(b: &[u8]) -> Vec<Tick> {
    let mut at = 0;
    let mut ticks = Vec::new();
    while at < b.len() {
        let n = u32_at(b, &mut at) as usize;
        let events = b[at..at + n].to_vec();
        at += n;
        let m = u32_at(b, &mut at) as usize;
        let ends = (0..m).map(|_| u32_at(b, &mut at)).collect();
        ticks.push(Tick { events, ends });
    }
    ticks
}

pub struct Runner {
    vms: Vec<Vm>,
    ticks: Vec<Tick>,
    commands: Vec<u8>,
}

impl Runner {
    /// `copies` machines of `wasm`, each to run every tick of `ticks`.
    pub fn new(
        wasm: &[u8],
        buttons: &[u32],
        ticks: &[u8],
        copies: usize,
    ) -> Result<Runner, String> {
        let mut vms = Vec::new();
        for _ in 0..copies {
            let mut vm = Vm::load(wasm, VmSettings::default()).map_err(|e| e.to_string())?;
            vm.set_buttons(buttons.iter().copied());
            vms.push(vm);
        }
        Ok(Runner {
            vms,
            ticks: parse_ticks(ticks),
            commands: Vec::with_capacity(1 << 16),
        })
    }

    pub fn ticks(&self) -> usize {
        self.ticks.len()
    }

    /// Every tick on every machine; returns the bytes of commands submitted.
    pub fn run(&mut self) -> Result<u64, String> {
        let mut bytes = 0;
        for vm in &mut self.vms {
            for t in &self.ticks {
                vm.tick(&t.events, &t.ends, &mut self.commands)
                    .map_err(|e| e.to_string())?;
                bytes += self.commands.len() as u64;
            }
        }
        Ok(bytes)
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct Bench(super::Runner);

    #[wasm_bindgen]
    impl Bench {
        #[wasm_bindgen(constructor)]
        pub fn new(
            wasm: &[u8],
            buttons: &[u32],
            ticks: &[u8],
            copies: usize,
        ) -> Result<Bench, JsError> {
            super::Runner::new(wasm, buttons, ticks, copies)
                .map(Bench)
                .map_err(|e| JsError::new(&e))
        }

        pub fn ticks(&self) -> usize {
            self.0.ticks()
        }

        pub fn run(&mut self) -> Result<f64, JsError> {
            self.0.run().map(|b| b as f64).map_err(|e| JsError::new(&e))
        }
    }
}
