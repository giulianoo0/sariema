// SPDX-License-Identifier: Apache-2.0

//! A game pack's frame for `bench`: one engine step with the input log's
//! actions and the audio feed, then the room, the actors and the HUD drawn
//! at the step's end. The game's messages to the host are let go, and its
//! loads answered as no save.

use std::path::Path;
use std::time::{Duration, Instant};

use sariema_engine::{Engine, EngineSettings};
use sariema_format::script::GamePack;
use sariema_render::Renderer;
use sariema_runtime::game::WasmLogic;
use sariema_runtime::input::{InputLog, Replay};
use sariema_ui::SafeArea;
use serde_json::{Value, json};

use crate::output::Failure;
use crate::simulate::read_log;
use sariema_view::{GameView, RoomsFrame, ViewSettings};

pub struct GameFrame {
    name: String,
    engine: Engine<WasmLogic>,
    view: GameView,
    replay: Replay,
    aspect: f32,
    /// The last frame's HUD pass.
    hud: HudPass,
}

/// What one frame of a game took on the CPU besides the room's frame.
pub struct Parts {
    pub step: Duration,
    /// The audio feed after the step.
    pub feed: Duration,
    pub hud: Duration,
    /// What loading rooms did in the frame.
    pub rooms: RoomsFrame,
    /// A room switch is under way.
    pub switching: bool,
    /// The step switched rooms.
    pub switched: bool,
    /// Steps run so far, this frame's included.
    pub steps: u64,
}

#[derive(Clone, Copy, Default)]
struct HudPass {
    passes: u32,
    draws: u32,
    bytes_uploaded: u64,
}

impl GameFrame {
    /// The game in `pack` at its first step, its rooms loaded as
    /// `settings` say.
    pub fn load(
        renderer: &mut Renderer,
        settings: ViewSettings,
        pack: GamePack,
        input: Option<&Path>,
        size: (u32, u32),
    ) -> Result<GameFrame, Failure> {
        let (name, assets) = (pack.name.clone(), pack.assets.clone());
        let failed = |e: &dyn std::fmt::Display, code: &'static str| {
            Failure::new(code, format!("{name}: {e}"))
        };
        let mut engine =
            Engine::load(pack, 0, EngineSettings::default()).map_err(|e| failed(&e, e.code))?;
        let view = GameView::load_with(renderer, &assets, &engine, settings)?;
        let log = match input {
            Some(path) => read_log(path)?,
            None => InputLog::default(),
        };
        let replay =
            Replay::new(&log, &engine.state().settings.input).map_err(|e| failed(&e, e.code()))?;
        engine.resize(size.0 as f32, size.1 as f32, SafeArea::default());
        Ok(GameFrame {
            name,
            engine,
            view,
            replay,
            aspect: size.0 as f32 / size.1 as f32,
            hud: HudPass::default(),
        })
    }

    /// Runs the next step and draws the frame at its end into `target`.
    pub fn draw(
        &mut self,
        renderer: &mut Renderer,
        target: &wgpu::TextureView,
    ) -> Result<Parts, Failure> {
        let start = Instant::now();
        let step = self.engine.steps();
        let room = self.engine.state().scenery.active();
        self.engine.step(self.replay.actions(step));
        let stepped = start.elapsed();
        let start = Instant::now();
        self.engine.feed_audio();
        let fed = start.elapsed();
        self.engine.take_host().for_each(drop);
        if let Some(stop) = self.engine.simulation().stopped() {
            return Err(Failure::new(
                stop.code,
                format!("{}: step {step}: {}", self.name, stop.message),
            ));
        }
        let (engine, aspect) = (&mut self.engine, self.aspect);
        self.view.world(renderer, engine, 1.0, aspect, target)?;
        let world = renderer.last_frame_stats();
        let start = Instant::now();
        self.view.hud(renderer, engine, 1.0, target);
        let hud = start.elapsed();
        let all = renderer.last_frame_stats();
        self.hud = HudPass {
            passes: all.passes - world.passes,
            draws: all.draws - world.draws,
            bytes_uploaded: all.bytes_uploaded - world.bytes_uploaded,
        };
        let scenery = &self.engine.state().scenery;
        Ok(Parts {
            step: stepped,
            feed: fed,
            hud,
            rooms: self.view.rooms_frame(),
            switching: scenery.switch().is_some(),
            switched: scenery.active() != room,
            steps: self.engine.steps(),
        })
    }

    pub fn name(&self) -> &String {
        &self.name
    }

    pub fn json(&self) -> Value {
        json!({
            "name": self.name,
            "steps": self.engine.steps(),
            "engine_hash": format!("{:016x}", self.engine.state_hash()),
            "hud_pass": {
                "passes": self.hud.passes,
                "draws": self.hud.draws,
                "bytes_uploaded": self.hud.bytes_uploaded,
            },
        })
    }
}
