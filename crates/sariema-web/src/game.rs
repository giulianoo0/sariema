// SPDX-License-Identifier: Apache-2.0

//! A game pack played whole in the page: the engine's step (runtime, game
//! logic, collision, animation, sound, HUD) at the runtime's fixed step,
//! drawn by `sariema-view` at the frame's `alpha`. Sound goes to the page's
//! AudioWorklet: the host takes the engine's mixer and mixes the blocks the
//! worklet's reports ask for, while the engine's audio feed runs between
//! steps. A replay instead keeps the mixer in the engine and mixes with the
//! steps, as `sariema run --headless --game` does, so its hashes match.

use std::time::Duration;

use glam::Vec3;
use sariema_audio::Mixer;
use sariema_engine::{Engine, EngineSettings};
use sariema_format::script::{GameAssets, GamePack, decode_game};
use sariema_render::Renderer;
use sariema_runtime::clock::Tick;
use sariema_runtime::game::WasmLogic;
use sariema_runtime::input::{InputLog, RawInput, Replay};
use sariema_ui::SafeArea;
use sariema_view::{GameView, ViewSettings};
use serde_json::{Value, json};

use crate::feed::{Feed, FeedSettings};
use crate::input::runtime_key;
use crate::stage::StageError;

struct Live {
    mixer: Mixer,
    feed: Feed,
    block: Vec<f32>,
}

pub struct GameHost {
    engine: Engine<WasmLogic>,
    view: GameView,
    raw: RawInput,
    last_ms: Option<f64>,
    /// Where the frame drawn lies between the last two steps.
    alpha: f32,
    last_steps: u32,
    size: (u32, u32),
    live: Option<Live>,
    replay: Option<Replay>,
    shadows: bool,
    /// Made by replayed steps, counted with the `alloc-count` feature.
    step_allocations: u64,
    /// Made by page frames' steps and audio feeds, likewise.
    frame_allocations: u64,
}

fn allocations() -> u64 {
    #[cfg(feature = "alloc-count")]
    return crate::alloc::so_far();
    #[cfg(not(feature = "alloc-count"))]
    0
}

fn failed(code: &'static str) -> impl Fn(String) -> StageError {
    move |message| StageError::new(code, message)
}

/// The actors of `pack` the view draws; the engine takes the rest, the
/// rooms included, which the view draws from it.
fn drawn(pack: &GamePack) -> GameAssets {
    GameAssets {
        actors: pack.assets.actors.clone(),
        ..GameAssets::default()
    }
}

impl GameHost {
    /// Starts game pack `bytes` with `seed`. With a `sample_rate` its sound
    /// is mixed live at that rate for the worklet in `feed`'s blocks;
    /// without one it is mixed with the steps, for a replay.
    pub fn load(
        bytes: &[u8],
        seed: u64,
        sample_rate: Option<u32>,
        feed: FeedSettings,
        renderer: &mut Renderer,
        size: (u32, u32),
        shadows: bool,
    ) -> Result<GameHost, StageError> {
        let pack = decode_game(bytes).map_err(|e| StageError::new(e.code(), e.to_string()))?;
        let view_assets = drawn(&pack);
        let mut settings = EngineSettings::default();
        if let Some(rate) = sample_rate {
            settings.mixer.sample_rate = rate;
        }
        let mut engine =
            Engine::load(pack, seed, settings).map_err(|e| StageError::new(e.code, e.message))?;
        let live = match sample_rate {
            Some(rate) => engine.take_mixer().map(|mixer| {
                let feed = Feed::new(feed, rate);
                let block = vec![0.0; 2 * feed.block_frames() as usize];
                Live { mixer, feed, block }
            }),
            None => None,
        };
        let view = view(renderer, &view_assets, &engine, shadows)?;
        let mut host = GameHost {
            engine,
            view,
            raw: RawInput::default(),
            last_ms: None,
            alpha: 0.0,
            last_steps: 0,
            size,
            live,
            replay: None,
            shadows,
            step_allocations: 0,
            frame_allocations: 0,
        };
        host.resize(size.0, size.1);
        Ok(host)
    }

    /// Loads the room and actors of game pack `bytes` into a new renderer,
    /// as after a lost device; the game itself goes on where it was.
    pub fn reload_view(&mut self, bytes: &[u8], renderer: &mut Renderer) -> Result<(), StageError> {
        let pack = decode_game(bytes).map_err(|e| StageError::new(e.code(), e.to_string()))?;
        self.view = view(renderer, &drawn(&pack), &self.engine, self.shadows)?;
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        (self.engine).resize(width as f32, height as f32, SafeArea::default());
    }

    /// `code` is the DOM `KeyboardEvent.code`; false for keys no game reads.
    pub fn key(&mut self, code: &str, down: bool) -> bool {
        match runtime_key(code) {
            Some(key) => {
                self.raw.set_key(key, down);
                true
            }
            None => false,
        }
    }

    /// Positions in physical pixels from the canvas's corner.
    pub fn pointer_down(&mut self, id: u64, x: f32, y: f32) {
        self.engine.pointer_down(id, x, y);
    }

    pub fn pointer_move(&mut self, id: u64, x: f32, y: f32) {
        self.engine.pointer_move(id, x, y);
    }

    pub fn pointer_up(&mut self, id: u64, x: f32, y: f32) {
        self.engine.pointer_up(id, x, y);
    }

    /// Runs the steps a display frame at `now_ms` owes, then the audio feed.
    pub fn advance(&mut self, now_ms: f64) -> Result<Tick, StageError> {
        let dt = self.last_ms.map_or(0.0, |last| (now_ms - last).max(0.0));
        self.last_ms = Some(now_ms);
        let before = allocations();
        let tick = self
            .engine
            .frame(Duration::from_secs_f64(dt / 1000.0), &self.raw);
        self.frame_allocations += allocations() - before;
        self.alpha = self.engine.alpha();
        self.last_steps = tick.steps;
        self.check()?;
        Ok(tick)
    }

    /// Forgets the time since the last frame: after the page was hidden the
    /// game resumes where it stopped.
    pub fn pause(&mut self) {
        self.last_ms = None;
    }

    /// Steps the game from where it is to step `to` with the actions of
    /// `log`, each step followed by the audio feed, as a headless run does.
    pub fn replay(&mut self, log: &str, to: u64) -> Result<(), StageError> {
        if self.replay.is_none() {
            let log: InputLog =
                serde_json::from_str(log).map_err(|e| failed("E_SCHEMA")(e.to_string()))?;
            let input = &self.engine.state().settings.input;
            let replay = Replay::new(&log, input).map_err(|e| failed("E_SCHEMA")(e.to_string()))?;
            self.replay = Some(replay);
        }
        let replay = self.replay.as_mut().expect("set above");
        for step in self.engine.steps()..to {
            let before = allocations();
            self.engine.step(replay.actions(step));
            self.step_allocations += allocations() - before;
            self.engine.feed_audio();
        }
        self.alpha = 1.0;
        self.check()
    }

    fn check(&self) -> Result<(), StageError> {
        match self.engine.simulation().stopped() {
            Some(stop) => Err(StageError::new(stop.code, stop.message.clone())),
            None => Ok(()),
        }
    }

    /// The frame `alpha` of the way to the next step, into `target`.
    pub fn draw(
        &mut self,
        renderer: &mut Renderer,
        target: &wgpu::TextureView,
    ) -> Result<(), StageError> {
        let aspect = self.size.0 as f32 / self.size.1.max(1) as f32;
        self.view
            .draw(renderer, &mut self.engine, self.alpha, aspect, target)
            .map_err(|e| StageError::new(e.code, e.message))
    }

    /// `frames` counts what the output has played since the game started;
    /// `out` gets the blocks due, interleaved stereo.
    pub fn mix(&mut self, frames: u64, out: &mut Vec<f32>) {
        out.clear();
        let Some(live) = &mut self.live else {
            return;
        };
        live.feed.played(frames);
        for _ in 0..live.feed.due_blocks() {
            live.mixer.render(&mut live.block);
            out.extend_from_slice(&live.block);
            live.feed.mixed_block();
        }
    }

    pub fn steps(&self) -> u64 {
        self.engine.steps()
    }

    /// The hashes `sariema run --headless --game` prints.
    pub fn hashes(&self) -> Value {
        let hex = |h: u64| format!("{h:016x}");
        json!({
            "steps": self.engine.steps(),
            "engine_hash": hex(self.engine.state_hash()),
            "state_hash": hex(self.engine.simulation().state_hash()),
            "physics_hash": hex(self.engine.physics().map_or(0, |p| p.world.state_hash())),
            "audio_hash": hex(self.engine.sound().map_or(0, |s| s.hash())),
        })
    }

    pub fn stats(&self) -> Value {
        let s = self.engine.state();
        let hero = s.world.pose(s.control.hero).map(|p| {
            let Vec3 { x, y, z } = p.position;
            [x, y, z]
        });
        json!({
            "steps": self.engine.steps(),
            "last_steps": self.last_steps,
            "hero": hero,
            "stopped": self.engine.simulation().stopped().map(|s| s.code),
            "audio_queued_frames": self.live.as_ref().map(|l| l.feed.queued()),
            "step_allocations": cfg!(feature = "alloc-count").then_some(self.step_allocations),
            "frame_allocations": cfg!(feature = "alloc-count").then_some(self.frame_allocations),
        })
    }
}

fn view(
    renderer: &mut Renderer,
    assets: &GameAssets,
    engine: &Engine<WasmLogic>,
    shadows: bool,
) -> Result<GameView, StageError> {
    let settings = ViewSettings {
        shadows,
        ..ViewSettings::default()
    };
    GameView::load_with(renderer, assets, engine, settings)
        .map_err(|e| StageError::new(e.code, e.message))
}
