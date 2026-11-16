// SPDX-License-Identifier: Apache-2.0

//! The Sariema engine as one: the game runtime stepped with the collision,
//! animation, sound and HUD modules wired in, at a fixed 1/60 s of world
//! time, with nothing on the step path that needs a thread, the wall clock
//! or a GPU. A host draws what [`Engine::camera`], [`Engine::actors`] and
//! [`Engine::draw_hud`] give at the frame's `alpha`. `docs/engine.md` has
//! the order of a step and every hand-off.

mod anim;
mod assets;
mod audio;
mod hud;
mod physics;
mod settings;
mod view;

use std::time::Duration;

use sariema_format::script::cutscene::Block;
use sariema_format::script::{Event, GamePack};
use sariema_runtime::clock::Tick;
use sariema_runtime::game::{LoadError, WasmLogic};
use sariema_runtime::input::{Actions, RawInput};
use sariema_runtime::sim::{Logic, Modules, Simulation};
use sariema_runtime::state::State;
use sariema_ui::SafeArea;

pub use anim::{Animation, Model};
pub use assets::Assets;
pub use audio::Sound;
pub use hud::{Hud, KEEP_IN_CUTSCENES};
pub use physics::{Physics, to_physics, to_runtime};
pub use settings::{EngineSettings, HudSettings, SoundSettings};
pub use view::ActorView;

/// Every code the modules wired in here give the game in a `rejected`
/// event, besides the runtime's own.
pub const REJECTED: &[&str] = &[
    "E_TELEPORT_BLOCKED",
    "E_COLLIDER_BLOCKED",
    "E_AUDIO_UNKNOWN_SOUND",
    "E_AUDIO_NOT_LOADED",
    "E_AUDIO_QUEUE_FULL",
    "E_AUDIO_NOT_FINITE",
    "E_AUDIO_DECODE",
    "E_UI_UNKNOWN",
    "E_UI_FONT",
    "E_UI_ATLAS_FULL",
    "E_UI_ANIMATIONS",
    "E_UI_TEXT_TOO_LONG",
];

pub struct Engine<L> {
    sim: Simulation<L>,
    anim: Animation,
    physics: Option<Physics>,
    sound: Option<Sound>,
    hud: Hud,
    actions: Actions,
    touch: Actions,
    rejected: Vec<Event<'static>>,
}

/// The modules run inside the runtime's step, where its built-in mover
/// would: animation first, as a traversal's contact needs this step's pose.
struct Stages<'a> {
    anim: &'a mut Animation,
    physics: Option<&'a mut Physics>,
}

impl Modules for Stages<'_> {
    fn step(&mut self, s: &mut State) {
        self.anim.step(s);
        if let Some(physics) = self.physics.as_deref_mut() {
            physics.step(s, self.anim);
        }
    }
}

impl Engine<WasmLogic> {
    /// A game pack with the assets it carries. Its actors move under the
    /// collision module when it carries a room's collision.
    pub fn load(
        mut pack: GamePack,
        seed: u64,
        settings: EngineSettings,
    ) -> Result<Self, LoadError> {
        let assets = Assets::decode(std::mem::take(&mut pack.assets))?;
        let mut runtime = sariema_runtime::game::pack_settings(&pack)?;
        if assets.collision.is_some() {
            runtime.motion.builtin = false;
        }
        let mut lines = Vec::new();
        for c in &pack.cutscenes {
            cutscene_lines(&c.body, &mut lines);
        }
        let lines: Vec<String> = lines.into_iter().map(String::from).collect();
        let sim = sariema_runtime::game::start_with_settings(pack, runtime, seed)?;
        let mut engine = Engine::new(sim, assets, settings)?;
        for text in &lines {
            engine.hud.prepare_line(text)?;
        }
        Ok(engine)
    }
}

fn cutscene_lines<'a>(blocks: &'a [Block], out: &mut Vec<&'a str>) {
    for block in blocks {
        match block {
            Block::Line { text, .. } => out.push(text),
            Block::Sequence(blocks) => cutscene_lines(blocks, out),
            Block::Parallel(branches) => branches.iter().for_each(|b| cutscene_lines(b, out)),
            Block::Branch {
                then, otherwise, ..
            } => {
                cutscene_lines(then, out);
                cutscene_lines(otherwise, out);
            }
            _ => {}
        }
    }
}

impl<L: Logic> Engine<L> {
    /// An engine over `sim`; with a room's collision among `assets`, the
    /// runtime's own mover must be off (`motion.builtin`), or `E_SETTINGS`.
    pub fn new(
        sim: Simulation<L>,
        assets: Assets,
        settings: EngineSettings,
    ) -> Result<Self, LoadError> {
        let s = sim.state();
        if assets.collision.is_some() && s.settings.motion.builtin {
            return Err(LoadError {
                code: "E_SETTINGS",
                message: "a room's collision moves the actors, so motion.builtin must be off"
                    .into(),
            });
        }
        let failed = |code: &'static str| move |message: String| LoadError { code, message };
        let models = assets
            .actors
            .iter()
            .map(|(name, pack)| Model::new(name, pack).map_err(failed("E_SCHEMA")))
            .collect::<Result<Vec<_>, _>>()?;
        let physics = match assets.collision {
            Some(pack) => Some(
                Physics::new(pack, &settings, s.settings.motion.turn_rate)
                    .map_err(failed("E_SCHEMA"))?,
            ),
            None => None,
        };
        let sound = match assets.sounds {
            Some(bank) => Some(
                Sound::new(bank, settings.mixer.clone(), settings.audio).map_err(|e| {
                    LoadError {
                        code: e.code(),
                        message: e.to_string(),
                    }
                })?,
            ),
            None => None,
        };
        let input = &s.settings.input;
        let hud = match &assets.hud {
            Some(hud) => Hud::new(
                Some(&hud.description),
                |file| {
                    hud.files
                        .iter()
                        .find(|(n, _)| n == file)
                        .map(|(_, bytes)| bytes.clone())
                        .ok_or_else(|| format!("{file} is not in the game pack"))
                },
                input,
                settings.hud,
            )
            .map_err(|e| LoadError {
                code: e.code(),
                message: e.to_string(),
            })?,
            None => Hud::bare(input, settings.hud),
        };
        Ok(Engine {
            anim: Animation::new(models, settings.anim.clone()),
            actions: Actions::for_settings(input),
            touch: Actions::for_settings(input),
            physics,
            sound,
            hud,
            rejected: Vec::new(),
            sim,
        })
    }

    /// One step with `actions`, as a headless run or an input log gives them;
    /// the HUD's joystick and buttons are added to them. It allocates
    /// nothing once the game is steady; [`Engine::feed_audio`] follows it.
    pub fn step(&mut self, actions: &Actions) {
        self.actions.clone_from(actions);
        self.run_step();
    }

    /// The sound's work between steps: plays what the steps since the last
    /// feed left (a sound loaded on demand, a stream to open), decodes the
    /// streams ahead and, without a device, mixes those steps' samples. A
    /// host calls it after its steps and before drawing; its refusals reach
    /// the game at the next step.
    pub fn feed_audio(&mut self) {
        let Some(sound) = &mut self.sound else {
            return;
        };
        sound.feed(self.sim.steps());
        for event in sound.rejected.drain(..) {
            self.sim.report(event);
        }
    }

    /// The steps a display frame of `dt` owes, each sampling `raw`, then the
    /// audio feed.
    pub fn frame(&mut self, dt: Duration, raw: &RawInput) -> Tick {
        let tick = self.sim.advance(dt);
        for _ in 0..tick.steps {
            self.sim.state().input.map(raw, &mut self.actions);
            self.run_step();
        }
        self.feed_audio();
        tick
    }

    fn run_step(&mut self) {
        let n = self.sim.steps();
        if let Some(touch) = self.hud.input(&mut self.actions) {
            let mut raw = RawInput::default();
            raw.touch = Some(touch);
            self.sim.state().input.map(&raw, &mut self.touch);
            for (axis, &t) in self.actions.axes.iter_mut().zip(&self.touch.axes) {
                if t != glam::Vec2::ZERO {
                    *axis = t;
                }
            }
        }
        let mut stages = Stages {
            anim: &mut self.anim,
            physics: self.physics.as_mut(),
        };
        self.sim.step_with(&self.actions, &mut stages);
        let s = self.sim.state();
        if let Some(physics) = &mut self.physics {
            physics.after_step(s);
        }
        if let Some(sound) = &mut self.sound {
            let at = if sound.offline() {
                sound.frame_of(n)
            } else {
                0
            };
            sound.step(s, at);
            self.rejected.append(&mut sound.rejected);
        }
        self.hud.step(s);
        self.rejected.append(&mut self.hud.rejected);
        for event in self.rejected.drain(..) {
            self.sim.report(event);
        }
    }

    pub fn simulation(&self) -> &Simulation<L> {
        &self.sim
    }

    pub fn simulation_mut(&mut self) -> &mut Simulation<L> {
        &mut self.sim
    }

    pub fn state(&self) -> &State {
        self.sim.state()
    }

    pub fn steps(&self) -> u64 {
        self.sim.steps()
    }

    pub fn alpha(&self) -> f32 {
        self.sim.alpha()
    }

    pub fn physics(&self) -> Option<&Physics> {
        self.physics.as_ref()
    }

    pub fn animation(&self) -> &Animation {
        &self.anim
    }

    pub fn sound(&self) -> Option<&Sound> {
        self.sound.as_ref()
    }

    pub fn hud(&self) -> &Hud {
        &self.hud
    }

    /// The HUD's screen, for a host uploading its glyph atlas and images.
    pub fn hud_mut(&mut self) -> &mut Hud {
        &mut self.hud
    }

    /// The mixer, for an audio device to pull from; sound is then live and
    /// no longer mixed with the steps.
    pub fn take_mixer(&mut self) -> Option<sariema_audio::Mixer> {
        self.sound.as_mut()?.take_mixer()
    }

    pub fn resize(&mut self, width: f32, height: f32, safe: SafeArea) {
        self.hud.resize(width, height, safe);
    }

    /// Pointer positions are physical pixels from the top-left.
    pub fn pointer_down(&mut self, pointer: u64, x: f32, y: f32) {
        self.hud.pointer_down(pointer, x, y);
    }

    pub fn pointer_move(&mut self, pointer: u64, x: f32, y: f32) {
        self.hud.pointer_move(pointer, x, y);
    }

    pub fn pointer_up(&mut self, pointer: u64, x: f32, y: f32) {
        self.hud.pointer_up(pointer, x, y);
    }

    /// What every host prints of a run, as `sariema run --headless --game`
    /// does: the steps and the hashes in hex, the physics and audio ones
    /// null for a game without them.
    pub fn hashes(&self) -> serde_json::Value {
        let hex = |h: u64| format!("{h:016x}");
        serde_json::json!({
            "steps": self.steps(),
            "engine_hash": hex(self.state_hash()),
            "state_hash": hex(self.sim.state_hash()),
            "physics_hash": self.physics.as_ref().map(|p| hex(p.world.state_hash())),
            "audio_hash": self.sound.as_ref().map(|s| hex(s.hash())),
        })
    }

    /// The runtime's state, the collision world's and every animator's
    /// clip and time: what the next steps depend on.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::default();
        h.u64(self.sim.state_hash());
        if let Some(physics) = &self.physics {
            h.u64(physics.world.state_hash());
        }
        for (e, clip, time) in self.anim.clip_times() {
            h.u64(u64::from(e.index) << 32 | u64::from(e.generation));
            h.u64(clip.map_or(u64::MAX, |c| u64::from(c.0)));
            h.u64(u64::from(time.to_bits()));
        }
        h.0
    }
}

struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
