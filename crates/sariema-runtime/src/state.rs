// SPDX-License-Identifier: Apache-2.0

//! Everything one step reads and writes, owned in one place so commands
//! from the game and from the sequencer go through the same code.

use sariema_format::script::{Controller, Entity, Event};

use crate::camera::Camera;
use crate::clips::ClipClock;
use crate::events::{EventQueue, Happening};
use crate::follow::Followers;
use crate::hash::StateHasher;
use crate::input::Input;
use crate::motion::{Goal, Motion};
use crate::outbox::Outbox;
use crate::rng::Pcg32;
use crate::screen::{Dialogue, Fader, Screen};
use crate::settings::Settings;
use crate::store::Store;
use crate::timer::Timers;
use crate::tween::Tweens;
use crate::world::World;
use crate::zone::Zones;

/// Who moves the hero, and how fast the player's axis moves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub hero: Entity,
    pub speed: f32,
    pub by: Controller,
}

/// What `cutscene_*` and `signal` commands ask of the sequencer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutsceneRequest {
    Bind { role: u32, entity: Entity },
    Play(u32),
    Skip,
}

pub struct State {
    pub settings: Settings,
    pub world: World,
    pub store: Store,
    pub tweens: Tweens,
    pub followers: Followers,
    pub timers: Timers,
    pub zones: Zones,
    pub camera: Camera,
    pub motion: Motion,
    pub screen: Screen,
    pub dialogue: Dialogue,
    /// World time per real time; see [`State::time_scale`].
    pub time_scale: Fader,
    pub clips: ClipClock,
    pub control: Control,
    pub input: Input,
    pub outbox: Outbox,
    pub rng: Pcg32,
    /// Events for the game's next tick.
    pub events: EventQueue,
    /// What happened this step and the step before, numbered in order, for
    /// the sequencer's waits.
    pub happened: Vec<(u64, Happening)>,
    pub seen: Vec<(u64, Happening)>,
    pub happenings: u64,
    pub requests: Vec<CutsceneRequest>,
}

impl State {
    pub fn new(settings: Settings, seed: u64) -> State {
        State {
            world: World::new(settings.max_entities),
            store: Store::default(),
            tweens: Tweens::default(),
            followers: Followers::default(),
            timers: Timers::default(),
            zones: Zones::default(),
            camera: Camera::new(settings.camera),
            motion: Motion::default(),
            screen: Screen::default(),
            dialogue: Dialogue::default(),
            time_scale: Fader::at(1.0),
            clips: ClipClock::default(),
            control: Control {
                hero: Entity::NONE,
                speed: 0.0,
                by: Controller::Player,
            },
            input: Input::new(settings.input.clone()),
            outbox: Outbox::default(),
            rng: Pcg32::new(seed, 0x5a51),
            events: EventQueue::default(),
            happened: Vec::new(),
            seen: Vec::new(),
            happenings: 0,
            requests: Vec::new(),
            settings,
        }
    }

    /// How fast the world plays against real time: the display frames'
    /// time is multiplied by it before it becomes steps.
    pub fn time_scale(&self) -> f32 {
        self.time_scale.value()
    }

    /// The clip `entity` plays and the time the animation module must
    /// show it at, in seconds.
    pub fn clip_time(&self, entity: Entity) -> Option<(u32, f32)> {
        self.clips.time(entity)
    }

    pub fn happen(&mut self, h: Happening) {
        self.happened.push((self.happenings, h));
        self.happenings += 1;
    }

    /// Whether `h` happened at or after the happening numbered `since`,
    /// this step or the one before.
    pub fn saw(&self, h: Happening, since: u64) -> bool {
        self.seen
            .iter()
            .chain(&self.happened)
            .any(|&(n, x)| n >= since && x == h)
    }

    /// Hands the hero over: stops what the player's axis was doing and
    /// clears the input buffer, so no press made before carries over.
    pub fn hand_over(&mut self, by: Controller) {
        if self.control.by == by {
            return;
        }
        if self.control.by == Controller::Player && self.world.is_alive(self.control.hero) {
            self.motion.mover_mut(self.control.hero).goal = Goal::Idle;
        }
        self.control.by = by;
        self.input.clear_buffer();
        self.events.push(Event::Control { by });
    }

    pub fn end_line(&mut self, line: u32) {
        self.events.push(Event::LineDone { line });
        self.happen(Happening::LineDone(line));
    }

    /// The world as the player sees it: entities, flags, variables, the
    /// screen, the camera and who controls the hero. Leaves out time.
    pub fn world_hash(&self) -> u64 {
        let mut h = StateHasher::default();
        for (e, n) in self.world.entities() {
            h.u32(e.index);
            h.u32(e.generation);
            h.u32(n.kind as u32);
            h.u32(n.asset);
            h.vec3(n.local.position);
            for v in [n.local.yaw, n.local.pitch, n.local.roll, n.local.scale] {
                h.f32(v);
            }
            h.u32(n.parent.index);
            h.u32(n.visible as u32);
            n.tags.iter().for_each(|t| h.u32(*t));
        }
        self.store.flags.iter().for_each(|f| h.u32(*f));
        for (k, v) in &self.store.vars {
            h.u32(*k);
            h.f32(*v);
        }
        h.f32(self.screen.fade.value());
        h.f32(self.screen.letterbox.value());
        self.camera.hash_into(&mut h);
        h.u32(self.control.hero.index);
        h.u32(self.control.by as u32);
        h.finish()
    }
}
