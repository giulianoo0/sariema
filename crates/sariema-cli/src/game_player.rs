// SPDX-License-Identifier: Apache-2.0

//! The `run --game` window: a game pack played in the whole engine, a frame
//! drawn every display refresh at the steps' `alpha`, the keyboard and the
//! pointer as its input and its sound on the default output device.

use std::fmt::Write;
use std::fs;
use std::path::Path;
use std::time::Instant;

use sariema_audio::{AudioError, Output};
use sariema_engine::{Engine, EngineSettings};
use sariema_format::script::{GameAssets, decode_game};
use sariema_runtime::game::WasmLogic;
use sariema_runtime::input::{Key, RawInput};
use sariema_runtime::sim::Logic;
use sariema_ui::SafeArea;
use serde_json::{Value, json};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::WindowId;

use crate::RunArgs;
use crate::output::{Done, Failure};
use crate::player::PlayerSettings;
use crate::run::frame_stats_json;
use crate::save_dir::SaveDir;
use crate::window::{FrameClock, Outcome, Screen, Stop, caught, window_error};
use sariema_view::GameView;

/// The mouse, as the HUD's one pointer.
const MOUSE: u64 = 0;

/// Plays the game pack at `path` until the window is closed or `frames`
/// frames are presented.
pub fn play(path: &Path, args: &RunArgs, frames: Option<u32>) -> Result<Done, Failure> {
    let bytes = fs::read(path).map_err(|e| Failure::io(path, e))?;
    let decode = || {
        decode_game(&bytes).map_err(|e| Failure::new(e.code(), format!("{}: {e}", path.display())))
    };
    let pack = decode()?;
    let (name, assets) = (pack.name.clone(), pack.assets.clone());
    let seed = args.game.seed.unwrap_or(0);
    let load = |pack, settings| {
        Engine::load(pack, seed, settings)
            .map_err(|e| Failure::new(e.code, format!("{}: {e}", path.display())))
    };
    let (engine, sound) = if assets.sounds.is_some() {
        match sariema_audio::default_rate() {
            Ok(rate) => {
                let mut settings = EngineSettings::default();
                settings.mixer.sample_rate = rate;
                settings.audio.stream_thread = true;
                let mut engine = load(pack, settings)?;
                let mixer = engine.take_mixer().expect("a game with sounds has a mixer");
                match Output::start(mixer, rate) {
                    Ok(output) => (engine, Sound::Device(output)),
                    Err(e) => (
                        load(decode()?, EngineSettings::default())?,
                        Sound::Silent(e),
                    ),
                }
            }
            Err(e) => (load(pack, EngineSettings::default())?, Sound::Silent(e)),
        }
    } else {
        (load(pack, EngineSettings::default())?, Sound::None)
    };
    let saves = SaveDir::open(path, args.game.save_dir.as_deref())?;
    let mut engine = engine;
    engine.answers_saves(true);
    let event_loop = EventLoop::new().map_err(window_error)?;
    let mut player = GamePlayer {
        args,
        name,
        assets,
        engine,
        sound,
        saves,
        view: None,
        screen: None,
        raw: RawInput::default(),
        cursor: None,
        limit: frames,
        max_skipped: PlayerSettings::default().max_skipped_frames,
        frames: 0,
        skipped: 0,
        dropped: 0,
        occluded: false,
        last: None,
        clock: FrameClock::default(),
        stopped: None,
    };
    event_loop.run_app(&mut player).map_err(window_error)?;
    if let Some(stop) = player.stopped.take() {
        return Err(stop.resume());
    }
    player.done()
}

enum Sound {
    /// The pack has no sound bank.
    None,
    Device(Output),
    /// No device: the steps mix the sound and nothing hears it.
    Silent(AudioError),
}

impl Sound {
    fn json(&self) -> Value {
        match self {
            Sound::None => Value::Null,
            Sound::Device(output) => json!({
                "device": true,
                "sample_rate": output.sample_rate,
                "channels": output.channels,
            }),
            Sound::Silent(e) => json!({
                "device": false,
                "warning": { "code": e.code(), "message": e.to_string() },
            }),
        }
    }
}

struct GamePlayer<'a> {
    args: &'a RunArgs,
    name: String,
    assets: GameAssets,
    engine: Engine<WasmLogic>,
    sound: Sound,
    saves: SaveDir,
    /// Once the window has a renderer.
    view: Option<GameView>,
    screen: Option<Screen>,
    raw: RawInput,
    /// Where the mouse last was, in physical pixels.
    cursor: Option<(f32, f32)>,
    limit: Option<u32>,
    max_skipped: u32,
    /// Presented.
    frames: u32,
    /// Frames in a row the surface gave no image for.
    skipped: u32,
    /// Steps of time thrown away by frames over the clock's cap.
    dropped: u64,
    occluded: bool,
    /// When the last frame's steps were run.
    last: Option<Instant>,
    clock: FrameClock,
    stopped: Option<Stop>,
}

impl ApplicationHandler for GamePlayer<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.guard(event_loop, |player| {
            if player.screen.is_none() {
                let args = player.args;
                let mut screen = Screen::open(
                    event_loop,
                    &player.name,
                    args.width,
                    args.height,
                    args.quality.settings(),
                )?;
                let view = GameView::load(&mut screen.renderer, &player.assets, &player.engine)?;
                player.view = Some(view);
                let (width, height) = (screen.config.width, screen.config.height);
                player
                    .engine
                    .resize(width as f32, height as f32, SafeArea::default());
                screen.window.request_redraw();
                player.screen = Some(screen);
            }
            Ok(())
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        self.guard(event_loop, |player| player.handle(event_loop, event));
    }
}

impl GamePlayer<'_> {
    fn guard(
        &mut self,
        event_loop: &ActiveEventLoop,
        step: impl FnOnce(&mut Self) -> Result<(), Failure>,
    ) {
        if self.stopped.is_some() {
            return;
        }
        if let Some(stop) = caught(|| step(self)) {
            self.stopped = Some(stop);
            event_loop.exit();
        }
    }

    fn handle(&mut self, event_loop: &ActiveEventLoop, event: WindowEvent) -> Result<(), Failure> {
        let Some(screen) = &mut self.screen else {
            return Ok(());
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        ..
                    },
                ..
            } => {
                if let Some(key) = key(code) {
                    self.raw.set_key(key, state == ElementState::Pressed);
                }
            }
            WindowEvent::Focused(false) => let_go(&mut self.engine, &mut self.raw, self.cursor),
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = (position.x as f32, position.y as f32);
                self.cursor = Some((x, y));
                self.engine.pointer_move(MOUSE, x, y);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(at) = self.cursor {
                    match state {
                        ElementState::Pressed => self.engine.pointer_down(MOUSE, at.0, at.1),
                        ElementState::Released => self.engine.pointer_up(MOUSE, at.0, at.1),
                    }
                }
            }
            WindowEvent::Resized(size) => {
                screen.resize(size);
                if !screen.zero_size {
                    let (width, height) = (screen.config.width, screen.config.height);
                    self.engine
                        .resize(width as f32, height as f32, SafeArea::default());
                }
                self.suspend_or_resume();
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                self.suspend_or_resume();
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop)?,
            _ => {}
        }
        Ok(())
    }

    fn drawing(&self) -> bool {
        !self.occluded && self.screen.as_ref().is_some_and(|s| !s.zero_size)
    }

    /// A hidden window runs no steps: the game pauses until it is shown.
    fn suspend_or_resume(&mut self) {
        if !self.drawing() {
            self.clock.pause();
            self.last = None;
        } else if let Some(screen) = &self.screen {
            screen.window.request_redraw();
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Failure> {
        if !self.drawing() {
            return Ok(());
        }
        let now = Instant::now();
        let dt = self
            .last
            .replace(now)
            .map_or_else(Default::default, |last| now - last);
        let tick = self.engine.frame(dt, &self.raw);
        self.dropped += u64::from(tick.dropped);
        if self.saves.serve(&mut self.engine)? {
            event_loop.exit();
            return Ok(());
        }
        if let Some(stop) = self.engine.simulation().stopped() {
            return Err(Failure::new(
                stop.code,
                format!(
                    "{}: step {}: {}",
                    self.name,
                    self.engine.steps(),
                    stop.message
                ),
            ));
        }
        let (Some(screen), Some(view)) = (&mut self.screen, &mut self.view) else {
            return Ok(());
        };
        let aspect = screen.config.width as f32 / screen.config.height as f32;
        let alpha = self.engine.alpha();
        let engine = &mut self.engine;
        let paint = |renderer: &mut sariema_render::Renderer, target: &wgpu::TextureView| {
            Ok(view.draw(renderer, engine, alpha, aspect, target)?)
        };
        match screen.draw(self.skipped, self.max_skipped, paint)? {
            Outcome::Presented => {
                self.frames += 1;
                self.skipped = 0;
                if let Some(ms) = self.clock.presented(now) {
                    let stats = screen.renderer.last_frame_stats();
                    let mut title = format!("{} - {ms:.1} ms per frame", self.name);
                    if stats.lights_dropped > 0 {
                        let _ = write!(title, ", {} lights dropped", stats.lights_dropped);
                    }
                    screen.window.set_title(&title);
                }
            }
            Outcome::Skipped => self.skipped += 1,
            Outcome::Hidden => {
                self.occluded = true;
                self.suspend_or_resume();
                return Ok(());
            }
        }
        if self.limit.is_some_and(|n| self.frames >= n) {
            event_loop.exit();
        } else {
            screen.window.request_redraw();
        }
        Ok(())
    }

    fn done(&self) -> Result<Done, Failure> {
        let ms_per_frame = self.clock.run.average_ms();
        let stats = (self.screen.as_ref())
            .filter(|_| self.frames > 0)
            .map(|s| s.renderer.last_frame_stats());
        let mut text = format!("{} frames, {} steps", self.frames, self.engine.steps());
        if let Some(ms) = ms_per_frame {
            let _ = write!(text, ", {ms:.2} ms per frame");
        }
        if let Sound::Silent(e) = &self.sound {
            let _ = write!(text, "; no sound ({}: {e})", e.code());
        }
        Ok(Done {
            json: json!({
                "frames": self.frames,
                "steps": self.engine.steps(),
                "steps_dropped": self.dropped,
                "ms_per_frame": ms_per_frame,
                "engine_hash": format!("{:016x}", self.engine.state_hash()),
                "frame_stats": stats.as_ref().map(frame_stats_json),
                "sound": self.sound.json(),
                "saves": self.saves.file(),
            }),
            text,
        })
    }
}

/// The runtime's name for a key, for those the input settings can bind.
fn key(code: KeyCode) -> Option<Key> {
    use KeyCode as C;
    Some(match code {
        C::KeyA => Key::A,
        C::KeyB => Key::B,
        C::KeyC => Key::C,
        C::KeyD => Key::D,
        C::KeyE => Key::E,
        C::KeyF => Key::F,
        C::KeyG => Key::G,
        C::KeyH => Key::H,
        C::KeyI => Key::I,
        C::KeyJ => Key::J,
        C::KeyK => Key::K,
        C::KeyL => Key::L,
        C::KeyM => Key::M,
        C::KeyN => Key::N,
        C::KeyO => Key::O,
        C::KeyP => Key::P,
        C::KeyQ => Key::Q,
        C::KeyR => Key::R,
        C::KeyS => Key::S,
        C::KeyT => Key::T,
        C::KeyU => Key::U,
        C::KeyV => Key::V,
        C::KeyW => Key::W,
        C::KeyX => Key::X,
        C::KeyY => Key::Y,
        C::KeyZ => Key::Z,
        C::Digit0 => Key::Digit0,
        C::Digit1 => Key::Digit1,
        C::Digit2 => Key::Digit2,
        C::Digit3 => Key::Digit3,
        C::Digit4 => Key::Digit4,
        C::Digit5 => Key::Digit5,
        C::Digit6 => Key::Digit6,
        C::Digit7 => Key::Digit7,
        C::Digit8 => Key::Digit8,
        C::Digit9 => Key::Digit9,
        C::ArrowUp => Key::Up,
        C::ArrowDown => Key::Down,
        C::ArrowLeft => Key::Left,
        C::ArrowRight => Key::Right,
        C::Space => Key::Space,
        C::Enter | C::NumpadEnter => Key::Enter,
        C::Escape => Key::Escape,
        C::Tab => Key::Tab,
        C::ShiftLeft | C::ShiftRight => Key::Shift,
        C::ControlLeft | C::ControlRight => Key::Control,
        C::AltLeft | C::AltRight => Key::Alt,
        C::Backspace => Key::Backspace,
        _ => return None,
    })
}

/// The window lost focus, so no release will reach it: every key and the
/// mouse button are let go.
fn let_go<L: Logic>(engine: &mut Engine<L>, raw: &mut RawInput, cursor: Option<(f32, f32)>) {
    *raw = RawInput::default();
    if let Some((x, y)) = cursor {
        engine.pointer_up(MOUSE, x, y);
    }
}

#[cfg(test)]
mod tests {
    use sariema_engine::Assets;
    use sariema_format::script::Hud;
    use sariema_runtime::input::Actions;
    use sariema_runtime::settings::Settings;
    use sariema_runtime::sim::{Simulation, Stop};

    use super::*;

    struct Idle;

    impl Logic for Idle {
        fn tick(&mut self, _: &[u8], _: &[u32], out: &mut Vec<u8>) -> Result<(), Stop> {
            out.clear();
            Ok(())
        }
    }

    const STICK: &str = r#"{ "nodes": [
      { "name": "zone", "absolute": true, "left": 0, "top": 0, "bottom": 0, "width": "50%",
        "widget": { "joystick": { "base": "base", "pin": "pin" } },
        "children": [
          { "name": "base", "absolute": true, "left": 16, "bottom": 16, "width": 128, "height": 128,
            "children": [ { "name": "pin", "absolute": true, "left": 36, "top": 36, "width": 56, "height": 56 } ] }
        ] }
    ] }"#;

    #[test]
    fn losing_focus_lets_go_of_the_keys_and_the_mouse() {
        let assets = Assets {
            rooms: Vec::new(),
            actors: Vec::new(),
            sounds: None,
            hud: Some(Hud {
                description: STICK.into(),
                files: Vec::new(),
            }),
        };
        let sim = Simulation::new(Settings::default(), Vec::new(), Idle, 1);
        let mut engine = Engine::new(sim, assets, EngineSettings::default()).unwrap();
        engine.resize(1280.0, 720.0, SafeArea::default());
        let actions = Actions::for_settings(&engine.state().settings.input);
        let zone = engine.hud().ui.node("zone").unwrap();
        let mut raw = RawInput::default();
        raw.set_key(Key::W, true);
        engine.pointer_down(MOUSE, 150.0, 560.0);
        engine.pointer_move(MOUSE, 250.0, 560.0);
        engine.step(&actions);
        let held = engine.hud().ui.stick(zone).unwrap();
        assert!(held[0] > 0.5, "the stick is pushed: {held:?}");
        let_go(&mut engine, &mut raw, Some((250.0, 560.0)));
        engine.step(&actions);
        assert_eq!(raw, RawInput::default());
        assert_eq!(
            engine.hud().ui.stick(zone),
            Some([0.0, 0.0]),
            "the stick let go"
        );
    }
}
