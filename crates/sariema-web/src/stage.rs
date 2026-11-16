// SPDX-License-Identifier: Apache-2.0

//! Everything the page drives, apart from the canvas: the renderer, the
//! room's camera, actors, the UI, sounds, the fixed-step clock and the
//! input queue. It draws into any texture view, so it runs and is tested
//! natively as it runs in a browser.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use glam::Mat4;
use sariema_audio::{Audio, Feeding, Mixer, MixerSettings, PlayParams};
use sariema_core::{Camera, room_camera};
use sariema_format::RoomCamera;
use sariema_render::ui::{UiBatch, UiInstance, UiPass, UiTarget, UiTextureFormat};
use sariema_render::{ActorId, ActorModel, ActorState, RenderConfig, RenderError, Renderer};
use sariema_ui::{Event, SafeArea, Ui, UiDesc};
use serde_json::{Value, json};

use crate::clock::{FixedStep, Tick};
use crate::feed::Feed;
use crate::game::Game;
use crate::input::Input;
use crate::pose::{Model, Playing, Posed};
use crate::settings::WebSettings;
use crate::textures;

/// A failure with one of the engine's stable error codes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageError {
    pub code: &'static str,
    pub message: String,
}

impl StageError {
    pub fn new(code: &'static str, message: impl Into<String>) -> StageError {
        StageError {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl From<RenderError> for StageError {
    fn from(e: RenderError) -> StageError {
        let code = match e {
            RenderError::Room(_) | RenderError::Actor(_) => "E_SCHEMA",
            RenderError::NoAdapter | RenderError::Device(_) => "E_GPU",
        };
        StageError::new(code, e.to_string())
    }
}

struct Sound {
    audio: Audio,
    mixer: Mixer,
    feed: Feed,
    block: Vec<f32>,
}

struct UiLayer {
    ui: Ui,
    pass: UiPass,
    batches: Vec<UiBatch>,
}

pub struct Stage {
    settings: WebSettings,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    renderer: Renderer,
    room_camera: Option<RoomCamera>,
    camera: Camera,
    models: Vec<(ActorModel, Model)>,
    actors: Vec<(ActorId, Posed)>,
    ui: Option<UiLayer>,
    sound: Option<Sound>,
    clock: FixedStep,
    input: Input,
    game: Option<Box<dyn Game>>,
    steps: u64,
    events: Vec<Value>,
    last: Tick,
}

impl Stage {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        settings: WebSettings,
        (width, height): (u32, u32),
        format: wgpu::TextureFormat,
    ) -> Result<Stage, StageError> {
        let config = RenderConfig {
            width,
            height,
            output_format: format,
            quality: settings.render_quality(),
        };
        let renderer = Renderer::new(device.clone(), queue.clone(), config)?;
        Ok(Stage {
            clock: FixedStep::new(settings.step),
            settings,
            device,
            queue,
            format,
            size: (width, height),
            renderer,
            room_camera: None,
            camera: Camera {
                view: Mat4::IDENTITY,
                proj: Mat4::IDENTITY,
                position: glam::Vec3::ZERO,
                focus: glam::Vec3::NEG_Z,
            },
            models: Vec::new(),
            actors: Vec::new(),
            ui: None,
            sound: None,
            input: Input::new(),
            game: None,
            steps: 0,
            events: Vec::new(),
            last: Tick {
                steps: 0,
                alpha: 0.0,
            },
        })
    }

    pub fn settings(&self) -> &WebSettings {
        &self.settings
    }

    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn input(&mut self) -> &mut Input {
        &mut self.input
    }

    /// Sets the game the fixed steps run; see [`Game`].
    pub fn set_game(&mut self, game: Option<Box<dyn Game>>) {
        self.game = game;
    }

    /// The draw size in physical pixels; sizes the device cannot hold are ignored.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
        self.size = (width, height);
        self.aim();
        if let Some(layer) = &mut self.ui {
            layer
                .ui
                .set_screen(width as f32, height as f32, SafeArea::default());
        }
    }

    fn aim(&mut self) {
        if let Some(cam) = &self.room_camera {
            let (w, h) = self.size;
            self.camera = room_camera(cam, w as f32 / h.max(1) as f32);
        }
    }

    /// Shows the room of pack `bytes` with its own camera, replacing any
    /// room shown before.
    pub fn load_room(&mut self, bytes: &[u8]) -> Result<(), StageError> {
        let room =
            sariema_format::decode(bytes).map_err(|e| StageError::new(e.code(), e.to_string()))?;
        self.renderer.load_room(&room)?;
        self.room_camera = Some(room.camera.clone());
        self.aim();
        Ok(())
    }

    /// The texture variants of room packs this device samples, preferred first.
    pub fn texture_variants(&self) -> Vec<&'static str> {
        textures::variants(self.device.features())
            .iter()
            .map(|v| v.name())
            .collect()
    }

    /// Loads actor pack `bytes`; returns the model to spawn actors of.
    pub fn load_actor(&mut self, bytes: &[u8]) -> Result<u32, StageError> {
        let pack = sariema_format::actor::decode(bytes)
            .map_err(|e| StageError::new(e.code(), e.to_string()))?;
        let model = Model::new(&pack).map_err(|e| StageError::new("E_SCHEMA", e))?;
        let loaded = self.renderer.load_actor(&pack)?;
        self.models.push((loaded, model));
        Ok(self.models.len() as u32 - 1)
    }

    /// A hidden actor of `model`, until [`Stage::set_actor`] shows it.
    pub fn spawn_actor(&mut self, model: u32) -> Result<u32, StageError> {
        let (handle, m) = (self.models.get(model as usize))
            .ok_or_else(|| StageError::new("E_USAGE", format!("no actor model {model}")))?;
        let id = self.renderer.spawn_actor(*handle)?;
        let joints = m.joints();
        self.actors.push((id, Posed::new(model as usize, joints)));
        Ok(self.actors.len() as u32 - 1)
    }

    /// Places `actor` and plays `clip` on it from `time_s` at `speed`, from
    /// the current step on; no clip holds the bind pose, speed 0 a still pose.
    pub fn set_actor(
        &mut self,
        actor: u32,
        transform: Mat4,
        clip: Option<&str>,
        time_s: f32,
        speed: f32,
        visible: bool,
    ) -> Result<(), StageError> {
        let now = self.now_s();
        let (_, posed) = (self.actors.get_mut(actor as usize))
            .ok_or_else(|| StageError::new("E_USAGE", format!("no actor {actor}")))?;
        let model = &self.models[posed.model].1;
        posed.playing = match clip {
            Some(name) => Some(Playing {
                clip: model
                    .clip(name)
                    .ok_or_else(|| StageError::new("E_USAGE", format!("no clip {name}")))?,
                time_s,
                since_s: now,
                speed,
            }),
            None => None,
        };
        posed.transform = transform;
        posed.visible = visible;
        Ok(())
    }

    /// Builds the UI of description `json`, its fonts and images read from
    /// `files` by the names it gives them.
    pub fn set_ui(
        &mut self,
        json: &str,
        files: &HashMap<String, Vec<u8>>,
    ) -> Result<(), StageError> {
        let desc: UiDesc =
            serde_json::from_str(json).map_err(|e| StageError::new("E_SCHEMA", e.to_string()))?;
        let mut ui = Ui::from_desc(&desc, |name| {
            files
                .get(name)
                .cloned()
                .ok_or_else(|| format!("{name} was not given"))
        })
        .map_err(|e| StageError::new(e.code(), e.to_string()))?;
        let (w, h) = self.size;
        ui.set_screen(w as f32, h as f32, SafeArea::default());
        let mut pass = UiPass::new(
            self.device.clone(),
            self.queue.clone(),
            self.format,
            self.settings.ui_quads,
        );
        for (i, image) in ui.images().iter().enumerate() {
            let format = UiTextureFormat::Rgba8;
            pass.set_texture(1 + i as u32, image.width, image.height, format, &image.rgba);
        }
        self.ui = Some(UiLayer {
            ui,
            pass,
            batches: Vec::new(),
        });
        Ok(())
    }

    /// Loads sound bank `bytes` to mix at `sample_rate`, the page's.
    pub fn load_sounds(&mut self, bytes: &[u8], sample_rate: u32) -> Result<(), StageError> {
        let bank = sariema_format::sound::decode(bytes)
            .map_err(|e| StageError::new(e.code(), e.to_string()))?;
        let base = MixerSettings {
            sample_rate,
            ..MixerSettings::default()
        };
        let (settings, sounds) = sariema_audio::runtime(bank, base);
        let (audio, mixer) = Audio::new(settings, sounds, Feeding::Manual)
            .map_err(|e| StageError::new(e.code(), e.to_string()))?;
        let feed = Feed::new(self.settings.audio, sample_rate);
        let block = vec![0.0; 2 * feed.block_frames() as usize];
        self.sound = Some(Sound {
            audio,
            mixer,
            feed,
            block,
        });
        Ok(())
    }

    /// Plays sound `name` as soon as the mixer can, a lead after now.
    pub fn play(&mut self, name: &str) -> Result<(), StageError> {
        let sound = (self.sound.as_mut())
            .ok_or_else(|| StageError::new("E_USAGE", "no sound bank is loaded"))?;
        let audio_error = |e: sariema_audio::AudioError| StageError::new(e.code(), e.to_string());
        let id = sound.audio.sound(name).map_err(audio_error)?;
        let at = sound.feed.now();
        sound
            .audio
            .play(at, id, PlayParams::default())
            .map_err(audio_error)?;
        Ok(())
    }

    /// Takes the worklet's count of frames played.
    pub fn audio_played(&mut self, frames: u64) {
        if let Some(sound) = &mut self.sound {
            sound.feed.played(frames);
        }
    }

    /// The blocks due to keep the lead, interleaved stereo, one after the
    /// other; empty when none is due or no bank is loaded.
    pub fn mix(&mut self, out: &mut Vec<f32>) {
        out.clear();
        let Some(sound) = &mut self.sound else {
            return;
        };
        for _ in 0..sound.feed.due_blocks() {
            sound.mixer.render(&mut sound.block);
            out.extend_from_slice(&sound.block);
            sound.feed.mixed_block();
        }
    }

    /// Simulation seconds since the stage began.
    pub fn now_s(&self) -> f64 {
        self.steps as f64 * self.settings.step.step_s
    }

    /// Runs the fixed steps due at `now_ms` and draws the frame between the
    /// last two into `target`.
    pub fn frame(&mut self, now_ms: f64, target: &wgpu::TextureView) -> Result<Tick, StageError> {
        let tick = self.clock.advance(now_ms);
        for _ in 0..tick.steps {
            self.step()?;
        }
        self.last = tick;
        self.draw(target)?;
        Ok(tick)
    }

    /// Forgets the time since the last frame: after the page was hidden,
    /// the simulation resumes where it stopped.
    pub fn pause(&mut self) {
        self.clock.reset();
    }

    fn step(&mut self) -> Result<(), StageError> {
        let step_s = self.settings.step.step_s;
        if let Some(game) = &mut self.game {
            game.step(step_s, self.input.queued());
        }
        let ui = self.ui.as_mut().map(|layer| &mut layer.ui);
        self.input.deliver(ui);
        self.steps += 1;
        if let Some(layer) = &mut self.ui {
            let happened = layer.ui.step().to_vec();
            (self.events).extend(
                happened
                    .iter()
                    .map(|e| event_json(&layer.ui, self.steps, e)),
            );
        }
        if let Some(sound) = &mut self.sound {
            sound
                .audio
                .pump()
                .map_err(|e| StageError::new(e.code(), e.to_string()))?;
        }
        Ok(())
    }

    /// Draws the last frame again into `target`, without stepping.
    pub fn draw(&mut self, target: &wgpu::TextureView) -> Result<(), StageError> {
        let alpha = self.last.alpha as f64;
        let now = self.now_s() + alpha * self.settings.step.step_s;
        for (id, posed) in &mut self.actors {
            let model = &self.models[posed.model].1;
            let (transform, visible) = (posed.transform, posed.visible);
            let palette = posed.palette(model, now);
            let state = ActorState {
                model: transform,
                palette,
                faces: &[],
                visible,
            };
            self.renderer.set_actor(*id, &state)?;
        }
        self.renderer.render(&self.camera, now as f32, target);
        if let Some(layer) = &mut self.ui {
            draw_ui(
                layer,
                &self.device,
                &self.queue,
                target,
                self.size,
                self.last.alpha,
            );
        }
        Ok(())
    }

    /// The UI events since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.events)
    }

    pub fn stats(&self) -> Value {
        let f = self.renderer.last_frame_stats();
        let q = self.renderer.effective_quality();
        json!({
            "steps": self.steps,
            "last_steps": self.last.steps,
            "path": format!("{:?}", q.path).to_lowercase(),
            "msaa": q.msaa,
            "size": [self.size.0, self.size.1],
            "passes": f.passes,
            "draws": f.draws,
            "lights": f.lights_in_frustum,
            "actors_drawn": f.actors_drawn,
            "bytes_uploaded": f.bytes_uploaded,
            "audio_queued_frames": self.sound.as_ref().map(|s| s.feed.queued()),
        })
    }
}

fn draw_ui(
    layer: &mut UiLayer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &wgpu::TextureView,
    (width, height): (u32, u32),
    alpha: f32,
) {
    let UiLayer { ui, pass, batches } = layer;
    if ui.take_atlas_changed() {
        let (side, texels) = ui.atlas();
        pass.set_texture(
            sariema_ui::ATLAS_TEXTURE,
            side,
            side,
            UiTextureFormat::R8,
            texels,
        );
    }
    let list = ui.draw(alpha);
    batches.clear();
    batches.extend(list.batches.iter().map(|b| UiBatch {
        texture: b.texture,
        scissor: b.scissor,
        start: b.start,
        count: b.count,
    }));
    let instances: &[UiInstance] = bytemuck::cast_slice(&list.quads);
    let mut encoder = device.create_command_encoder(&Default::default());
    let target = UiTarget {
        view: target,
        width,
        height,
    };
    pass.draw(&mut encoder, target, instances, batches, None);
    queue.submit([encoder.finish()]);
}

fn event_json(ui: &Ui, step: u64, e: &Event) -> Value {
    let name = |n| ui.name(n).map_or(Value::Null, |s| Value::String(s.into()));
    let (kind, node, extra) = match *e {
        Event::Press {
            node,
            pointer,
            x,
            y,
        } => ("press", node, json!({ "pointer": pointer, "x": x, "y": y })),
        Event::Drag {
            node,
            pointer,
            x,
            y,
        } => ("drag", node, json!({ "pointer": pointer, "x": x, "y": y })),
        Event::Release {
            node,
            pointer,
            x,
            y,
        } => (
            "release",
            node,
            json!({ "pointer": pointer, "x": x, "y": y }),
        ),
        Event::Click { node } => ("click", node, json!({})),
        Event::Stick { node, x, y } => ("stick", node, json!({ "x": x, "y": y })),
        Event::Tap { node } => ("tap", node, json!({})),
        Event::Held { node } => ("held", node, json!({})),
        Event::Focus { node } => ("focus", node, json!({})),
    };
    let mut v = json!({ "step": step, "type": kind, "node": name(node) });
    if let (Value::Object(o), Value::Object(more)) = (&mut v, extra) {
        o.extend(more);
    }
    v
}
