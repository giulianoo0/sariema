// SPDX-License-Identifier: Apache-2.0

//! The `run` command's window: the room seen by its camera, which the mouse
//! and keys move.

use std::any::Any;
use std::fmt::{Display, Write};
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sariema_core::{Camera, room_camera};
use sariema_format::{Look, RoomCamera, RoomPack};
use sariema_render::{FrameStats, RenderConfig, RenderError, Renderer, request_device};
use serde_json::json;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::RunArgs;
use crate::output::{Done, Failure};
use crate::run::frame_stats_json;

/// Below this pitch the floor is nearly edge-on, and a vertical drag
/// corrected for its foreshortening would throw the focus far away.
const MIN_DRAG_PITCH_DEG: f32 = 6.0;

/// How the window's controls respond.
pub struct PlayerSettings {
    /// Metres; the wheel keeps the camera's distance between the two, widened
    /// to include the room's own distance.
    pub min_distance: f32,
    pub max_distance: f32,
    /// Seconds `T` takes to move the tilt-shift blur between 0 and 1.
    pub tilt_seconds: f32,
    /// At 1, the floor under the focus follows the cursor.
    pub drag_sensitivity: f32,
    /// What one wheel line divides the distance by.
    pub wheel_zoom: f32,
    /// Trackpad scroll pixels that count as one wheel line.
    pub pixels_per_line: f32,
    /// Frames in a row the surface may fail to give (timed out, outdated or
    /// lost) before the run fails with E_GPU; a hidden window does not count.
    pub max_skipped_frames: u32,
}

impl Default for PlayerSettings {
    fn default() -> PlayerSettings {
        PlayerSettings {
            min_distance: 3.0,
            max_distance: 60.0,
            tilt_seconds: 0.4,
            drag_sensitivity: 1.0,
            wheel_zoom: 1.1,
            pixels_per_line: 20.0,
            max_skipped_frames: 10,
        }
    }
}

/// What the controls change of the room's camera and look.
pub struct OrbitState {
    pub focus: [f32; 3],
    /// Metres from the focus.
    pub distance: f32,
    /// The look's tilt-shift strength, moving towards `tilt_target`.
    pub tilt: f32,
    pub tilt_target: f32,
}

impl OrbitState {
    pub fn new(camera: &RoomCamera, tilt: f32) -> OrbitState {
        OrbitState {
            focus: camera.focus,
            distance: camera.distance,
            tilt,
            tilt_target: tilt,
        }
    }

    /// `camera` moved to this focus and distance; `aspect` must be positive.
    pub fn camera(&self, camera: &RoomCamera, aspect: f32) -> Camera {
        let moved = RoomCamera {
            focus: self.focus,
            distance: self.distance,
            ..camera.clone()
        };
        room_camera(&moved, aspect)
    }

    /// `look` with this tilt. Its focus depth, measured from the camera, moves
    /// with the distance so the sharp range keeps its place around the focus.
    pub fn look(&self, look: &Look, camera: &RoomCamera) -> Look {
        Look {
            tilt: self.tilt,
            focus: look.focus + self.distance - camera.distance,
            ..look.clone()
        }
    }
}

pub enum PlayerInput {
    /// The cursor moved with the left button held, in a viewport `viewport_h` pixels high.
    Drag {
        dx_px: f32,
        dy_px: f32,
        viewport_h: f32,
    },
    /// Lines scrolled; scrolling up, positive, brings the camera closer.
    Wheel(f32),
    /// Back to the room's focus, distance and tilt.
    Reset,
    /// Sends the tilt towards 1 when its target is 0, and towards 0 otherwise.
    ToggleTilt,
    /// Seconds elapsed.
    Tick(f32),
}

/// `default` and `default_look` are the room's own camera and look.
pub fn apply_input(
    state: &mut OrbitState,
    default: &RoomCamera,
    default_look: &Look,
    settings: &PlayerSettings,
    input: PlayerInput,
) {
    match input {
        PlayerInput::Drag {
            dx_px,
            dy_px,
            viewport_h,
        } => {
            if viewport_h <= 0.0 {
                return;
            }
            let view_h = 2.0 * state.distance * (default.fov_y_deg.to_radians() / 2.0).tan();
            let metres_per_px = settings.drag_sensitivity * view_h / viewport_h;
            // Seen at `pitch`, the floor shrinks vertically by sin(pitch).
            let foreshortening = default.pitch_deg.max(MIN_DRAG_PITCH_DEG).to_radians().sin();
            // The floor follows the cursor, so the focus moves against it; screen up is −Z.
            state.focus[0] -= dx_px * metres_per_px;
            state.focus[2] -= dy_px * metres_per_px / foreshortening;
        }
        PlayerInput::Wheel(lines) => {
            let min = settings.min_distance.min(default.distance);
            let max = settings.max_distance.max(default.distance);
            state.distance = (state.distance / settings.wheel_zoom.powf(lines)).clamp(min, max);
        }
        PlayerInput::Reset => *state = OrbitState::new(default, default_look.tilt),
        PlayerInput::ToggleTilt => {
            state.tilt_target = if state.tilt_target > 0.0 { 0.0 } else { 1.0 };
        }
        PlayerInput::Tick(seconds) => {
            let step = if settings.tilt_seconds > 0.0 {
                seconds.max(0.0) / settings.tilt_seconds
            } else {
                f32::INFINITY
            };
            state.tilt += (state.tilt_target - state.tilt).clamp(-step, step);
        }
    }
}

/// Why the surface gave no frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NoFrame {
    Outdated,
    Lost,
    Timeout,
    Occluded,
    Validation,
}

#[derive(Debug, PartialEq, Eq)]
enum Recovery {
    Reconfigure,
    /// Make the surface again from the window.
    Recreate,
    Retry,
    /// Draw nothing until the window is shown again.
    Hide,
    Fail(String),
}

/// What to do when the surface gives no frame after `skipped` frames in a row went undrawn.
fn recovery(why: NoFrame, skipped: u32, settings: &PlayerSettings) -> Recovery {
    match why {
        NoFrame::Occluded => Recovery::Hide,
        NoFrame::Validation => {
            Recovery::Fail("the window's surface reported a validation error".into())
        }
        _ if skipped >= settings.max_skipped_frames => Recovery::Fail(format!(
            "the window's surface gave no frame {} times in a row, the last time {why:?}",
            skipped + 1
        )),
        NoFrame::Outdated => Recovery::Reconfigure,
        NoFrame::Lost => Recovery::Recreate,
        NoFrame::Timeout => Recovery::Retry,
    }
}

/// Times between presented frames, over the run and over the title's last second.
#[derive(Default)]
struct FrameClock {
    last: Option<Instant>,
    run: Intervals,
    second: Intervals,
}

impl FrameClock {
    /// Counts a frame presented `at`. Once the intervals since the last report
    /// add up to a second, returns their average in ms.
    fn presented(&mut self, at: Instant) -> Option<f64> {
        if let Some(last) = self.last.replace(at) {
            self.run.add(at - last);
            self.second.add(at - last);
        }
        if self.second.total < Duration::from_secs(1) {
            return None;
        }
        std::mem::take(&mut self.second).average_ms()
    }

    /// Leaves the time until the next presented frame uncounted.
    fn pause(&mut self) {
        self.last = None;
    }
}

#[derive(Default)]
struct Intervals {
    total: Duration,
    count: u32,
}

impl Intervals {
    fn add(&mut self, interval: Duration) {
        self.total += interval;
        self.count += 1;
    }

    fn average_ms(&self) -> Option<f64> {
        (self.count > 0).then(|| self.total.as_secs_f64() * 1000.0 / self.count as f64)
    }
}

/// Shows `room` until the window is closed, Esc is pressed or `frames`
/// frames are presented.
pub fn play(room: &RoomPack, args: &RunArgs, frames: Option<u32>) -> Result<Done, Failure> {
    let event_loop = EventLoop::new().map_err(window_error)?;
    let mut player = Player {
        room,
        args,
        limit: frames,
        settings: PlayerSettings::default(),
        orbit: OrbitState::new(&room.camera, room.look.tilt),
        look: room.look.clone(),
        screen: None,
        dragging: false,
        cursor: None,
        occluded: false,
        started: Instant::now(),
        last_tick: None,
        frames: 0,
        skipped: 0,
        clock: FrameClock::default(),
        stopped: None,
    };
    event_loop.run_app(&mut player).map_err(window_error)?;
    match player.stopped.take() {
        Some(Stop::Failed(failure)) => return Err(failure),
        Some(Stop::Panicked(payload)) => panic::resume_unwind(payload),
        None => {}
    }
    let ms_per_frame = player.clock.run.average_ms();
    let stats = (player.screen.as_ref())
        .filter(|_| player.frames > 0)
        .map(|s| s.renderer.last_frame_stats());
    let mut text = format!("{} frames", player.frames);
    if let Some(ms) = ms_per_frame {
        let _ = write!(text, ", {ms:.2} ms per frame");
    }
    if let Some(stats) = &stats {
        let _ = write!(text, ", {} post passes", stats.post_passes);
    }
    Ok(Done {
        json: json!({
            "frames": player.frames,
            "ms_per_frame": ms_per_frame,
            "frame_stats": stats.as_ref().map(frame_stats_json),
        }),
        text,
    })
}

struct Player<'a> {
    room: &'a RoomPack,
    args: &'a RunArgs,
    limit: Option<u32>,
    settings: PlayerSettings,
    orbit: OrbitState,
    /// The look the renderer has.
    look: Look,
    screen: Option<Screen>,
    dragging: bool,
    cursor: Option<PhysicalPosition<f64>>,
    occluded: bool,
    started: Instant,
    last_tick: Option<Instant>,
    /// Presented.
    frames: u32,
    /// Frames in a row the surface gave no image for.
    skipped: u32,
    clock: FrameClock,
    stopped: Option<Stop>,
}

enum Stop {
    Failed(Failure),
    Panicked(Box<dyn Any + Send>),
}

impl ApplicationHandler for Player<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.guard(event_loop, |player| {
            if player.screen.is_none() {
                let screen = Screen::open(event_loop, player.room, player.args)?;
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

impl Player<'_> {
    /// Runs `step` and ends the loop at its first failure or panic, keeping
    /// either for [`play`]: a panic must not unwind through the platform's event loop.
    fn guard(
        &mut self,
        event_loop: &ActiveEventLoop,
        step: impl FnOnce(&mut Self) -> Result<(), Failure>,
    ) {
        if self.stopped.is_some() {
            return;
        }
        let stop = match panic::catch_unwind(AssertUnwindSafe(|| step(self))) {
            Ok(Ok(())) => return,
            Ok(Err(failure)) => Stop::Failed(failure),
            Err(payload) => Stop::Panicked(payload),
        };
        self.stopped = Some(stop);
        event_loop.exit();
    }

    fn input(&mut self, input: PlayerInput) {
        let room = self.room;
        apply_input(
            &mut self.orbit,
            &room.camera,
            &room.look,
            &self.settings,
            input,
        );
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
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } => match code {
                KeyCode::Escape => event_loop.exit(),
                KeyCode::KeyR => self.input(PlayerInput::Reset),
                KeyCode::KeyT => self.input(PlayerInput::ToggleTilt),
                _ => {}
            },
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => self.dragging = state == ElementState::Pressed,
            WindowEvent::Focused(false) => self.dragging = false,
            WindowEvent::CursorLeft { .. } => {
                self.dragging = false;
                self.cursor = None;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let viewport_h = screen.config.height as f32;
                if let (true, Some(last)) = (self.dragging, self.cursor.replace(position)) {
                    self.input(PlayerInput::Drag {
                        dx_px: (position.x - last.x) as f32,
                        dy_px: (position.y - last.y) as f32,
                        viewport_h,
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / self.settings.pixels_per_line,
                };
                self.input(PlayerInput::Wheel(lines));
            }
            WindowEvent::Resized(size) => {
                screen.resize(size);
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

    /// A minimised or zero-sized window draws nothing until it is shown again.
    fn drawing(&self) -> bool {
        !self.occluded && self.screen.as_ref().is_some_and(|s| !s.zero_size)
    }

    fn suspend_or_resume(&mut self) {
        if !self.drawing() {
            self.clock.pause();
        } else if let Some(screen) = &self.screen {
            screen.window.request_redraw();
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Failure> {
        if !self.drawing() {
            return Ok(());
        }
        let now = Instant::now();
        let seconds = self
            .last_tick
            .replace(now)
            .map_or(0.0, |last| (now - last).as_secs_f32());
        self.input(PlayerInput::Tick(seconds));
        let look = self.orbit.look(&self.room.look, &self.room.camera);
        let Some(screen) = &mut self.screen else {
            return Ok(());
        };
        if look != self.look {
            screen.renderer.set_look(&look);
            self.look = look;
        }
        let aspect = screen.config.width as f32 / screen.config.height as f32;
        let camera = self.orbit.camera(&self.room.camera, aspect);
        let time_s = self.started.elapsed().as_secs_f32();
        match screen.draw(&camera, time_s, self.skipped, &self.settings)? {
            Outcome::Presented => {
                self.frames += 1;
                self.skipped = 0;
                if let Some(ms) = self.clock.presented(now) {
                    let stats = screen.renderer.last_frame_stats();
                    screen.window.set_title(&title(&self.room.name, ms, &stats));
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
}

/// The room, the average frame time and what the renderer had to leave out.
fn title(room: &str, ms_per_frame: f64, stats: &FrameStats) -> String {
    let mut title = format!("{room} - {ms_per_frame:.1} ms per frame");
    if stats.lights_dropped > 0 {
        let _ = write!(title, ", {} lights dropped", stats.lights_dropped);
    }
    if stats.shadows_skipped > 0 {
        let _ = write!(title, ", {} shadows skipped", stats.shadows_skipped);
    }
    title
}

enum Outcome {
    Presented,
    /// No image this time; the next frame tries again.
    Skipped,
    /// The window is hidden.
    Hidden,
}

/// The window, its surface and the renderer drawing into it.
struct Screen {
    window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    renderer: Renderer,
    zero_size: bool,
}

impl Screen {
    fn open(
        event_loop: &ActiveEventLoop,
        room: &RoomPack,
        args: &RunArgs,
    ) -> Result<Screen, Failure> {
        let attributes = Window::default_attributes()
            .with_title(format!("{} - sariema", room.name))
            .with_inner_size(PhysicalSize::new(args.width, args.height));
        let window = Arc::new(event_loop.create_window(attributes).map_err(window_error)?);
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(window.clone())
            .map_err(device_error)?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|_| RenderError::NoAdapter)?;
        let (device, queue) = request_device(&adapter, "sariema window")?;
        let size = window.inner_size();
        let (width, height) = fit(size, &device);
        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or_else(|| device_error("the adapter cannot present to this window"))?;
        let formats = surface.get_capabilities(&adapter).formats;
        config.format = formats
            .into_iter()
            .find(|f| f.is_srgb())
            .unwrap_or(config.format);
        config.present_mode = wgpu::PresentMode::Fifo;
        let render_config = RenderConfig {
            width: config.width,
            height: config.height,
            output_format: config.format,
            quality: args.quality.settings(),
        };
        let mut renderer = Renderer::new(device.clone(), queue.clone(), render_config)?;
        renderer.load_room(room)?;
        let mut screen = Screen {
            window,
            instance,
            surface,
            config,
            device,
            queue,
            renderer,
            zero_size: true,
        };
        screen.resize(size);
        Ok(screen)
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        let (width, height) = fit(size, &self.device);
        self.zero_size = width == 0 || height == 0;
        if self.zero_size {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.renderer.resize(width, height);
    }

    /// `skipped` frames in a row went undrawn before this one.
    fn draw(
        &mut self,
        camera: &Camera,
        time_s: f32,
        skipped: u32,
        settings: &PlayerSettings,
    ) -> Result<Outcome, Failure> {
        let why = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => {
                return Ok(self.present(frame, camera, time_s, false));
            }
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                return Ok(self.present(frame, camera, time_s, true));
            }
            wgpu::CurrentSurfaceTexture::Outdated => NoFrame::Outdated,
            wgpu::CurrentSurfaceTexture::Lost => NoFrame::Lost,
            wgpu::CurrentSurfaceTexture::Timeout => NoFrame::Timeout,
            wgpu::CurrentSurfaceTexture::Occluded => NoFrame::Occluded,
            wgpu::CurrentSurfaceTexture::Validation => NoFrame::Validation,
        };
        match recovery(why, skipped, settings) {
            Recovery::Reconfigure => self.surface.configure(&self.device, &self.config),
            Recovery::Recreate => {
                self.surface = self
                    .instance
                    .create_surface(self.window.clone())
                    .map_err(device_error)?;
                self.surface.configure(&self.device, &self.config);
            }
            Recovery::Retry => {}
            Recovery::Hide => return Ok(Outcome::Hidden),
            Recovery::Fail(why) => return Err(device_error(why)),
        }
        Ok(Outcome::Skipped)
    }

    /// A suboptimal surface is configured again after the frame is shown.
    fn present(
        &mut self,
        frame: wgpu::SurfaceTexture,
        camera: &Camera,
        time_s: f32,
        suboptimal: bool,
    ) -> Outcome {
        let view = frame.texture.create_view(&Default::default());
        self.renderer.render(camera, time_s, &view);
        self.queue.present(frame);
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Outcome::Presented
    }
}

/// `size` within the largest texture the device makes, since the renderer
/// ignores a resize beyond it.
fn fit(size: PhysicalSize<u32>, device: &wgpu::Device) -> (u32, u32) {
    let max = device.limits().max_texture_dimension_2d;
    (size.width.min(max), size.height.min(max))
}

fn window_error(e: impl Display) -> Failure {
    Failure::new("E_GPU", format!("window: {e}"))
}

fn device_error(e: impl Display) -> Failure {
    RenderError::Device(e.to_string()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> RoomCamera {
        RoomCamera {
            fov_y_deg: 40.0,
            pitch_deg: 45.0,
            distance: 6.0,
            focus: [1.5, 0.5, -1.5],
        }
    }

    fn look() -> Look {
        Look {
            exposure: 1.0,
            bloom_threshold: 1.0,
            bloom_strength: 0.3,
            tint: [1.0; 3],
            real: 0.0,
            night: 0.0,
            tilt: 0.0,
            focus: 8.0,
            ao_strength: Look::DEFAULT_AO_STRENGTH,
            ao_radius_m: Look::DEFAULT_AO_RADIUS_M,
        }
    }

    fn apply_to(
        camera: &RoomCamera,
        state: &mut OrbitState,
        inputs: impl IntoIterator<Item = PlayerInput>,
    ) {
        let settings = PlayerSettings::default();
        for input in inputs {
            apply_input(state, camera, &look(), &settings, input);
        }
    }

    fn apply(state: &mut OrbitState, inputs: impl IntoIterator<Item = PlayerInput>) {
        apply_to(&camera(), state, inputs);
    }

    fn drag(dx_px: f32, dy_px: f32) -> PlayerInput {
        PlayerInput::Drag {
            dx_px,
            dy_px,
            viewport_h: 540.0,
        }
    }

    #[test]
    fn wheel_clamps_distance() {
        let mut state = OrbitState::new(&camera(), 0.0);
        apply(&mut state, (0..200).map(|_| PlayerInput::Wheel(5.0)));
        assert_eq!(state.distance, 3.0);
        apply(&mut state, (0..200).map(|_| PlayerInput::Wheel(-5.0)));
        assert_eq!(state.distance, 60.0);
    }

    #[test]
    fn room_distance_outside_the_range_widens_it() {
        let far = RoomCamera {
            distance: 80.0,
            ..camera()
        };
        let mut state = OrbitState::new(&far, 0.0);
        apply_to(&far, &mut state, [PlayerInput::Wheel(1.0)]);
        assert!(
            (state.distance - 80.0 / 1.1).abs() < 1e-3,
            "{}",
            state.distance
        );
        apply_to(&far, &mut state, (0..50).map(|_| PlayerInput::Wheel(-1.0)));
        assert_eq!(state.distance, 80.0);

        let near = RoomCamera {
            distance: 2.0,
            ..camera()
        };
        let mut state = OrbitState::new(&near, 0.0);
        apply_to(&near, &mut state, [PlayerInput::Wheel(-1.0)]);
        assert!((state.distance - 2.2).abs() < 1e-4, "{}", state.distance);
        apply_to(&near, &mut state, (0..50).map(|_| PlayerInput::Wheel(1.0)));
        assert_eq!(state.distance, 2.0);
    }

    #[test]
    fn drag_moves_focus_on_ground_plane() {
        let mut state = OrbitState::new(&camera(), 0.0);
        apply(&mut state, [drag(100.0, 0.0)]);
        assert_eq!(state.focus[1], 0.5);
        assert!(state.focus[0] < 1.5, "{:?}", state.focus);
        assert_eq!(state.focus[2], -1.5);
        apply(&mut state, [drag(0.0, 100.0)]);
        assert_eq!(state.focus[1], 0.5);
        assert!(state.focus[2] < -1.5, "{:?}", state.focus);
    }

    /// How far a 100 px drag right and a 100 px drag down move the focus.
    fn drag_steps(pitch_deg: f32) -> (f32, f32) {
        let pitched = RoomCamera {
            pitch_deg,
            ..camera()
        };
        let mut state = OrbitState::new(&pitched, 0.0);
        apply_to(&pitched, &mut state, [drag(100.0, 0.0)]);
        let across = pitched.focus[0] - state.focus[0];
        apply_to(&pitched, &mut state, [drag(0.0, 100.0)]);
        (across, pitched.focus[2] - state.focus[2])
    }

    #[test]
    fn vertical_drag_follows_the_foreshortened_floor() {
        let (across, along) = drag_steps(90.0);
        assert!((along - across).abs() < 1e-5, "{across} {along}");
        let (across, along) = drag_steps(45.0);
        assert!(
            (along / across - 2f32.sqrt()).abs() < 1e-4,
            "{across} {along}"
        );
        let (across, along) = drag_steps(0.0);
        let limit = 1.0 / MIN_DRAG_PITCH_DEG.to_radians().sin();
        assert!((along / across - limit).abs() < 1e-3, "{across} {along}");
    }

    #[test]
    fn reset_restores_default() {
        let mut state = OrbitState::new(&camera(), 0.0);
        apply(
            &mut state,
            [
                PlayerInput::Wheel(3.0),
                drag(40.0, -25.0),
                PlayerInput::ToggleTilt,
                PlayerInput::Tick(0.2),
                PlayerInput::Reset,
            ],
        );
        assert_eq!(state.focus, camera().focus);
        assert_eq!(state.distance, camera().distance);
        assert_eq!((state.tilt, state.tilt_target), (look().tilt, look().tilt));
    }

    #[test]
    fn tilt_reaches_target_in_point_four_seconds() {
        let mut state = OrbitState::new(&camera(), 0.0);
        apply(
            &mut state,
            [PlayerInput::ToggleTilt, PlayerInput::Tick(0.2)],
        );
        assert!(state.tilt > 0.0 && state.tilt < 1.0, "{}", state.tilt);
        apply(&mut state, [PlayerInput::Tick(0.2)]);
        assert_eq!(state.tilt, 1.0);
        apply(
            &mut state,
            [PlayerInput::ToggleTilt, PlayerInput::Tick(0.4)],
        );
        assert_eq!(state.tilt, 0.0);
    }

    #[test]
    fn focus_depth_follows_the_distance() {
        let mut state = OrbitState::new(&camera(), 0.0);
        state.distance = 10.0;
        state.tilt = 0.5;
        let moved = state.look(&look(), &camera());
        assert_eq!(moved.focus, 12.0);
        assert_eq!(moved.tilt, 0.5);
        assert_eq!(moved.exposure, look().exposure);
        assert_eq!(
            OrbitState::new(&camera(), 0.0).look(&look(), &camera()),
            look()
        );
    }

    #[test]
    fn validation_fails_at_once_and_a_hidden_window_never_fails() {
        let settings = PlayerSettings::default();
        assert!(matches!(
            recovery(NoFrame::Validation, 0, &settings),
            Recovery::Fail(_)
        ));
        assert_eq!(
            recovery(NoFrame::Occluded, u32::MAX, &settings),
            Recovery::Hide
        );
    }

    #[test]
    fn skipped_frames_fail_past_the_cap() {
        let settings = PlayerSettings {
            max_skipped_frames: 3,
            ..PlayerSettings::default()
        };
        assert_eq!(
            recovery(NoFrame::Outdated, 0, &settings),
            Recovery::Reconfigure
        );
        assert_eq!(recovery(NoFrame::Lost, 1, &settings), Recovery::Recreate);
        assert_eq!(recovery(NoFrame::Timeout, 2, &settings), Recovery::Retry);
        for why in [NoFrame::Outdated, NoFrame::Lost, NoFrame::Timeout] {
            let Recovery::Fail(message) = recovery(why, 3, &settings) else {
                panic!("{why:?} past the cap should fail");
            };
            assert!(message.contains("4 times"), "{message}");
        }
    }

    #[test]
    fn frame_time_counts_presented_frames_only() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut clock = FrameClock::default();
        clock.presented(t0);
        assert_eq!(clock.run.average_ms(), None);
        clock.presented(ms(10));
        // A frame skipped at 20 ms is not counted, so the next interval is 20 ms.
        clock.presented(ms(30));
        assert_eq!(clock.run.average_ms(), Some(15.0));
        clock.pause();
        clock.presented(ms(5_000));
        clock.presented(ms(5_015));
        assert_eq!(clock.run.average_ms(), Some(15.0));
    }

    #[test]
    fn frame_clock_reports_once_a_second_has_passed() {
        let t0 = Instant::now();
        let mut clock = FrameClock::default();
        assert_eq!(clock.presented(t0), None);
        assert_eq!(clock.presented(t0 + Duration::from_millis(600)), None);
        assert_eq!(
            clock.presented(t0 + Duration::from_millis(1_200)),
            Some(600.0)
        );
        assert_eq!(clock.presented(t0 + Duration::from_millis(1_210)), None);
    }

    #[test]
    fn title_names_only_what_was_left_out() {
        let mut stats = FrameStats::default();
        assert_eq!(title("demo", 16.66, &stats), "demo - 16.7 ms per frame");
        stats.shadows_skipped = 2;
        assert_eq!(
            title("demo", 16.66, &stats),
            "demo - 16.7 ms per frame, 2 shadows skipped"
        );
        stats.lights_dropped = 3;
        assert_eq!(
            title("demo", 16.66, &stats),
            "demo - 16.7 ms per frame, 3 lights dropped, 2 shadows skipped"
        );
    }
}
