// SPDX-License-Identifier: Apache-2.0

//! The Sariema renderer: draws a [`RoomPack`] with `wgpu` into any target,
//! a window's surface or an offscreen texture.
//!
//! Local lights use clustered forward shading: each frame the CPU culls the
//! room's lights to the view, bins them into screen tiles × depth slices and
//! uploads the lists; the fragment shader loops over its cluster's lights.
//! Shadows come from one depth atlas whose slots are redrawn only when what
//! they show changes. Post-processing blurs the HDR image into a chain of
//! smaller levels for bloom and a miniature blur, then one pass tone maps,
//! grades and dithers it into the target.
//! Actors are skinned packs drawn after the room's opaque draws with the same
//! lights and shadows; see [`Renderer::load_actor`].
//! Every room pipeline is created in [`Renderer::new`] and every actor
//! pipeline when the first pack needing it is loaded; [`Renderer::render`]
//! creates no pipeline and no buffer.

use std::cell::Cell;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use sariema_core::Camera;
use sariema_format::actor::ActorPack;
use sariema_format::{AlphaMode, Look, RoomPack};

mod actors;
mod cluster;
mod cull;
#[cfg(test)]
mod glsl_es;
mod golden;
mod headless;
mod lights;
mod post;
mod quality;
mod resources;
mod scene;
mod shadow;
mod table;
mod timing;
pub mod ui;

pub use actors::{ActorId, ActorModel, ActorState, DiscShadow};
pub use golden::{GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, GoldenDiff, compare_images};
pub use headless::{Headless, render_coverage, render_to_image};
pub use quality::{Quality, RenderPath};
pub use timing::{FrameTimings, PassTime};

use actors::Actors;
use cluster::Visible;
use post::Post;
use resources::Room;
use scene::Scene;
use shadow::{Assigned, Shadows};
use timing::{GpuTimer, Stopwatch};
use ui::{UiBatch, UiInstance, UiPass, UiTarget, UiTextureFormat};

#[derive(Clone, Debug)]
pub struct RenderConfig {
    pub width: u32,
    pub height: u32,
    /// Format of the targets given to [`Renderer::render`].
    pub output_format: wgpu::TextureFormat,
    pub quality: Quality,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    NoAdapter,
    Device(String),
    /// The room passed `decode` but cannot be drawn.
    Room(String),
    /// An actor pack that cannot be drawn, or an actor given what it cannot show.
    Actor(String),
}

impl RenderError {
    /// The stable error code: a room or actor that cannot be drawn is
    /// treated like a pack that fails `decode`.
    pub fn code(&self) -> &'static str {
        match self {
            RenderError::Room(_) | RenderError::Actor(_) => "E_SCHEMA",
            RenderError::NoAdapter | RenderError::Device(_) => "E_GPU",
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::NoAdapter => write!(f, "no graphics adapter is available"),
            RenderError::Device(why) => write!(f, "graphics device: {why}"),
            RenderError::Room(why) => write!(f, "room cannot be rendered: {why}"),
            RenderError::Actor(why) => write!(f, "actor cannot be rendered: {why}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// What the last [`Renderer::render`] did with the room's local lights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub lights_in_frustum: u32,
    /// In the frustum but past `Quality::max_visible_lights`.
    pub lights_dropped: u32,
    pub clusters: u32,
    /// The most lights whose reach overlapped one cluster, counting those
    /// past `Quality::max_lights_per_cluster`.
    pub max_lights_overlapping_cluster: u32,
    /// Clusters that had more than `Quality::max_lights_per_cluster` lights.
    pub cluster_overflows: u32,
    /// Lights with `casts_shadow` that were shaded without a shadow: past
    /// `Quality::max_shadowed_lights`, or left without a square of at least
    /// `Quality::shadow_min_slot_px` in the atlas.
    pub shadows_skipped: u32,
    /// Full-screen passes after the scene, the composite included; effects
    /// the look turns off draw none.
    pub post_passes: u32,
    /// Visible room draws left out because their bounds are outside the view.
    pub draws_culled: u32,
    pub actors_drawn: u32,
    /// Visible actors left out because their bounds are outside the view.
    pub actors_culled: u32,
    /// Render passes of the frame: shadows, scene and post-processing.
    pub passes: u32,
    /// Draw calls of every pass.
    pub draws: u32,
    /// Bytes written to GPU buffers during the frame.
    pub bytes_uploaded: u64,
}

/// The device and queue, counting what is created and done through them.
pub(crate) struct Gpu {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pipelines: Cell<u32>,
    buffers: Cell<u32>,
    passes: Cell<u32>,
    draws: Cell<u32>,
    uploaded: Cell<u64>,
    timer: Option<GpuTimer>,
    /// Every shader's label and WGSL, for the GLSL ES check.
    #[cfg(test)]
    shaders: std::cell::RefCell<Vec<(String, String)>>,
}

impl Gpu {
    pub(crate) fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Gpu {
        Gpu {
            device,
            queue,
            pipelines: Cell::new(0),
            buffers: Cell::new(0),
            passes: Cell::new(0),
            draws: Cell::new(0),
            uploaded: Cell::new(0),
            timer: None,
            #[cfg(test)]
            shaders: Default::default(),
        }
    }

    pub fn shader(&self, label: &str, source: String) -> wgpu::ShaderModule {
        #[cfg(test)]
        self.shaders
            .borrow_mut()
            .push((label.into(), source.clone()));
        self.device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
    }

    /// Counts a render pass about to begin and gives its timestamp writes
    /// while passes are being timed.
    pub fn pass(&self, label: &'static str) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.passes.set(self.passes.get() + 1);
        self.timer.as_ref()?.pass(label)
    }

    pub fn count_draws(&self, draws: u32) {
        self.draws.set(self.draws.get() + draws);
    }

    pub fn write(&self, buffer: &wgpu::Buffer, bytes: &[u8]) {
        self.uploaded.set(self.uploaded.get() + bytes.len() as u64);
        self.queue.write_buffer(buffer, 0, bytes);
    }

    /// Writes `width × rows` texels from row `y` of `texture`, each row of
    /// `bytes` starting `bytes_per_row` after the one before.
    pub fn write_texture(
        &self,
        texture: &wgpu::Texture,
        y: u32,
        (width, rows): (u32, u32),
        bytes_per_row: u32,
        bytes: &[u8],
    ) {
        self.uploaded.set(self.uploaded.get() + bytes.len() as u64);
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(rows),
            },
            wgpu::Extent3d {
                width,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn pipeline(&self, desc: &wgpu::RenderPipelineDescriptor) -> wgpu::RenderPipeline {
        self.pipelines.set(self.pipelines.get() + 1);
        self.device.create_render_pipeline(desc)
    }

    pub fn buffer(&self, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.buffers.set(self.buffers.get() + 1);
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    }

    pub fn buffer_with(
        &self,
        label: &str,
        contents: &[u8],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        self.buffers.set(self.buffers.get() + 1);
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
    }
}

pub struct Renderer {
    gpu: Gpu,
    quality: Quality,
    width: u32,
    height: u32,
    scene: Scene,
    shadows: Shadows,
    post: Post,
    actors: Actors,
    room: Option<Room>,
    ui: UiPass,
    visible: Visible,
    stats: FrameStats,
    timings: FrameTimings,
    cpu_clock: Option<fn() -> Duration>,
    warned: bool,
}

impl Renderer {
    /// Fails when `config.quality` forces [`RenderPath::Full`] on a device
    /// without fragment-stage storage buffers, or the device cannot hold a
    /// `width × height` target. `config.quality` is clamped to the device and
    /// its path chosen; see [`Renderer::effective_quality`].
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        config: RenderConfig,
    ) -> Result<Renderer, RenderError> {
        let limits = device.limits();
        let storage = limits.max_storage_buffers_per_shader_stage;
        if config.quality.path == RenderPath::Full && storage < scene::STORAGE_BUFFERS {
            return Err(RenderError::Device(format!(
                "the full path needs {} storage buffers in the fragment stage and this device has {storage}; \
                 RenderPath::Downlevel runs without",
                scene::STORAGE_BUFFERS
            )));
        }
        let RenderConfig { width, height, .. } = config;
        if !fits(width, height, &limits) {
            return Err(RenderError::Device(format!(
                "cannot render at {width}×{height}; the device allows 1 to {} per side",
                limits.max_texture_dimension_2d
            )));
        }
        let quality = config.quality.clamped(&device);
        let gpu = Gpu::new(device, queue);
        let layouts = scene::layouts(&gpu, quality.path);
        let shadows = Shadows::new(&gpu, &quality, &layouts.draw);
        let scene = Scene::new(&gpu, (width, height), &quality, layouts, &shadows);
        let post = Post::new(&gpu, config.output_format, &quality, &scene);
        let actors = Actors::new(&gpu, &quality);
        let ui = UiPass::new(
            gpu.device.clone(),
            gpu.queue.clone(),
            config.output_format,
            quality.ui_quads,
        );
        gpu.pipelines.set(gpu.pipelines.get() + 1);
        Ok(Renderer {
            gpu,
            quality,
            width,
            height,
            scene,
            shadows,
            post,
            actors,
            room: None,
            ui,
            visible: Visible::default(),
            stats: FrameStats::default(),
            timings: FrameTimings::default(),
            cpu_clock: None,
            warned: false,
        })
    }

    pub fn effective_quality(&self) -> &Quality {
        &self.quality
    }

    /// Uploads the room's meshes, textures and materials, replacing any room
    /// loaded before, and applies its look.
    pub fn load_room(&mut self, room: &RoomPack) -> Result<(), RenderError> {
        let uploaded = resources::upload(&self.gpu, self.scene.layouts(), room)?;
        self.shadows.reset(&uploaded);
        self.room = Some(uploaded);
        self.set_look(&room.look);
        self.warned = false;
        Ok(())
    }

    /// Uploads an actor pack, creating the pipelines its materials need; the
    /// first pack also creates the shadow and disc pipelines. Fails on a
    /// pack the device cannot hold, or on the downlevel path, one whose
    /// record outgrows `Quality::actor_uniform_rows`.
    pub fn load_actor(&mut self, pack: &ActorPack) -> Result<ActorModel, RenderError> {
        let layouts = self.scene.layouts();
        let view = &self.shadows.view_layout;
        (self.actors).load(&self.gpu, layouts, view, &self.quality, pack)
    }

    /// A new actor showing `model`, hidden until [`Renderer::set_actor`].
    /// It creates a buffer only when the actors outgrow the one they share.
    pub fn spawn_actor(&mut self, model: ActorModel) -> Result<ActorId, RenderError> {
        self.actors.spawn(&self.gpu, model)
    }

    /// Places and poses `actor` for the frames from the next one on.
    pub fn set_actor(&mut self, actor: ActorId, state: &ActorState) -> Result<(), RenderError> {
        self.actors.set(actor, state)
    }

    /// Shows or hides the actor's parts called `part`; false if there is none.
    pub fn set_actor_part_visible(&mut self, actor: ActorId, part: &str, visible: bool) -> bool {
        self.actors.set_part_visible(actor, part, visible)
    }

    /// Replaces the actor's disc shadow; `None` draws none.
    pub fn set_actor_disc(
        &mut self,
        actor: ActorId,
        disc: Option<DiscShadow>,
    ) -> Result<(), RenderError> {
        self.actors.set_disc(actor, disc)
    }

    /// Sizes of 0, or beyond what the device holds, are ignored.
    pub fn resize(&mut self, width: u32, height: u32) {
        if !fits(width, height, &self.gpu.device.limits()) {
            return;
        }
        self.width = width;
        self.height = height;
        self.scene.resize(&self.gpu, width, height, &self.quality);
        self.post.resize(&self.gpu, &self.scene);
    }

    pub fn set_look(&mut self, look: &Look) {
        self.post.set_look(&self.gpu, look);
    }

    /// Shows or hides every draw called `name`; false if there is none.
    pub fn set_draw_visible(&mut self, name: &str, visible: bool) -> bool {
        let Some(room) = &mut self.room else {
            return false;
        };
        let mut found = false;
        for draw in room
            .draws
            .iter_mut()
            .filter(|d| d.name.as_deref() == Some(name))
        {
            if draw.casts_shadow && draw.visible != visible {
                self.shadows.caster_toggled(draw.min, draw.max);
            }
            draw.visible = visible;
            found = true;
        }
        found
    }

    /// Draws a frame into `target`, which must be `width × height` in the
    /// configured output format.
    pub fn render(&mut self, camera: &Camera, time_s: f32, target: &wgpu::TextureView) {
        let mut watch = Stopwatch::start(self.cpu_clock);
        for count in [&self.gpu.passes, &self.gpu.draws] {
            count.set(0);
        }
        self.gpu.uploaded.set(0);
        let room = self.room.as_ref();
        self.visible.find(
            room.map_or(&[][..], |r| &r.lights[..]),
            camera.view,
            camera.proj,
            camera.position,
            self.quality.max_visible_lights,
        );
        self.actors.upload(&self.gpu);
        self.shadows.assign(
            &self.gpu,
            room,
            camera,
            &self.visible.lights,
            self.actors.moved(),
        );
        let draws_culled = self.scene.cull(room, camera);
        let actor_stats = (self.actors).prepare(camera, |b| self.shadows.maps(b));
        self.actors.forget_moves();
        let cull = watch.lap();
        let mut stats = self.scene.prepare(
            &self.gpu,
            room,
            camera,
            time_s,
            &self.visible,
            self.shadows.assigned(),
        );
        let bin = watch.lap();
        stats.draws_culled = draws_culled;
        stats.actors_drawn = actor_stats.drawn;
        stats.actors_culled = actor_stats.culled;
        if !self.warned && (stats.lights_dropped > 0 || stats.cluster_overflows > 0) {
            self.warned = true;
            log::warn!(
                "{} of {} lights in view were dropped and {} of {} clusters overflowed; \
                 raise Quality::max_visible_lights or Quality::max_lights_per_cluster",
                stats.lights_dropped,
                stats.lights_in_frustum,
                stats.cluster_overflows,
                stats.clusters,
            );
        }
        self.stats = stats;
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        if let Some(room) = &self.room {
            self.shadows
                .draw(&self.gpu, &mut encoder, room, &self.actors);
        }
        let keep_depth = self.post.reads_depth();
        self.scene.draw(
            &self.gpu,
            &mut encoder,
            self.room.as_ref(),
            &self.actors,
            keep_depth,
        );
        let record = watch.lap();
        self.stats.post_passes = self.post.draw(&self.gpu, &mut encoder, camera, target);
        if let Some(timer) = &self.gpu.timer {
            timer.end_frame(&mut encoder);
        }
        let post = watch.lap();
        let commands = encoder.finish();
        let finish = watch.lap();
        self.gpu.queue.submit([commands]);
        let submit = watch.lap();
        self.stats.passes = self.gpu.passes.get();
        self.stats.draws = self.gpu.draws.get();
        self.stats.bytes_uploaded = self.gpu.uploaded.get();
        self.timings = FrameTimings {
            cull,
            bin,
            record,
            post,
            finish,
            submit,
        };
    }

    /// Creates HUD texture `id` or replaces its texels, uploaded with the
    /// next [`Renderer::render_ui`]: the glyph atlas is
    /// `sariema_ui::ATLAS_TEXTURE`, image `i` is `1 + i`.
    pub fn set_ui_texture(
        &mut self,
        id: u32,
        width: u32,
        height: u32,
        format: UiTextureFormat,
        texels: &[u8],
    ) {
        self.ui.set_texture(id, width, height, format, texels);
    }

    /// Draws the HUD's quads over `target` after [`Renderer::render`] drew
    /// the frame into it; its pass, draws and uploads count with that frame.
    pub fn render_ui(
        &mut self,
        target: &wgpu::TextureView,
        instances: &[UiInstance],
        batches: &[UiBatch],
    ) {
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let target = UiTarget {
            view: target,
            width: self.width,
            height: self.height,
        };
        self.ui.draw(&mut encoder, target, instances, batches, None);
        self.gpu.queue.submit([encoder.finish()]);
        self.stats.passes += 1;
        self.stats.draws += self.ui.draw_calls();
        self.stats.bytes_uploaded += self.ui.uploaded();
    }

    /// Depth of the non-additive visible draws into a single-sample
    /// `Depth32Float` view cleared to 1.
    pub(crate) fn render_depth(&mut self, camera: &Camera, depth: &wgpu::TextureView) {
        let room = self.room.as_ref();
        let (none, no_shadows) = (Visible::default(), Assigned::default());
        self.scene.cull(room, camera);
        self.scene
            .prepare(&self.gpu, room, camera, 0.0, &none, &no_shadows);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let covers = |alpha| alpha != AlphaMode::Additive;
        let room = self.room.as_ref();
        (self.scene).draw_depth(&self.gpu, &mut encoder, room, None, depth, covers);
        self.gpu.queue.submit([encoder.finish()]);
    }

    /// Render pipelines created since [`Renderer::new`].
    pub fn pipelines_created(&self) -> u32 {
        self.gpu.pipelines.get()
    }

    /// Buffers created since [`Renderer::new`].
    pub fn buffers_created(&self) -> u32 {
        self.gpu.buffers.get() + self.ui.buffers_created()
    }

    /// Shadow maps drawn since [`Renderer::new`], one per atlas slot drawn.
    pub fn shadow_passes_rendered(&self) -> u32 {
        self.shadows.drawn()
    }

    pub fn last_frame_stats(&self) -> FrameStats {
        self.stats
    }

    /// The CPU time of each stage of the last frame, zero unless
    /// [`Renderer::time_cpu_stages`] gave a clock.
    pub fn last_frame_timings(&self) -> FrameTimings {
        self.timings
    }

    /// Times each stage of the next frames with `clock`, the time since any
    /// fixed origin; `None`, the default, reads no clock at all. A clock
    /// built on `std::time::Instant` panics on wasm32-unknown-unknown, so
    /// the web passes one built on `performance.now()`.
    pub fn time_cpu_stages(&mut self, clock: Option<fn() -> Duration>) {
        self.cpu_clock = clock;
    }

    /// Times every render pass of the next `frames` frames on the GPU; false,
    /// and nothing timed, where the device lacks `Features::TIMESTAMP_QUERY`.
    /// Replaces the passes timed before.
    pub fn time_gpu_passes(&mut self, frames: u32) -> bool {
        let device = &self.gpu.device;
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return false;
        }
        // The shadow atlas, the scene and its depth pass, then the post-processing chain.
        let passes = 2 + self.scene.depth_passes() + self.post.most_passes();
        self.gpu.timer = Some(GpuTimer::new(device, passes, frames.max(1)));
        true
    }

    /// Per frame timed by [`Renderer::time_gpu_passes`], its passes in order;
    /// waits for the GPU to finish them.
    pub fn gpu_pass_times(&self) -> Result<Vec<Vec<PassTime>>, RenderError> {
        match &self.gpu.timer {
            Some(timer) => timer.read(&self.gpu.device, &self.gpu.queue),
            None => Ok(Vec::new()),
        }
    }

    pub(crate) fn gpu(&self) -> &Gpu {
        &self.gpu
    }
}

/// The float formats the renderer draws into besides its target: the scene,
/// view depth and the occlusion. WebGL2 renders into them only with
/// `EXT_color_buffer_float`, which a device must have.
pub const FLOAT_TARGETS: [wgpu::TextureFormat; 3] =
    [scene::HDR_FORMAT, post::VIEW_DEPTH_FORMAT, post::ao::FORMAT];

/// A device of `adapter` with every limit it has, and with ASTC sampling,
/// adapter-specific format features and timestamp queries when it offers them.
pub fn request_device(
    adapter: &wgpu::Adapter,
    label: &str,
) -> Result<(Arc<wgpu::Device>, Arc<wgpu::Queue>), RenderError> {
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        // Adapter-specific format features let the quality clamp offer sample
        // counts beyond WebGPU's 1 and 4; ASTC lets packs keep compressed
        // textures; timestamps let `Renderer::time_gpu_passes` time passes.
        required_features: adapter.features()
            & (wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                | wgpu::Features::TEXTURE_COMPRESSION_ASTC
                | wgpu::Features::TIMESTAMP_QUERY),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|e| RenderError::Device(e.to_string()))?;
    Ok((Arc::new(device), Arc::new(queue)))
}

fn fits(width: u32, height: u32, limits: &wgpu::Limits) -> bool {
    let side = 1..=limits.max_texture_dimension_2d;
    side.contains(&width) && side.contains(&height)
}
