// SPDX-License-Identifier: Apache-2.0

//! Everything one step reads and writes, owned in one place so commands
//! from the game and from the sequencer go through the same code.

use sariema_format::script::{Controller, Entity, Event, Kind};

use crate::aim::Aims;
use crate::camera::Camera;
use crate::clips::ClipClock;
use crate::events::{EventQueue, Happening};
use crate::follow::Followers;
use crate::fx::Fx;
use crate::hash::StateHasher;
use crate::hold::Holds;
use crate::input::Input;
use crate::motion::{Goal, Motion};
use crate::outbox::Outbox;
use crate::paths::Paths;
use crate::rng::Pcg32;
use crate::scenery::Scenery;
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
    pub scenery: Scenery,
    /// Particles: drawn, never hashed.
    pub fx: Fx,
    pub followers: Followers,
    pub holds: Holds,
    pub aims: Aims,
    pub timers: Timers,
    pub zones: Zones,
    pub camera: Camera,
    pub motion: Motion,
    pub screen: Screen,
    pub dialogue: Dialogue,
    /// World time per real time; see [`State::time_scale`].
    pub time_scale: Fader,
    pub clips: ClipClock,
    pub paths: Paths,
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
    /// Entities `locate` asked for this step, answered at its end.
    pub locates: Vec<Entity>,
}

impl State {
    pub fn new(settings: Settings, seed: u64) -> State {
        State {
            world: World::new(settings.max_entities),
            store: Store::default(),
            tweens: Tweens::default(),
            scenery: Scenery::default(),
            fx: Fx::new(&settings.fx),
            followers: Followers::default(),
            holds: Holds::default(),
            aims: Aims::default(),
            timers: Timers::default(),
            zones: Zones::default(),
            camera: Camera::new(settings.camera, &settings.motion),
            motion: Motion::default(),
            screen: Screen::default(),
            dialogue: Dialogue::default(),
            time_scale: Fader::at(1.0),
            clips: ClipClock::default(),
            paths: Paths::default(),
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
            locates: Vec::new(),
            settings,
        }
    }

    /// How fast the world plays against real time: the display frames'
    /// time is multiplied by it before it becomes steps.
    pub fn time_scale(&self) -> f32 {
        self.time_scale.value()
    }

    /// The clip `entity` plays and its time at the last step, in seconds:
    /// what markers and the end of the clip are judged against.
    pub fn clip_time(&self, entity: Entity) -> Option<(u32, f32)> {
        self.clips.time(entity)
    }

    /// The clip time to draw at `alpha` between the last two steps
    /// ([`Simulation::alpha`](crate::sim::Simulation::alpha)), interpolated
    /// like every pose, so a slowed clip never shows a still frame.
    pub fn clip_time_at(&self, entity: Entity, alpha: f32) -> Option<(u32, f32)> {
        self.clips.time_at(entity, alpha)
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

    /// Whether `e` may be held: an actor of its own, since the collision
    /// module poses a carried one where its carrier holds it.
    pub fn may_hold(&self, e: Entity) -> bool {
        self.world
            .node(e)
            .is_some_and(|n| n.kind == Kind::Actor && n.parent.is_none())
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

    /// The world as the player sees it: entities, an actor held where a
    /// kinematic move ended, flags, variables, the screen, the camera, the
    /// rooms and who controls the hero. Leaves out time. A hold counts only
    /// while one is held, so a world that never holds hashes as before.
    pub fn world_hash(&self) -> u64 {
        const HELD: u32 = u32::MAX;
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
            if self.motion.held(e) {
                h.u32(HELD);
            }
        }
        self.store.flags.iter().for_each(|f| h.u32(*f));
        for (k, v) in &self.store.vars {
            h.u32(*k);
            h.f32(*v);
        }
        h.f32(self.screen.fade.value());
        h.f32(self.screen.letterbox.value());
        self.camera.hash_into(&mut h);
        self.scenery.hash_into(&mut h);
        h.u32(self.control.hero.index);
        h.u32(self.control.by as u32);
        h.finish()
    }
}
