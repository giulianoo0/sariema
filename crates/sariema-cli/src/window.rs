// SPDX-License-Identifier: Apache-2.0

//! The window `run` draws into: its surface, the renderer drawing into it,
//! what to do when the surface gives no image, and the time between the
//! frames shown.

use std::any::Any;
use std::fmt::Display;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sariema_render::{Quality, RenderConfig, RenderError, Renderer, request_device};
use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::output::Failure;

/// Why a window's event loop ended early.
pub enum Stop {
    Failed(Failure),
    Panicked(Box<dyn Any + Send>),
}

impl Stop {
    /// The failure, or the panic carried on from where the loop returned.
    pub fn resume(self) -> Failure {
        match self {
            Stop::Failed(failure) => failure,
            Stop::Panicked(payload) => panic::resume_unwind(payload),
        }
    }
}

/// Runs `step`, catching a panic: one must not unwind through the
/// platform's event loop.
pub fn caught(step: impl FnOnce() -> Result<(), Failure>) -> Option<Stop> {
    match panic::catch_unwind(AssertUnwindSafe(step)) {
        Ok(Ok(())) => None,
        Ok(Err(failure)) => Some(Stop::Failed(failure)),
        Err(payload) => Some(Stop::Panicked(payload)),
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
fn recovery(why: NoFrame, skipped: u32, max_skipped: u32) -> Recovery {
    match why {
        NoFrame::Occluded => Recovery::Hide,
        NoFrame::Validation => {
            Recovery::Fail("the window's surface reported a validation error".into())
        }
        _ if skipped >= max_skipped => Recovery::Fail(format!(
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
pub struct FrameClock {
    last: Option<Instant>,
    pub run: Intervals,
    second: Intervals,
}

impl FrameClock {
    /// Counts a frame presented `at`. Once the intervals since the last report
    /// add up to a second, returns their average in ms.
    pub fn presented(&mut self, at: Instant) -> Option<f64> {
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
    pub fn pause(&mut self) {
        self.last = None;
    }
}

#[derive(Default)]
pub struct Intervals {
    total: Duration,
    count: u32,
}

impl Intervals {
    fn add(&mut self, interval: Duration) {
        self.total += interval;
        self.count += 1;
    }

    pub fn average_ms(&self) -> Option<f64> {
        (self.count > 0).then(|| self.total.as_secs_f64() * 1000.0 / self.count as f64)
    }
}

pub enum Outcome {
    Presented,
    /// No image this time; the next frame tries again.
    Skipped,
    /// The window is hidden.
    Hidden,
}

/// The window, its surface and the renderer drawing into it.
pub struct Screen {
    pub window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pub renderer: Renderer,
    pub zero_size: bool,
}

impl Screen {
    /// A window of `width` by `height` physical pixels named `name`, and a
    /// renderer with nothing loaded.
    pub fn open(
        event_loop: &ActiveEventLoop,
        name: &str,
        width: u32,
        height: u32,
        quality: Quality,
    ) -> Result<Screen, Failure> {
        let attributes = Window::default_attributes()
            .with_title(format!("{name} - sariema"))
            .with_inner_size(PhysicalSize::new(width, height));
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
            quality,
        };
        let renderer = Renderer::new(device.clone(), queue.clone(), render_config)?;
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

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
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

    /// Draws a frame with `paint` into the surface's next image and shows
    /// it; `skipped` frames in a row went undrawn before this one, and past
    /// `max_skipped` the run fails. A suboptimal surface is configured again
    /// after the frame is shown.
    pub fn draw(
        &mut self,
        skipped: u32,
        max_skipped: u32,
        paint: impl FnOnce(&mut Renderer, &wgpu::TextureView) -> Result<(), Failure>,
    ) -> Result<Outcome, Failure> {
        let (frame, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            other => return self.recover(other, skipped, max_skipped),
        };
        let view = frame.texture.create_view(&Default::default());
        paint(&mut self.renderer, &view)?;
        self.queue.present(frame);
        if suboptimal {
            self.surface.configure(&self.device, &self.config);
        }
        Ok(Outcome::Presented)
    }

    fn recover(
        &mut self,
        texture: wgpu::CurrentSurfaceTexture,
        skipped: u32,
        max_skipped: u32,
    ) -> Result<Outcome, Failure> {
        let why = match texture {
            wgpu::CurrentSurfaceTexture::Outdated => NoFrame::Outdated,
            wgpu::CurrentSurfaceTexture::Lost => NoFrame::Lost,
            wgpu::CurrentSurfaceTexture::Timeout => NoFrame::Timeout,
            wgpu::CurrentSurfaceTexture::Occluded => NoFrame::Occluded,
            _ => NoFrame::Validation,
        };
        match recovery(why, skipped, max_skipped) {
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
}

/// `size` within the largest texture the device makes, since the renderer
/// ignores a resize beyond it.
fn fit(size: PhysicalSize<u32>, device: &wgpu::Device) -> (u32, u32) {
    let max = device.limits().max_texture_dimension_2d;
    (size.width.min(max), size.height.min(max))
}

pub fn window_error(e: impl Display) -> Failure {
    Failure::new("E_GPU", format!("window: {e}"))
}

fn device_error(e: impl Display) -> Failure {
    RenderError::Device(e.to_string()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_fails_at_once_and_a_hidden_window_never_fails() {
        assert!(matches!(
            recovery(NoFrame::Validation, 0, 10),
            Recovery::Fail(_)
        ));
        assert_eq!(recovery(NoFrame::Occluded, u32::MAX, 10), Recovery::Hide);
    }

    #[test]
    fn skipped_frames_fail_past_the_cap() {
        assert_eq!(recovery(NoFrame::Outdated, 0, 3), Recovery::Reconfigure);
        assert_eq!(recovery(NoFrame::Lost, 1, 3), Recovery::Recreate);
        assert_eq!(recovery(NoFrame::Timeout, 2, 3), Recovery::Retry);
        for why in [NoFrame::Outdated, NoFrame::Lost, NoFrame::Timeout] {
            let Recovery::Fail(message) = recovery(why, 3, 3) else {
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
}
