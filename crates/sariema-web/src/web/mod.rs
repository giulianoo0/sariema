// SPDX-License-Identifier: Apache-2.0

//! The `Engine` class the page sees. Errors reach JavaScript as `Error`s
//! whose message starts with the stable code, as in `E_GPU: ...`.

mod capture;
mod gpu;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use glam::Mat4;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::game::GameHost;
use crate::input::PointerKind;
use crate::settings::WebSettings;
use crate::stage::{Stage, StageError};
use capture::Capture;
use gpu::Gpu;

const WAITING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

fn js(e: StageError) -> JsError {
    JsError::new(&e.to_string())
}

#[wasm_bindgen(start)]
fn start() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&format!("sariema panicked: {info}").into());
    }));
}

#[wasm_bindgen]
pub struct Engine {
    gpu: Gpu,
    stage: Stage,
    capture: Option<Capture>,
    /// The last capture's read back: [`WAITING`], [`MAPPED`] or [`FAILED`].
    captured: Arc<AtomicU8>,
    files: HashMap<String, Vec<u8>>,
    mixed: Vec<f32>,
    game: Option<GameHost>,
    /// Physical pixels per CSS pixel of the canvas.
    scale: f32,
}

/// A GPU opened on a canvas, for [`Engine::create`] or for an engine whose
/// own was lost to adopt with [`Engine::adopt`].
#[wasm_bindgen]
pub struct OpenedGpu {
    gpu: Gpu,
    stage: Stage,
}

/// Opens the GPU on `canvas`, `css_width × css_height` CSS pixels at
/// `pixel_ratio`, sized in physical pixels as [`Engine::physical_size`]
/// does. `options` is the JSON of the settings; see `docs/web.md`.
#[wasm_bindgen]
pub async fn open_gpu(
    canvas: HtmlCanvasElement,
    options: String,
    css_width: f32,
    css_height: f32,
    pixel_ratio: f32,
) -> Result<OpenedGpu, JsError> {
    let settings =
        WebSettings::from_json(&options).map_err(|e| js(StageError::new("E_USAGE", e)))?;
    let (width, height) = settings.physical_size((css_width, css_height), pixel_ratio);
    canvas.set_width(width);
    canvas.set_height(height);
    let gpu = Gpu::new(canvas, &settings).await.map_err(js)?;
    let size = (gpu.config.width, gpu.config.height);
    let format = gpu.config.format;
    let stage = Stage::new(
        gpu.device.clone(),
        gpu.queue.clone(),
        settings,
        size,
        format,
    )
    .map_err(js)?;
    Ok(OpenedGpu { gpu, stage })
}

#[wasm_bindgen]
impl Engine {
    /// The engine on a GPU [`open_gpu`] opened.
    pub fn create(opened: OpenedGpu) -> Engine {
        Engine {
            gpu: opened.gpu,
            stage: opened.stage,
            capture: None,
            captured: Arc::new(AtomicU8::new(WAITING)),
            files: HashMap::new(),
            mixed: Vec::new(),
            game: None,
            scale: 1.0,
        }
    }

    /// Goes on with a GPU [`open_gpu`] opened, after this engine's device
    /// or context was lost: a game at the step it had reached, its room and
    /// actors loaded into the new renderer; anything else the page loads
    /// again. On an error the engine is as it was, to adopt another.
    pub fn adopt(&mut self, opened: OpenedGpu) -> Result<(), JsError> {
        let OpenedGpu { gpu, mut stage } = opened;
        if let Some(game) = &mut self.game {
            game.reload_view(stage.renderer_mut()).map_err(js)?;
            let (w, h) = stage.size();
            game.resize(w, h);
        }
        self.gpu = gpu;
        self.stage = stage;
        self.capture = None;
        self.files.clear();
        Ok(())
    }

    pub fn has_game(&self) -> bool {
        self.game.is_some()
    }

    /// Plays game pack `pack` from its first step with `seed`, in place of
    /// any room or actors shown. With `sample_rate`, the page's audio rate,
    /// its sound is mixed for the worklet; without, with the steps, as a
    /// replay's must be.
    pub fn load_game(
        &mut self,
        pack: &[u8],
        seed: u64,
        sample_rate: Option<u32>,
    ) -> Result<(), JsError> {
        let feed = self.stage.settings().audio;
        let shadows = self.stage.settings().shadows;
        let size = self.stage.size();
        let renderer = self.stage.renderer_mut();
        let game =
            GameHost::load(pack, seed, sample_rate, feed, renderer, size, shadows).map_err(js)?;
        self.game = Some(game);
        Ok(())
    }

    /// Steps the game to step `to` with the actions of input log `log`
    /// (JSON), as `sariema run --headless --game --input` does.
    pub fn replay_game(&mut self, log: &str, to: f64) -> Result<(), JsError> {
        let game = (self.game.as_mut())
            .ok_or_else(|| js(StageError::new("E_USAGE", "no game is loaded")))?;
        game.replay(log, to as u64).map_err(js)
    }

    /// JSON: the game's steps and its engine, state, physics and audio hashes.
    pub fn game_hashes(&self) -> Option<String> {
        self.game.as_ref().map(|g| g.hashes().to_string())
    }

    /// `webgpu` or `webgl2`.
    pub fn backend(&self) -> String {
        self.gpu.backend().into()
    }

    /// JSON: the backend, adapter, limits and texture formats, and the
    /// renderer's path and texture variants.
    pub fn capabilities(&self) -> String {
        let mut v = self.gpu.describe();
        let q = self.stage.renderer().effective_quality();
        v["path"] = format!("{:?}", q.path).to_lowercase().into();
        v["msaa"] = q.msaa.into();
        v["texture_variants"] = self.stage.texture_variants().into();
        v.to_string()
    }

    /// Physical pixels for a canvas of `css_width × css_height` CSS pixels
    /// at `pixel_ratio`, capped by the settings; the page sets the canvas's
    /// width and height to them and then calls [`Engine::resize`].
    pub fn physical_size(&self, css_width: f32, css_height: f32, pixel_ratio: f32) -> Vec<u32> {
        let (w, h) = (self.stage.settings()).physical_size((css_width, css_height), pixel_ratio);
        vec![w, h]
    }

    /// The canvas now has `width × height` physical pixels for
    /// `css_width` CSS pixels across.
    pub fn resize(&mut self, width: u32, height: u32, css_width: f32) {
        self.gpu.resize(width, height);
        self.stage.resize(width, height);
        self.scale = width as f32 / css_width.max(1.0);
        self.stage.input().set_scale(self.scale);
        if let Some(game) = &mut self.game {
            game.resize(width, height);
        }
    }

    pub fn load_room(&mut self, pack: &[u8]) -> Result<(), JsError> {
        self.stage.load_room(pack).map_err(js)
    }

    pub fn load_actor(&mut self, pack: &[u8]) -> Result<u32, JsError> {
        self.stage.load_actor(pack).map_err(js)
    }

    pub fn spawn_actor(&mut self, model: u32) -> Result<u32, JsError> {
        self.stage.spawn_actor(model).map_err(js)
    }

    /// `transform` is 16 floats, column-major; `clip` empty for the bind pose.
    pub fn set_actor(
        &mut self,
        actor: u32,
        transform: &[f32],
        clip: &str,
        time_s: f32,
        speed: f32,
        visible: bool,
    ) -> Result<(), JsError> {
        let matrix: &[f32; 16] = transform
            .try_into()
            .map_err(|_| js(StageError::new("E_USAGE", "a transform is 16 floats")))?;
        let clip = (!clip.is_empty()).then_some(clip);
        let m = Mat4::from_cols_array(matrix);
        (self.stage)
            .set_actor(actor, m, clip, time_s, speed, visible)
            .map_err(js)
    }

    /// Keeps `bytes` as file `name` for the next [`Engine::set_ui`].
    pub fn add_file(&mut self, name: String, bytes: Vec<u8>) {
        self.files.insert(name, bytes);
    }

    pub fn set_ui(&mut self, description: &str) -> Result<(), JsError> {
        let files = std::mem::take(&mut self.files);
        self.stage.set_ui(description, &files).map_err(js)
    }

    pub fn load_sounds(&mut self, bank: &[u8], sample_rate: u32) -> Result<(), JsError> {
        self.stage.load_sounds(bank, sample_rate).map_err(js)
    }

    pub fn play(&mut self, sound: &str) -> Result<(), JsError> {
        self.stage.play(sound).map_err(js)
    }

    /// Frames per audio block, which the worklet reports played once each.
    pub fn audio_block_frames(&self) -> u32 {
        self.stage.settings().audio.whole_block_frames()
    }

    /// The audio blocks due, interleaved stereo; empty if none.
    pub fn mix(&mut self, frames_played: f64) -> js_sys::Float32Array {
        match &mut self.game {
            Some(game) => game.mix(frames_played as u64, &mut self.mixed),
            None => {
                self.stage.audio_played(frames_played as u64);
                self.stage.mix(&mut self.mixed);
            }
        }
        js_sys::Float32Array::from(self.mixed.as_slice())
    }

    /// `kind` is the DOM event type; positions in CSS pixels from the canvas's corner.
    pub fn pointer(&mut self, kind: &str, id: u32, x: f32, y: f32) {
        let Some(kind) = PointerKind::from_dom(kind) else {
            return;
        };
        let Some(game) = &mut self.game else {
            self.stage.input().pointer(kind, id as u64, x, y);
            return;
        };
        let (id, x, y) = (id as u64, x * self.scale, y * self.scale);
        match kind {
            PointerKind::Down => game.pointer_down(id, x, y),
            PointerKind::Move => game.pointer_move(id, x, y),
            PointerKind::Up | PointerKind::Cancel => game.pointer_up(id, x, y),
        }
    }

    /// `code` is the DOM `KeyboardEvent.code`; false for keys the engine ignores.
    pub fn key(&mut self, code: &str, down: bool) -> bool {
        match &mut self.game {
            Some(game) => game.key(code, down),
            None => self.stage.input().key(code, down),
        }
    }

    /// Runs the steps due at `now_ms` and draws to the canvas. Returns the
    /// steps run, or −1 when the device is lost and the page must make a
    /// new engine.
    pub fn frame(&mut self, now_ms: f64) -> Result<i32, JsError> {
        if self.gpu.is_lost() {
            return Ok(-1);
        }
        let frame = match self.gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                let (w, h) = self.stage.size();
                self.gpu.resize(w, h);
                return Ok(0);
            }
            _ => return Ok(0),
        };
        let view = frame.texture.create_view(&Default::default());
        let steps = match &mut self.game {
            Some(game) => {
                let tick = game.advance(now_ms).map_err(js)?;
                game.draw(self.stage.renderer_mut(), &view).map_err(js)?;
                tick.steps
            }
            None => self.stage.frame(now_ms, &view).map_err(js)?.steps,
        };
        self.gpu.queue.present(frame);
        Ok(if self.gpu.is_lost() { -1 } else { steps as i32 })
    }

    /// After the page was hidden: no catch-up of the time it was away.
    pub fn pause(&mut self) {
        self.stage.pause();
        if let Some(game) = &mut self.game {
            game.pause();
        }
    }

    /// Draws the current frame offscreen `draws` times, back to back, and
    /// starts reading the last back; the page polls [`Engine::take_capture`]
    /// until it returns the bytes. Many draws time the renderer as the
    /// three.js bench does, with one read at the end.
    pub fn capture(&mut self, draws: u32) -> Result<(), JsError> {
        let size = self.stage.size();
        let device = &self.gpu.device;
        if !self.capture.as_ref().is_some_and(|c| c.fits(size)) {
            self.capture = Some(Capture::new(device, self.gpu.config.format, size));
        }
        let capture = self.capture.as_ref().expect("made above");
        for _ in 0..draws.max(1) {
            match &mut self.game {
                Some(game) => game.draw(self.stage.renderer_mut(), &capture.view),
                None => self.stage.draw(&capture.view),
            }
            .map_err(js)?;
        }
        self.captured.store(WAITING, Ordering::Relaxed);
        let captured = self.captured.clone();
        capture.read(device, &self.gpu.queue, move |ok| {
            captured.store(if ok { MAPPED } else { FAILED }, Ordering::Relaxed)
        });
        Ok(())
    }

    /// The captured frame as RGBA rows once read back; undefined until then.
    pub fn take_capture(&mut self) -> Result<Option<Vec<u8>>, JsError> {
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        match self.captured.swap(WAITING, Ordering::Relaxed) {
            MAPPED => Ok(self.capture.as_ref().map(Capture::rgba)),
            FAILED => Err(js(StageError::new(
                "E_GPU",
                "reading the frame back failed",
            ))),
            _ => Ok(None),
        }
    }

    /// JSON array of the UI events since the last call; undefined if none.
    pub fn take_events(&mut self) -> Option<String> {
        let events = self.stage.take_events();
        (!events.is_empty()).then(|| serde_json::Value::from(events).to_string())
    }

    /// JSON of the last frame: steps, path, passes, draws, uploads, audio.
    pub fn stats(&self) -> String {
        let mut v = self.stage.stats();
        v["backend"] = self.gpu.backend().into();
        if let Some(game) = &self.game {
            v["game"] = game.stats();
        }
        v.to_string()
    }

    /// Destroys the device, as a lost one would be; for testing the page's recovery.
    pub fn lose_device(&self) {
        self.gpu.device.destroy();
    }
}
