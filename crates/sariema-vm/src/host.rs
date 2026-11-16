// SPDX-License-Identifier: Apache-2.0

//! The imports a game sees, written once over a plain memory slice so every
//! interpreter backend shares them. Module `sariema`:
//!
//! - `submit(ptr, len)`: appends `len` bytes of commands.
//! - `events(dst, cap) -> n`: copies the next whole events of this step into
//!   `dst`, at most `cap` bytes; returns the bytes copied, 0 once all were
//!   read, or minus the size of the next event when it alone exceeds `cap`.
//! - `register(kind, ptr, len) -> id`: a bulk data region the engine reads
//!   in place after every tick; −1 if refused.
//! - `log(level, ptr, len)`: a line of UTF-8 text.
//! - `button(name) -> index`: the input button whose name has the id
//!   `name`, or −1.
//!
//! An access outside memory traps the call. Submitting more than
//! `commands_bytes_per_step` in one tick stops the game as over budget; log
//! text and lines past their per-tick caps are dropped.

use crate::{VmError, VmSettings};

/// A region of the game's memory the engine reads after each tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub kind: u32,
    pub offset: u32,
    pub len: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// 0 debug, 1 info, 2 warning, 3 error.
    pub level: u8,
    pub text: String,
}

pub struct HostState {
    pub(crate) events: Vec<u8>,
    pub(crate) ends: Vec<u32>,
    pub(crate) cursor: usize,
    pub(crate) commands: Vec<u8>,
    pub(crate) regions: Vec<Region>,
    pub(crate) log: Vec<LogLine>,
    log_left: usize,
    /// The input buttons' name ids, in their order.
    pub(crate) buttons: Vec<u32>,
    /// Why a host call stopped the game, when the reason is not a trap.
    pub(crate) stop: Option<VmError>,
    settings: VmSettings,
}

/// Region kinds the engine knows.
pub const REGION_KINDS: &[u32] = &[1];

fn range(
    memory_len: usize,
    ptr: u32,
    len: u32,
    what: &str,
) -> Result<std::ops::Range<usize>, String> {
    let end = u64::from(ptr) + u64::from(len);
    if end > memory_len as u64 {
        return Err(format!(
            "{what}: bytes {ptr}..{end} are outside the {memory_len}-byte memory"
        ));
    }
    Ok(ptr as usize..end as usize)
}

impl HostState {
    pub(crate) fn new(settings: VmSettings) -> HostState {
        HostState {
            events: Vec::new(),
            ends: Vec::new(),
            cursor: 0,
            commands: Vec::new(),
            regions: Vec::new(),
            log: Vec::new(),
            log_left: settings.log_bytes_per_step as usize,
            buttons: Vec::new(),
            stop: None,
            settings,
        }
    }

    pub(crate) fn button(&self, name: u32) -> i32 {
        self.buttons
            .iter()
            .position(|&b| b == name)
            .map_or(-1, |i| i as i32)
    }

    pub(crate) fn begin(&mut self, events: &[u8], ends: &[u32]) {
        self.events.clear();
        self.events.extend_from_slice(events);
        self.ends.clear();
        self.ends.extend_from_slice(ends);
        self.cursor = 0;
        self.commands.clear();
        self.log.clear();
        self.log_left = self.settings.log_bytes_per_step as usize;
        self.stop = None;
    }

    pub(crate) fn submit(&mut self, memory: &[u8], ptr: u32, len: u32) -> Result<(), String> {
        let r = range(memory.len(), ptr, len, "submit")?;
        if self.commands.len() + r.len() > self.settings.commands_bytes_per_step as usize {
            self.stop = Some(VmError::Budget);
            return Err(format!(
                "submit: more than {} bytes of commands in one step",
                self.settings.commands_bytes_per_step
            ));
        }
        self.commands.extend_from_slice(&memory[r]);
        Ok(())
    }

    pub(crate) fn events(&mut self, memory: &mut [u8], dst: u32, cap: u32) -> Result<i32, String> {
        let out = range(memory.len(), dst, cap, "events")?;
        let start = match self.cursor {
            0 => 0,
            n => self.ends[n - 1] as usize,
        };
        let mut end = start;
        let mut next = self.cursor;
        while next < self.ends.len() && self.ends[next] as usize - start <= cap as usize {
            end = self.ends[next] as usize;
            next += 1;
        }
        if next == self.cursor && next < self.ends.len() {
            let needed = self.ends[next] as usize - start;
            return Ok(-(needed.min(i32::MAX as usize) as i32));
        }
        let n = end - start;
        memory[out.start..out.start + n].copy_from_slice(&self.events[start..end]);
        self.cursor = next;
        Ok(n as i32)
    }

    pub(crate) fn register(&mut self, memory_len: usize, kind: u32, ptr: u32, len: u32) -> i32 {
        if !REGION_KINDS.contains(&kind)
            || range(memory_len, ptr, len, "register").is_err()
            || self.regions.len() >= self.settings.max_regions as usize
        {
            return -1;
        }
        self.regions.push(Region {
            kind,
            offset: ptr,
            len,
        });
        self.regions.len() as i32 - 1
    }

    pub(crate) fn log(
        &mut self,
        memory: &[u8],
        level: u32,
        ptr: u32,
        len: u32,
    ) -> Result<(), String> {
        let r = range(memory.len(), ptr, len, "log")?;
        if self.log_left == 0 || self.log.len() >= self.settings.log_lines_per_step as usize {
            return Ok(());
        }
        let bytes = &memory[r];
        let kept = &bytes[..bytes.len().min(self.log_left)];
        self.log_left -= kept.len();
        self.log.push(LogLine {
            level: level.min(3) as u8,
            text: String::from_utf8_lossy(kept).into_owned(),
        });
        Ok(())
    }
}
