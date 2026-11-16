// SPDX-License-Identifier: Apache-2.0

//! The fixed-step simulation: one tick of the game's logic per step, its
//! commands applied in order, then the engine's own systems.
//!
//! A step, in order: input is sampled; the game reads the events of the
//! last step plus this step's input and writes its commands; the commands
//! and bulk regions are applied; the cutscene sequencer runs; then the
//! hero's control, movement, followers, tweens, timers, the screen, the
//! dialogue, the camera, world placement and trigger zones.

use std::time::Duration;

use glam::Vec3;
use sariema_format::script::cutscene::Cutscene;
use sariema_format::script::{Command, Controller, Entity, Event, Kind, records};

use crate::apply::{Source, apply};
use crate::clock::{FixedClock, Tick};
use crate::cutscene::Sequencer;
use crate::events::{EventQueue, Happening};
use crate::hash::StateHasher;
use crate::input::{Actions, RawInput};
use crate::motion::Goal;
use crate::outbox::{HostIn, HostOut};
use crate::settings::Settings;
use crate::state::State;

/// Why a game stopped for good.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    pub code: &'static str,
    pub message: String,
}

/// A game's logic: the WebAssembly machine, or native code in tests.
pub trait Logic {
    /// One tick: read `events` (records ending at `ends`), write the command
    /// batch into `commands`.
    fn tick(&mut self, events: &[u8], ends: &[u32], commands: &mut Vec<u8>) -> Result<(), Stop>;

    /// Bulk regions after the last tick, by kind.
    fn regions(&self, each: &mut dyn FnMut(u32, &[u8])) {
        let _ = each;
    }

    /// The logic's memory, hashed into the state when there is one.
    fn memory(&self) -> Option<&[u8]> {
        None
    }
}

/// Region kind 1: `u32` count, then that many records of entity index and
/// generation (`u32` each), position (3 × `f32`) and yaw (`f32`), little-endian.
pub const REGION_TRANSFORMS: u32 = 1;
const TRANSFORM_RECORD: usize = 24;

pub struct Simulation<L> {
    state: State,
    logic: L,
    sequencer: Sequencer,
    clock: FixedClock,
    commands: Vec<u8>,
    head: EventQueue,
    actions: Actions,
    incoming: Vec<HostIn>,
    reported: Vec<Event<'static>>,
    step: u64,
    seed: u64,
    stopped: Option<Stop>,
}

impl<L: Logic> Simulation<L> {
    pub fn new(settings: Settings, cutscenes: Vec<Cutscene>, logic: L, seed: u64) -> Simulation<L> {
        let actions = Actions::for_settings(&settings.input);
        Simulation {
            clock: FixedClock::new(settings.max_steps_per_frame),
            state: State::new(settings, seed),
            logic,
            sequencer: Sequencer::new(cutscenes),
            commands: Vec::new(),
            head: EventQueue::default(),
            actions,
            incoming: Vec::new(),
            reported: Vec::new(),
            step: 0,
            seed,
            stopped: None,
        }
    }

    /// Runs the steps one display frame owes, sampling `raw` for each.
    pub fn frame(&mut self, dt: Duration, raw: &RawInput) -> Tick {
        let tick = self.clock.advance(dt);
        for _ in 0..tick.steps {
            let mut actions = std::mem::take(&mut self.actions);
            self.state.input.map(raw, &mut actions);
            self.step(&actions);
            self.actions = actions;
        }
        tick
    }

    /// Like [`Simulation::frame`], with each step's actions given by
    /// `actions(step, out)`, as when replaying a recorded input log.
    pub fn frame_with(&mut self, dt: Duration, mut actions: impl FnMut(u64, &mut Actions)) -> Tick {
        let tick = self.clock.advance(dt);
        for _ in 0..tick.steps {
            let mut a = std::mem::take(&mut self.actions);
            actions(self.step, &mut a);
            self.step(&a);
            self.actions = a;
        }
        tick
    }

    /// How far rendering is between the last two steps.
    pub fn alpha(&self) -> f32 {
        self.clock.alpha()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn logic(&self) -> &L {
        &self.logic
    }

    pub fn steps(&self) -> u64 {
        self.step
    }

    pub fn stopped(&self) -> Option<&Stop> {
        self.stopped.as_ref()
    }

    pub fn playing_cutscene(&self) -> Option<u32> {
        self.sequencer.playing()
    }

    /// A message from the host, delivered as events at the next step.
    pub fn deliver(&mut self, message: HostIn) {
        self.incoming.push(message);
    }

    /// An event from another module (collision, animation), delivered to
    /// the game and the sequencer at the next step.
    pub fn report(&mut self, event: Event<'static>) {
        self.reported.push(event);
    }

    /// The events the logic was given at the last step.
    pub fn delivered(&self) -> &EventQueue {
        &self.head
    }

    /// Messages for the host, oldest first.
    pub fn take_host(&mut self) -> Vec<HostOut> {
        std::mem::take(&mut self.state.outbox.host)
    }

    pub fn step(&mut self, actions: &Actions) {
        let s = &mut self.state;
        s.world.begin_step();
        s.camera.begin_step();
        s.outbox.begin_step();
        std::mem::swap(&mut s.seen, &mut s.happened);
        s.happened.clear();
        let head = &mut self.head;
        head.clear();
        if self.step == 0 {
            head.push(Event::Started { seed: self.seed });
        }
        for message in self.incoming.drain(..) {
            push_host(head, message);
        }
        for event in self.reported.drain(..) {
            happening(s, &event);
            head.push(event);
        }
        head.append(&s.events);
        s.events.clear();
        s.input.step(actions, |e| head.push(e));

        if self.stopped.is_none() {
            match self
                .logic
                .tick(self.head.bytes(), self.head.ends(), &mut self.commands)
            {
                Ok(()) => self.apply_commands(),
                Err(stop) => self.stop(stop),
            }
        }
        let s = &mut self.state;
        if self.stopped.is_none() {
            self.logic.regions(&mut |kind, bytes| {
                if kind == REGION_TRANSFORMS {
                    apply_transforms(s, bytes);
                }
            });
        }
        self.sequencer.step(s);
        drive_hero(s);
        if s.settings.motion.builtin {
            let settings = s.settings.motion;
            let (events, happened, count) = (&mut s.events, &mut s.happened, &mut s.happenings);
            s.motion.step(&mut s.world, &settings, |entity| {
                events.push(Event::Arrived { entity });
                happened.push((*count, Happening::Arrived(entity)));
                *count += 1;
            });
        }
        s.followers.step(&mut s.world);
        let events = &mut s.events;
        let rest = s.settings.spring_rest;
        s.tweens.step(&mut s.world, &mut s.store, rest, |tween| {
            events.push(Event::TweenDone { tween })
        });
        s.timers
            .step(|timer| events.push(Event::TimerFired { timer }));
        s.screen.step();
        if let Some(line) = s.dialogue.step() {
            s.end_line(line);
        }
        s.world.resolve();
        s.camera.step(&s.world);
        let events = &mut s.events;
        s.zones.step(&s.world, |e| events.push(e));
        self.step += 1;
    }

    fn apply_commands(&mut self) {
        let s = &mut self.state;
        for record in records::<Command>(&self.commands) {
            match record {
                Ok(command) => {
                    if let Err(r) = apply(s, &command, Source::Game) {
                        s.events.push(Event::Rejected {
                            opcode: command.opcode(),
                            code: r.code,
                            entity: r.entity,
                        });
                    }
                }
                Err(e) => {
                    let stop = Stop {
                        code: e.code(),
                        message: e.to_string(),
                    };
                    self.stop(stop);
                    return;
                }
            }
        }
    }

    fn stop(&mut self, stop: Stop) {
        self.state.outbox.host.push(HostOut::Stopped {
            code: stop.code,
            message: stop.message.clone(),
        });
        self.stopped = Some(stop);
    }

    /// Everything the simulation's future depends on: the world, the step,
    /// the engine's random state, the cutscene playing and the logic's memory.
    pub fn state_hash(&self) -> u64 {
        let mut h = StateHasher::default();
        h.u64(self.state.world_hash());
        h.u64(self.step);
        let (a, b) = self.state.rng.state();
        h.u64(a);
        h.u64(b);
        h.u32(self.sequencer.playing().unwrap_or(0));
        if let Some(memory) = self.logic.memory() {
            h.bytes(memory);
        }
        h.finish()
    }
}

fn push_host(q: &mut EventQueue, message: HostIn) {
    match message {
        HostIn::Saves(entries) => {
            for (key, value) in &entries {
                q.push(Event::SaveLoaded { key, value });
            }
            q.push(Event::SaveLoadDone {});
        }
        HostIn::Message { topic, payload } => q.push(Event::HostMessage {
            topic: &topic,
            payload: &payload,
        }),
        HostIn::Orientation(orientation) => q.push(Event::OrientationChanged { orientation }),
        HostIn::Exit => q.push(Event::ExitRequested {}),
    }
}

fn happening(s: &mut State, event: &Event<'_>) {
    match *event {
        Event::Arrived { entity } => s.happen(Happening::Arrived(entity)),
        Event::Blocked { entity } => s.happen(Happening::Blocked(entity)),
        Event::ClipFinished { entity, .. } => s.happen(Happening::ClipFinished(entity)),
        _ => {}
    }
}

fn drive_hero(s: &mut State) {
    let hero = s.control.hero;
    if s.control.by != Controller::Player || !s.world.is_alive(hero) {
        return;
    }
    let axis = s.input.axis(0);
    let (mut right, mut forward) = (Vec3::X, Vec3::NEG_Z);
    if s.settings.motion.camera_relative
        && let Some(pose) = s.camera.pose()
    {
        let f = pose.forward();
        let flat = Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z);
        forward = flat;
        right = Vec3::new(-flat.z, 0.0, flat.x);
    }
    let velocity = (right * axis.x + forward * axis.y) * s.control.speed;
    let mover = s.motion.mover_mut(hero);
    mover.goal = if velocity == Vec3::ZERO {
        Goal::Idle
    } else {
        Goal::Velocity(velocity)
    };
}

fn apply_transforms(s: &mut State, bytes: &[u8]) {
    let Some((count, records)) = bytes.split_first_chunk::<4>() else {
        return;
    };
    let count = (u32::from_le_bytes(*count) as usize).min(records.len() / TRANSFORM_RECORD);
    let f = |b: &[u8], i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let u = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    for r in records.as_chunks::<TRANSFORM_RECORD>().0.iter().take(count) {
        let e = Entity {
            index: u(r, 0),
            generation: u(r, 4),
        };
        let (position, yaw) = (Vec3::new(f(r, 8), f(r, 12), f(r, 16)), f(r, 20));
        if !(position.is_finite() && yaw.is_finite()) {
            continue;
        }
        if let Ok(node) = s.world.node_mut(e)
            && node.kind != Kind::Actor
        {
            node.local.position = position;
            node.local.yaw = yaw;
        }
    }
}
