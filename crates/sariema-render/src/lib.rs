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
//!
//! Several rooms can be resident at once, each in a slot the host chooses,
//! and drawn together in one pass ([`Renderer::set_drawn`]): a room the
//! player is about to enter, or the two rooms of a switch, the old one
//! dissolving into the new. A room is uploaded whole with
//! [`Renderer::load_room_into`], or a share at a time with
//! [`Renderer::begin_room`] and [`Renderer::upload_rooms`], so that loading
//! one in the background never takes a frame's whole budget.

use std::cell::Cell;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use glam::Mat4;
use sariema_core::Camera;
use sariema_format::actor::ActorPack;
use sariema_format::{AlphaMode, Background, CharacterLight, Light, Look, RoomPack, Sun};

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
mod sprites;
mod table;
mod timing;
pub mod ui;

pub use actors::{ActorId, ActorModel, ActorState, DiscShadow};
pub use golden::{GOLDEN_MAX_DELTA, GOLDEN_MAX_RATIO, GoldenDiff, compare_images};
pub use headless::{Headless, render_coverage, render_to_image};
pub use quality::{Quality, RenderPath};
pub use sprites::{SHAPES, Sprite};
pub use timing::{FrameTimings, PassTime};

use actors::Actors;
use cluster::Visible;
use post::Post;
use resources::{PendingRoom, Room};
use scene::Scene;
use shadow::{Assigned, Shadows};
use sprites::Sprites;
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
    /// Particles and ground dots drawn, and those past
    /// `Quality::max_sprites` left out.
    pub sprites: u32,
    pub sprites_dropped: u32,
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
        self.write_at(buffer, 0, bytes);
    }

    pub fn write_at(&self, buffer: &wgpu::Buffer, offset: u64, bytes: &[u8]) {
        self.uploaded.set(self.uploaded.get() + bytes.len() as u64);
        self.queue.write_buffer(buffer, offset, bytes);
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

/// The sun, ambient light, background and character light a frame is lit
/// with, when the host gives them instead of the first drawn room's.
#[derive(Clone, Debug, PartialEq)]
pub struct Lighting {
    pub sun: Option<Sun>,
    /// 9 SH coefficients, as [`sariema_format::Ambient`].
    pub ambient: [[f32; 3]; 9],
    pub background: Background,
    pub character: CharacterLight,
}

impl Lighting {
    /// What `room` is lit with as its pack has it.
    pub fn of(room: &RoomPack) -> Lighting {
        Lighting {
            sun: room.sun.clone(),
            ambient: room.ambient.sh9,
            background: room.background.clone(),
            character: room.character.clone(),
        }
    }
}

/// The resident rooms by slot, and those drawn this frame.
#[derive(Default)]
pub(crate) struct Stage {
    rooms: Vec<Option<Room>>,
    drawn: Vec<u32>,
}

impl Stage {
    pub fn drawn(&self) -> impl Iterator<Item = (u32, &Room)> + '_ {
        (self.drawn.iter()).filter_map(|&s| Some((s, self.rooms.get(s as usize)?.as_ref()?)))
    }

    pub fn is_empty(&self) -> bool {
        self.drawn().next().is_none()
    }

    /// A drawn room; `slot` must be one.
    pub fn room(&self, slot: u32) -> &Room {
        self.rooms[slot as usize]
            .as_ref()
            .expect("drawn rooms are resident")
    }

    fn get_mut(&mut self, slot: u32) -> Option<&mut Room> {
        self.rooms.get_mut(slot as usize)?.as_mut()
    }

    fn first(&self) -> Option<&Room> {
        self.drawn().next().map(|(_, r)| r)
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
    sprites: Sprites,
    stage: Stage,
    pending: Vec<(u32, PendingRoom)>,
    /// The drawn rooms' lights that are on, and their slot and index.
    lights: Vec<Light>,
    light_keys: Vec<(u32, u32)>,
    /// The drawn set or a light switched since `lights` was made.
    lights_stale: bool,
    lighting: Option<Lighting>,
    ui: UiPass,
    visible: Visible,
    stats: FrameStats,
    timings: FrameTimings,
    cpu_clock: Option<fn() -> Duration>,
    warned: bool,
    /// The next frame keeps the one drawn before it.
    hold: bool,
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
        let sprites = Sprites::new(&gpu, scene.layouts(), &quality);
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
            sprites,
            stage: Stage::default(),
            pending: Vec::new(),
            lights: Vec::new(),
            light_keys: Vec::new(),
            lights_stale: true,
            lighting: None,
            ui,
            visible: Visible::default(),
            stats: FrameStats::default(),
            timings: FrameTimings::default(),
            cpu_clock: None,
            warned: false,
            hold: false,
        })
    }

    pub fn effective_quality(&self) -> &Quality {
        &self.quality
    }

    /// Uploads the room's meshes, textures and materials, replacing every
    /// room loaded before, draws it alone and applies its look.
    pub fn load_room(&mut self, room: &RoomPack) -> Result<(), RenderError> {
        self.stage.rooms.clear();
        self.pending.clear();
        self.load_room_into(0, room)?;
        self.set_drawn(&[0]);
        self.set_look(&room.look);
        self.warned = false;
        Ok(())
    }

    /// Uploads `room` whole into `slot`, replacing what the slot held.
    pub fn load_room_into(&mut self, slot: u32, room: &RoomPack) -> Result<(), RenderError> {
        let uploaded = resources::upload(&self.gpu, self.scene.layouts(), room)?;
        self.put(slot, uploaded);
        Ok(())
    }

    /// Starts uploading `room` into `slot`, its buffers and textures made
    /// empty; [`Renderer::upload_rooms`] fills them. The slot holds what it
    /// held until the upload ends.
    pub fn begin_room(&mut self, slot: u32, room: &RoomPack) -> Result<u64, RenderError> {
        self.pending.retain(|(s, _)| *s != slot);
        let pending = PendingRoom::begin(&self.gpu, room)?;
        let total = pending.total;
        self.pending.push((slot, pending));
        Ok(total)
    }

    /// Uploads about `budget` bytes of the room begun in `slot`, given its
    /// pack again; true once it is resident.
    pub fn upload_room(&mut self, slot: u32, room: &RoomPack, budget: u64) -> bool {
        let Some(k) = self.pending.iter().position(|(s, _)| *s == slot) else {
            return self.resident(slot);
        };
        if !self.pending[k].1.upload(&self.gpu, room, budget) {
            return false;
        }
        let (_, pending) = self.pending.swap_remove(k);
        let uploaded = pending.finish(&self.gpu, self.scene.layouts(), room);
        self.put(slot, uploaded);
        true
    }

    /// Bytes of the room begun in `slot` uploaded so far, and in all.
    pub fn upload_progress(&self, slot: u32) -> Option<(u64, u64)> {
        let (_, p) = self.pending.iter().find(|(s, _)| *s == slot)?;
        Some((p.uploaded, p.total))
    }

    /// Whether `slot` holds an uploaded room.
    pub fn resident(&self, slot: u32) -> bool {
        self.stage
            .rooms
            .get(slot as usize)
            .is_some_and(Option::is_some)
    }

    /// Frees the room in `slot` and any upload begun there.
    pub fn unload_room(&mut self, slot: u32) {
        self.pending.retain(|(s, _)| *s != slot);
        if let Some(room) = self.stage.rooms.get_mut(slot as usize) {
            *room = None;
        }
        if self.stage.drawn.contains(&slot) {
            let drawn: Vec<u32> = (self.stage.drawn.iter())
                .copied()
                .filter(|&s| s != slot)
                .collect();
            self.set_drawn(&drawn);
        }
    }

    fn put(&mut self, slot: u32, room: Room) {
        let i = slot as usize;
        if self.stage.rooms.len() <= i {
            self.stage.rooms.resize_with(i + 1, || None);
        }
        self.stage.rooms[i] = Some(room);
        if self.stage.drawn.contains(&slot) {
            self.restage();
        }
    }

    /// Draws the rooms in `slots`, the resident ones, in that order; without
    /// [`Renderer::set_lighting`] the first lights the frame.
    pub fn set_drawn(&mut self, slots: &[u32]) {
        if self.stage.drawn == slots {
            return;
        }
        self.stage.drawn.clear();
        self.stage.drawn.extend_from_slice(slots);
        self.restage();
    }

    fn restage(&mut self) {
        self.lights_stale = true;
        self.refresh_lights();
        let sun = self.frame_sun().cloned();
        self.shadows.reset(&self.stage, &self.lights, sun.as_ref());
    }

    /// Lights the frames with `lighting`; `None` lights them with the first
    /// drawn room's own.
    pub fn set_lighting(&mut self, lighting: Option<Lighting>) {
        let shadowed = |l: &Option<Lighting>| {
            l.as_ref()
                .and_then(|l| l.sun.as_ref())
                .map(|s| s.casts_shadow)
        };
        let reset = shadowed(&lighting) != shadowed(&self.lighting);
        self.lighting = lighting;
        if reset {
            self.restage();
        }
    }

    fn frame_sun(&self) -> Option<&Sun> {
        match &self.lighting {
            Some(l) => l.sun.as_ref(),
            None => self.stage.first().and_then(|r| r.sun.as_ref()),
        }
    }

    /// Switches group `group` of the room in `slot`; false if there is none.
    pub fn set_group(&mut self, slot: u32, group: usize, on: bool) -> bool {
        self.stage
            .get_mut(slot)
            .is_some_and(|r| r.set_group(group, on))
    }

    /// Shows state `state` of piece `piece` of the room in `slot`; false if
    /// there is none.
    pub fn set_piece_state(&mut self, slot: u32, piece: usize, state: u32) -> bool {
        self.stage
            .get_mut(slot)
            .is_some_and(|r| r.set_piece(piece, state))
    }

    /// Shows or hides node `node` of the room in `slot`; false if there is none.
    pub fn set_node_visible(&mut self, slot: u32, node: usize, visible: bool) -> bool {
        self.stage
            .get_mut(slot)
            .is_some_and(|r| r.set_node_visible(node, visible))
    }

    /// Moves node `node`'s draws by `offset`, column-major, from where the
    /// pack placed them; false if there is none.
    pub fn set_node_offset(&mut self, slot: u32, node: usize, offset: Mat4) -> bool {
        self.stage
            .get_mut(slot)
            .is_some_and(|r| r.set_node_offset(node, offset))
    }

    /// Makes node `node` of the room in `slot` glow in its own colour: its
    /// albedo and vertex colour, brightest channel at 1, added over all of
    /// it at `fill` and at its rim at `rim`; false if there is none.
    pub fn set_node_glow(&mut self, slot: u32, node: usize, fill: f32, rim: f32) -> bool {
        self.stage
            .get_mut(slot)
            .is_some_and(|r| r.set_node_glow(node, fill, rim))
    }

    /// How much of the room in `slot` is drawn: 1 whole, 0 nothing; between,
    /// its opaque draws dither, the others fade, and it casts shadows from
    /// halfway. `inverted` dithers it in the complement
    /// of the pattern, so a room at `p` and one at `1 − p` inverted share
    /// no pixel: a dissolve.
    pub fn set_room_presence(&mut self, slot: u32, presence: f32, inverted: bool) {
        let Some(room) = self.stage.get_mut(slot) else {
            return;
        };
        let presence = presence.clamp(0.0, 1.0);
        let shadowing = |p: f32| p >= shadow::SHADOWING;
        let crossed = shadowing(room.presence) != shadowing(presence);
        room.set_presence(presence, inverted);
        if crossed && self.stage.drawn.contains(&slot) {
            self.restage();
        }
    }

    /// Multiplies the intensity of every light of the room in `slot`, as
    /// when one room's light gives way to another's.
    pub fn set_room_light_scale(&mut self, slot: u32, scale: f32) {
        if let Some(room) = self.stage.get_mut(slot) {
            room.light_scale = scale.max(0.0);
        }
    }

    /// Sets the colour (linear RGB), intensity (candela) and range (metres)
    /// of light `light` of the room in `slot`; false if there is none.
    pub fn set_light_values(
        &mut self,
        slot: u32,
        light: usize,
        color: [f32; 3],
        intensity: f32,
        range: f32,
    ) -> bool {
        let Some(l) = self
            .stage
            .get_mut(slot)
            .and_then(|r| r.lights.get_mut(light))
        else {
            return false;
        };
        l.color = color;
        l.intensity = intensity;
        l.range = range;
        true
    }

    /// Moves light `light` of the room in `slot` to `position`, a spot
    /// pointing at `target`; false if there is none.
    pub fn set_light_position(
        &mut self,
        slot: u32,
        light: usize,
        position: [f32; 3],
        target: Option<[f32; 3]>,
    ) -> bool {
        let Some(l) = self
            .stage
            .get_mut(slot)
            .and_then(|r| r.lights.get_mut(light))
        else {
            return false;
        };
        l.position = position;
        if let (sariema_format::LightKind::Spot { target: aim, .. }, Some(target)) =
            (&mut l.kind, target)
        {
            *aim = target;
        }
        true
    }

    /// Switches light `light` of the room in `slot`; off, it takes no place
    /// among the frame's lights. False if there is none.
    pub fn set_light_on(&mut self, slot: u32, light: usize, on: bool) -> bool {
        let Some(room) = self.stage.get_mut(slot) else {
            return false;
        };
        let Some(was) = room.light_on.get_mut(light) else {
            return false;
        };
        if *was != on {
            *was = on;
            room.lights_switched = true;
        }
        true
    }

    /// The frame's lights: rebuilt when the drawn set or a light switched,
    /// otherwise their values copied in place, which allocates nothing.
    fn refresh_lights(&mut self) {
        let mut switched = false;
        for room in self.stage.rooms.iter_mut().flatten() {
            switched |= std::mem::take(&mut room.lights_switched);
        }
        if switched || self.lights_stale {
            self.lights_stale = false;
            self.lights.clear();
            self.light_keys.clear();
            for (slot, room) in self.stage.drawn() {
                for (i, light) in room.lights.iter().enumerate() {
                    if room.light_on[i] {
                        self.lights.push(Light {
                            intensity: light.intensity * room.light_scale,
                            ..light.clone()
                        });
                        self.light_keys.push((slot, i as u32));
                    }
                }
            }
            if switched {
                let sun = self.frame_sun().cloned();
                self.shadows.reset(&self.stage, &self.lights, sun.as_ref());
            }
            return;
        }
        for (light, &(slot, i)) in self.lights.iter_mut().zip(&self.light_keys) {
            let room = self.stage.room(slot);
            let source = &room.lights[i as usize];
            light.color = source.color;
            light.intensity = source.intensity * room.light_scale;
            light.range = source.range;
            light.position = source.position;
            light.kind = source.kind.clone();
        }
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

    /// Makes `actor` glow in its own colour, as [`Renderer::set_node_glow`]
    /// does a node.
    pub fn set_actor_glow(
        &mut self,
        actor: ActorId,
        fill: f32,
        rim: f32,
    ) -> Result<(), RenderError> {
        self.actors.set_glow(actor, fill, rim)
    }

    /// Replaces the actor's disc shadow; `None` draws none.
    pub fn set_actor_disc(
        &mut self,
        actor: ActorId,
        disc: Option<DiscShadow>,
    ) -> Result<(), RenderError> {
        self.actors.set_disc(actor, disc)
    }

    /// The particles and ground dots of the frames from the next one on.
    pub fn set_sprites(&mut self, sprites: &[Sprite]) {
        self.sprites.stage(sprites);
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

    /// Keeps the frame drawn last, as it was composited, before the HUD:
    /// the next [`Renderer::render`] composites it again into a texture
    /// before drawing anything, at the cost of one full-screen pass. Shown
    /// over the frames after it by [`Renderer::set_held_frame`].
    pub fn hold_frame(&mut self) {
        self.hold = true;
    }

    /// How much of the frame held shows over every frame drawn: 1 only it,
    /// 0 none, which skips its pass.
    pub fn set_held_frame(&mut self, opacity: f32) {
        self.post.set_held_opacity(&self.gpu, opacity);
    }

    /// Shows or hides every draw called `name` in the drawn rooms; false if
    /// there is none.
    pub fn set_draw_visible(&mut self, name: &str, visible: bool) -> bool {
        let mut found = false;
        for &slot in &self.stage.drawn {
            let Some(Some(room)) = self.stage.rooms.get_mut(slot as usize) else {
                continue;
            };
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
        self.refresh_lights();
        for room in self.stage.rooms.iter_mut().flatten() {
            for (min, max) in room.toggled.drain(..) {
                self.shadows.caster_toggled(min, max);
            }
            room.flush(&self.gpu);
        }
        self.visible.find(
            &self.lights,
            camera.view,
            camera.proj,
            camera.position,
            self.quality.max_visible_lights,
        );
        self.actors.upload(&self.gpu);
        self.sprites.prepare(&self.gpu, camera, self.height);
        let lighting = match &self.lighting {
            Some(l) => Some(l.clone()),
            None => self.stage.first().map(|r| Lighting {
                sun: r.sun.clone(),
                ambient: r.ambient,
                background: r.background.clone(),
                character: r.character.clone(),
            }),
        };
        let sun = lighting.as_ref().and_then(|l| l.sun.as_ref());
        self.shadows.assign(
            &self.gpu,
            &self.stage,
            (&self.lights, sun),
            camera,
            &self.visible.lights,
            self.actors.moved(),
        );
        let draws_culled = self.scene.cull(&self.stage, camera);
        let actor_stats = (self.actors).prepare(camera, |b| self.shadows.maps(b));
        self.actors.forget_moves();
        let cull = watch.lap();
        let mut stats = self.scene.prepare(
            &self.gpu,
            &self.lights,
            lighting.as_ref(),
            camera,
            time_s,
            &self.visible,
            self.shadows.assigned(),
        );
        let bin = watch.lap();
        stats.draws_culled = draws_culled;
        stats.actors_drawn = actor_stats.drawn;
        stats.actors_culled = actor_stats.culled;
        stats.sprites = self.sprites.count();
        stats.sprites_dropped = self.sprites.dropped;
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
        let size = (self.width, self.height);
        let held = std::mem::take(&mut self.hold) && self.post.hold(&self.gpu, &mut encoder, size);
        if !self.stage.is_empty() {
            (self.shadows).draw(
                &self.gpu,
                &mut encoder,
                &self.stage,
                &self.lights,
                &self.actors,
            );
        }
        let keep_depth = self.post.reads_depth();
        (self.scene).draw(
            &self.gpu,
            &mut encoder,
            &self.stage,
            (&self.actors, &self.sprites),
            keep_depth,
        );
        let record = watch.lap();
        self.stats.post_passes =
            held as u32 + self.post.draw(&self.gpu, &mut encoder, camera, target);
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
        let (none, no_shadows) = (Visible::default(), Assigned::default());
        self.scene.cull(&self.stage, camera);
        (self.scene).prepare(&self.gpu, &[], None, camera, 0.0, &none, &no_shadows);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let covers = |alpha| alpha != AlphaMode::Additive;
        (self.scene).draw_depth(&self.gpu, &mut encoder, &self.stage, None, depth, covers);
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
