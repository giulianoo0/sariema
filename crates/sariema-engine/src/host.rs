// SPDX-License-Identifier: Apache-2.0

//! The game's messages to the host: taken from the runtime after every step
//! into a queue the host empties, bounded by `host.max_messages` and
//! `host.max_bytes`, a message past them refused with `E_HOST_QUEUE_FULL`.
//! Every `save_load` is answered: by the host when it says it answers saves,
//! within `host.save_load_timeout_s`, or else as no save.

use std::collections::VecDeque;

use sariema_core::STEP_SECONDS;
use sariema_format::script::{Command, Entity, Event, Orientation};
use sariema_runtime::outbox::{HostIn, HostOut};
use sariema_runtime::sim::{Logic, Simulation};

use crate::settings::HostSettings;

pub const QUEUE_FULL: &str = "E_HOST_QUEUE_FULL";

pub struct Host {
    /// Each message with the step that sent it, oldest first.
    queue: VecDeque<(u64, HostOut)>,
    /// Bytes of the queue's keys, values, topics and payloads.
    bytes: usize,
    /// The step of each `save_load` waiting for the host, oldest first.
    loads: VecDeque<u64>,
    answers: bool,
    timeout_steps: u64,
    settings: HostSettings,
}

impl Host {
    pub fn new(settings: HostSettings) -> Host {
        Host {
            queue: VecDeque::with_capacity(settings.max_messages),
            bytes: 0,
            loads: VecDeque::new(),
            answers: false,
            timeout_steps: (settings.save_load_timeout_s / STEP_SECONDS)
                .round()
                .max(0.0) as u64,
            settings,
        }
    }

    /// Takes step `step`'s messages from `sim` and answers the loads no
    /// host will: at once without a save handler, else once they time out.
    /// A message past the queue's bounds is refused into `rejected`; the
    /// game's stop is always kept.
    pub fn after_step<L: Logic>(
        &mut self,
        step: u64,
        sim: &mut Simulation<L>,
        rejected: &mut Vec<Event<'static>>,
    ) {
        let mut unanswered = 0;
        for message in sim.drain_host() {
            let size = size(&message);
            let stop = matches!(message, HostOut::Stopped { .. });
            let full = self.queue.len() >= self.settings.max_messages
                || self.bytes + size > self.settings.max_bytes;
            if full && !stop {
                rejected.push(Event::Rejected {
                    opcode: opcode(&message),
                    code: QUEUE_FULL,
                    entity: Entity::NONE,
                });
                continue;
            }
            if message == HostOut::LoadSaves {
                if self.answers {
                    self.loads.push_back(step);
                } else {
                    unanswered += 1;
                }
            }
            self.bytes += size;
            self.queue.push_back((step, message));
        }
        for _ in 0..unanswered {
            sim.deliver(HostIn::Saves(Vec::new()));
        }
        while let Some(&asked) = self.loads.front()
            && step - asked >= self.timeout_steps
        {
            self.loads.pop_front();
            log::warn!(
                "the save_load of step {asked} had no answer from the host in {} s; answered as no save",
                self.settings.save_load_timeout_s
            );
            sim.deliver(HostIn::Saves(Vec::new()));
        }
    }

    /// Every message since the last call, with the step that sent it.
    pub fn take(&mut self) -> impl Iterator<Item = (u64, HostOut)> + '_ {
        self.bytes = 0;
        self.queue.drain(..)
    }

    /// Whether the host answers `save_load` itself; when it stops, the loads
    /// it left are answered as no save.
    pub fn answers_saves<L: Logic>(&mut self, yes: bool, sim: &mut Simulation<L>) {
        self.answers = yes;
        if !yes {
            for _ in self.loads.drain(..) {
                sim.deliver(HostIn::Saves(Vec::new()));
            }
        }
    }

    /// Answers the oldest load waiting; false when none waits, as when it
    /// timed out.
    pub fn answer<L: Logic>(
        &mut self,
        saves: Vec<(String, Vec<u8>)>,
        sim: &mut Simulation<L>,
    ) -> bool {
        if self.loads.pop_front().is_none() {
            return false;
        }
        sim.deliver(HostIn::Saves(saves));
        true
    }
}

fn size(message: &HostOut) -> usize {
    match message {
        HostOut::Save { key, value } => key.len() + value.len(),
        HostOut::Message { topic, payload } => topic.len() + payload.len(),
        _ => 0,
    }
}

/// The command that sent `message`.
fn opcode(message: &HostOut) -> u16 {
    match message {
        HostOut::Ready => Command::HostReady {},
        HostOut::Exit => Command::HostExit {},
        HostOut::Orientation(_) => Command::HostOrientation {
            orientation: Orientation::Landscape,
        },
        HostOut::Save { .. } => Command::Save {
            key: "",
            value: &[],
        },
        HostOut::LoadSaves => Command::SaveLoad {},
        HostOut::Message { .. } => Command::HostSend {
            topic: "",
            payload: &[],
        },
        HostOut::Stopped { .. } => return 0,
    }
    .opcode()
}
