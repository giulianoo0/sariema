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
//! Every pipeline is created in [`Renderer::new`]; [`Renderer::render`]
//! creates no pipeline and no buffer.

use std::cell::Cell;
use std::fmt;
use std::sync::Arc;

use sariema_core::Camera;
use sariema_format::{Look, RoomPack};

mod cluster;
mod golden;
mod headless;
mod lights;
mod post;
mod quality;
mod resources;
mod scene;
mod shadow;
pub mod ui;

pub use golden::{GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, GoldenDiff, compare_images};
pub use headless::{Headless, render_coverage, render_to_image};
pub use quality::Quality;

use cluster::Visible;
use post::Post;
use resources::Room;
use scene::Scene;
use shadow::{Assigned, Shadows};

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
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::NoAdapter => write!(f, "no graphics adapter is available"),
            RenderError::Device(why) => write!(f, "graphics device: {why}"),
            RenderError::Room(why) => write!(f, "room cannot be rendered: {why}"),
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
}

/// The device and queue, counting what is created through them.
pub(crate) struct Gpu {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pipelines: Cell<u32>,
    buffers: Cell<u32>,
}

impl Gpu {
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
    room: Option<Room>,
    stats: FrameStats,
    warned: bool,
}

impl Renderer {
    /// Fails when the device lacks fragment-stage storage buffers or cannot
    /// hold a `width × height` target. `config.quality` is clamped to the
    /// device; see [`Renderer::effective_quality`].
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        config: RenderConfig,
    ) -> Result<Renderer, RenderError> {
        let limits = device.limits();
        let storage = limits.max_storage_buffers_per_shader_stage;
        if storage < scene::STORAGE_BUFFERS {
            return Err(RenderError::Device(format!(
                "lighting needs {} storage buffers in the fragment stage and this device has {storage}",
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
        let gpu = Gpu {
            device,
            queue,
            pipelines: Cell::new(0),
            buffers: Cell::new(0),
        };
        let layouts = scene::layouts(&gpu);
        let shadows = Shadows::new(&gpu, &quality, &layouts.draw);
        let scene = Scene::new(&gpu, (width, height), &quality, layouts, &shadows);
        let post = Post::new(&gpu, config.output_format, &quality, &scene);
        Ok(Renderer {
            gpu,
            quality,
            width,
            height,
            scene,
            shadows,
            post,
            room: None,
            stats: FrameStats::default(),
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
        let room = self.room.as_ref();
        let visible = cluster::visible_lights(
            room.map_or(&[][..], |r| &r.lights[..]),
            camera.view,
            camera.proj,
            camera.position,
            self.quality.max_visible_lights,
        );
        let shadows = self
            .shadows
            .assign(&self.gpu, room, camera, &visible.lights);
        let stats = self
            .scene
            .prepare(&self.gpu, room, camera, time_s, &visible, &shadows);
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
            self.shadows.draw(&mut encoder, room);
        }
        let keep_depth = self.post.reads_depth();
        self.scene
            .draw(&mut encoder, self.room.as_ref(), camera, keep_depth);
        self.stats.post_passes = self.post.draw(&self.gpu, &mut encoder, camera, target);
        self.gpu.queue.submit([encoder.finish()]);
    }

    /// Depth of the non-additive visible draws into a single-sample
    /// `Depth32Float` view cleared to 1.
    pub(crate) fn render_depth(&mut self, camera: &Camera, depth: &wgpu::TextureView) {
        let room = self.room.as_ref();
        let (none, no_shadows) = (Visible::default(), Assigned::default());
        self.scene
            .prepare(&self.gpu, room, camera, 0.0, &none, &no_shadows);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.scene
            .draw_depth(&mut encoder, self.room.as_ref(), camera, depth);
        self.gpu.queue.submit([encoder.finish()]);
    }

    /// Render pipelines created since [`Renderer::new`].
    pub fn pipelines_created(&self) -> u32 {
        self.gpu.pipelines.get()
    }

    /// Buffers created since [`Renderer::new`].
    pub fn buffers_created(&self) -> u32 {
        self.gpu.buffers.get()
    }

    /// Shadow maps drawn since [`Renderer::new`], one per atlas slot drawn.
    pub fn shadow_passes_rendered(&self) -> u32 {
        self.shadows.drawn()
    }

    pub fn last_frame_stats(&self) -> FrameStats {
        self.stats
    }

    pub(crate) fn gpu(&self) -> &Gpu {
        &self.gpu
    }
}

/// A device of `adapter` with every limit it has, and with ASTC sampling and
/// adapter-specific format features when it offers them.
pub fn request_device(
    adapter: &wgpu::Adapter,
    label: &str,
) -> Result<(Arc<wgpu::Device>, Arc<wgpu::Queue>), RenderError> {
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        // Adapter-specific format features let the quality clamp offer sample
        // counts beyond WebGPU's 1 and 4; ASTC lets packs keep compressed textures.
        required_features: adapter.features()
            & (wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                | wgpu::Features::TEXTURE_COMPRESSION_ASTC),
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
