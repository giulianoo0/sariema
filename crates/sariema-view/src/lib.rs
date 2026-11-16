// SPDX-License-Identifier: Apache-2.0

//! A game in the engine drawn by the renderer: the rooms of its pack, loaded
//! and shown as `rooms` tells, a drawn actor for every animated entity, the
//! camera the runtime gives at the frame's `alpha`, and the HUD over it all.

mod rooms;
mod slide;

pub use rooms::{RoomsFrame, ViewSettings};

use glam::Mat4;
use sariema_core::{Camera, pose_camera, room_camera};
use sariema_engine::Engine;
use sariema_format::script::{Entity, GameAssets};
use sariema_format::{RoomCamera, RoomPack};
use sariema_render::ui::{UiBatch, UiInstance, UiTextureFormat};
use sariema_render::{ActorId, ActorModel, ActorState, RenderError, Renderer};
use sariema_runtime::sim::Logic;

/// Why a game cannot be drawn, with the stable code of the CLI's errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewError {
    pub code: &'static str,
    pub message: String,
}

impl ViewError {
    fn new(code: &'static str, message: impl Into<String>) -> ViewError {
        ViewError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ViewError {}

impl From<RenderError> for ViewError {
    fn from(e: RenderError) -> ViewError {
        ViewError::new(e.code(), e.to_string())
    }
}

pub struct GameView {
    /// By the engine's model index, with a palette of as many joints.
    models: Vec<ActorModel>,
    rest: Vec<Vec<Mat4>>,
    /// By entity slot: the entity, its model and the actor drawing it.
    drawn: Vec<Option<(Entity, usize, ActorId)>>,
    /// Hidden actors of entities gone, by model, for the next ones.
    free: Vec<(usize, ActorId)>,
    batches: Vec<UiBatch>,
    images: usize,
    /// Whether this view's renderer has the HUD's glyph atlas, which a
    /// new renderer for a running game lacks.
    atlas: bool,
    rooms: rooms::Rooms,
    /// The start room's camera, for a game that set none in a room without
    /// one.
    camera: RoomCamera,
    slide: slide::Slide,
}

/// The room a game pack starts in, decoded.
pub fn room(assets: &GameAssets) -> Result<RoomPack, ViewError> {
    let bytes = (assets.rooms.first())
        .and_then(|r| r.room.as_ref())
        .ok_or_else(|| {
            ViewError::new(
                "E_SCHEMA",
                "game pack: it has no room to draw; give the project a room",
            )
        })?;
    sariema_format::decode(bytes).map_err(|e| ViewError::new(e.code(), format!("game pack: {e}")))
}

/// Turns off every shadow of `room`, for a device without the budget.
pub fn without_shadows(room: &mut RoomPack) {
    if let Some(sun) = &mut room.sun {
        sun.casts_shadow = false;
    }
    for light in &mut room.lights {
        light.casts_shadow = false;
    }
}

impl GameView {
    /// Loads the room the game is in and the actor packs of the engine's
    /// models; the other rooms load as the engine wants them.
    pub fn load<L: Logic>(
        renderer: &mut Renderer,
        assets: &GameAssets,
        engine: &Engine<L>,
    ) -> Result<GameView, ViewError> {
        GameView::load_with(renderer, assets, engine, ViewSettings::default())
    }

    pub fn load_with<L: Logic>(
        renderer: &mut Renderer,
        assets: &GameAssets,
        engine: &Engine<L>,
        settings: ViewSettings,
    ) -> Result<GameView, ViewError> {
        let failed = |code: &'static str, e: &dyn std::fmt::Display| {
            ViewError::new(code, format!("game pack: {e}"))
        };
        let slide = (
            settings.camera_slide_frequency,
            settings.camera_slide_damping,
        );
        let mut rooms = rooms::Rooms::new(engine, settings);
        let active = engine.state().scenery.active();
        let camera = rooms.load_now(renderer, active)?.unwrap_or(RoomCamera {
            fov_y_deg: 40.0,
            pitch_deg: 45.0,
            distance: 10.0,
            focus: [0.0; 3],
        });
        let (mut models, mut rest) = (Vec::new(), Vec::new());
        for model in engine.models() {
            let bytes = assets
                .actors
                .iter()
                .find(|(name, _)| *name == model.name)
                .map(|(_, bytes)| bytes)
                .ok_or_else(|| {
                    ViewError::new("E_SCHEMA", format!("game pack: no actor {}", model.name))
                })?;
            let pack = sariema_format::actor::decode(bytes).map_err(|e| failed(e.code(), &e))?;
            models.push(renderer.load_actor(&pack)?);
            rest.push(vec![Mat4::IDENTITY; pack.joints.len()]);
        }
        Ok(GameView {
            models,
            rest,
            drawn: Vec::new(),
            free: Vec::new(),
            batches: Vec::new(),
            images: 0,
            atlas: false,
            rooms,
            slide: slide::Slide::new(active, engine.steps(), camera.clone(), slide.0, slide.1),
            camera,
        })
    }

    /// Loads, frees and shows the rooms as a frame at `alpha` would, without
    /// drawing it: for a host that skips drawing some frames.
    pub fn update_rooms<L: Logic>(
        &mut self,
        renderer: &mut Renderer,
        engine: &mut Engine<L>,
        alpha: f32,
    ) -> Result<(), ViewError> {
        self.rooms.update(renderer, engine, alpha)?;
        self.follow_rooms(engine);
        Ok(())
    }

    fn follow_rooms<L: Logic>(&mut self, engine: &Engine<L>) {
        let (rooms, start) = (&self.rooms, &self.camera);
        let camera = |room: usize| rooms.camera(room).unwrap_or(start).clone();
        self.slide.advance(engine, &camera);
    }

    /// What loading rooms did in the last frame drawn.
    pub fn rooms_frame(&self) -> RoomsFrame {
        self.rooms.frame()
    }

    /// The runtime's camera at `alpha`; the room's own before the game
    /// chose one, sliding to the next room's across a continuous switch.
    pub fn camera<L: Logic>(&self, engine: &Engine<L>, alpha: f32, aspect: f32) -> Camera {
        let active = engine.state().scenery.active();
        let room = &self
            .slide
            .at(self.rooms.camera(active).unwrap_or(&self.camera), alpha);
        match engine.camera(alpha) {
            Some(pose) => pose_camera(
                pose.position,
                pose.rotation,
                pose.fov,
                aspect,
                room.distance,
            ),
            None => room_camera(room, aspect),
        }
    }

    /// Draws the frame `alpha` of the way to the next step into `target`,
    /// the renderer's size, then the HUD over it.
    pub fn draw<L: Logic>(
        &mut self,
        renderer: &mut Renderer,
        engine: &mut Engine<L>,
        alpha: f32,
        aspect: f32,
        target: &wgpu::TextureView,
    ) -> Result<(), ViewError> {
        self.world(renderer, engine, alpha, aspect, target)?;
        self.hud(renderer, engine, alpha, target);
        Ok(())
    }

    /// The room and the actors, seen by the camera at `alpha`.
    pub fn world<L: Logic>(
        &mut self,
        renderer: &mut Renderer,
        engine: &mut Engine<L>,
        alpha: f32,
        aspect: f32,
        target: &wgpu::TextureView,
    ) -> Result<(), ViewError> {
        self.rooms.update(renderer, engine, alpha)?;
        self.follow_rooms(engine);
        self.pose(renderer, engine, alpha)?;
        let camera = self.camera(engine, alpha, aspect);
        let time_s = engine.steps() as f32 * sariema_core::STEP_SECONDS;
        renderer.render(&camera, time_s, target);
        Ok(())
    }

    /// The HUD over what `target` holds, its glyphs and images uploaded
    /// when they changed.
    pub fn hud<L: Logic>(
        &mut self,
        renderer: &mut Renderer,
        engine: &mut Engine<L>,
        alpha: f32,
        target: &wgpu::TextureView,
    ) {
        let ui = &mut engine.hud_mut().ui;
        if ui.take_atlas_changed() || !self.atlas {
            self.atlas = true;
            let (side, texels) = ui.atlas();
            renderer.set_ui_texture(
                sariema_ui::ATLAS_TEXTURE,
                side,
                side,
                UiTextureFormat::R8,
                texels,
            );
        }
        for (i, image) in ui.images().iter().enumerate().skip(self.images) {
            renderer.set_ui_texture(
                1 + i as u32,
                image.width,
                image.height,
                UiTextureFormat::Rgba8,
                &image.rgba,
            );
        }
        self.images = ui.images().len();
        let list = engine.draw_hud(alpha);
        self.batches.clear();
        self.batches.extend(list.batches.iter().map(|b| UiBatch {
            texture: b.texture,
            scissor: b.scissor,
            start: b.start,
            count: b.count,
        }));
        let instances: &[UiInstance] = bytemuck::cast_slice(&list.quads);
        renderer.render_ui(target, instances, &self.batches);
    }

    /// One drawn actor per animated entity, posed at `alpha`; an entity gone
    /// hides its actor for the next of its model.
    fn pose<L: Logic>(
        &mut self,
        renderer: &mut Renderer,
        engine: &mut Engine<L>,
        alpha: f32,
    ) -> Result<(), ViewError> {
        for slot in &mut self.drawn {
            if let Some((entity, model, id)) = *slot
                && !engine.state().world.is_alive(entity)
            {
                hide(renderer, id, &self.rest[model])?;
                self.free.push((model, id));
                *slot = None;
            }
        }
        for actor in engine.actors(alpha) {
            let i = actor.entity.index as usize;
            if self.drawn.len() <= i {
                self.drawn.resize(i + 1, None);
            }
            let id = match self.drawn[i] {
                Some((entity, model, id)) if entity == actor.entity && model == actor.model => id,
                old => {
                    if let Some((_, model, id)) = old {
                        hide(renderer, id, &self.rest[model])?;
                        self.free.push((model, id));
                    }
                    let id = match self.free.iter().position(|&(m, _)| m == actor.model) {
                        Some(k) => self.free.swap_remove(k).1,
                        None => renderer.spawn_actor(self.models[actor.model])?,
                    };
                    self.drawn[i] = Some((actor.entity, actor.model, id));
                    id
                }
            };
            let state = ActorState {
                model: actor.transform,
                palette: actor.palette,
                faces: actor.faces,
                visible: actor.visible,
            };
            renderer.set_actor(id, &state)?;
        }
        Ok(())
    }
}

fn hide(renderer: &mut Renderer, id: ActorId, rest: &[Mat4]) -> Result<(), ViewError> {
    let state = ActorState {
        model: Mat4::IDENTITY,
        palette: rest,
        faces: &[],
        visible: false,
    };
    Ok(renderer.set_actor(id, &state)?)
}
