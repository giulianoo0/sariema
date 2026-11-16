// SPDX-License-Identifier: Apache-2.0

//! What a host draws for a frame, `alpha` of the way from the last step to
//! the next: poses and the camera interpolated, every animator posed at the
//! same point (each step advances it by exactly the runtime clip clock's
//! delta, so this is the clip time `clip_time_at(alpha)` gives), and the HUD.

use glam::{Mat4, Vec3};
use sariema_format::script::Entity;
use sariema_runtime::camera::CameraPose;
use sariema_runtime::sim::Logic;
use sariema_ui::DrawList;

use crate::Engine;
use crate::anim::Model;

pub struct ActorView<'a> {
    pub entity: Entity,
    /// Index into [`Engine::models`].
    pub model: usize,
    pub transform: Mat4,
    pub palette: &'a [Mat4],
    pub faces: &'a [u32],
    pub visible: bool,
}

impl<L: Logic> Engine<L> {
    pub fn models(&self) -> &[Model] {
        &self.anim.models
    }

    pub fn camera(&self, alpha: f32) -> Option<CameraPose> {
        self.sim.state().camera.interpolated(alpha)
    }

    /// Every actor drawn with a model, posed at `alpha`.
    pub fn actors(&mut self, alpha: f32) -> impl Iterator<Item = ActorView<'_>> {
        self.anim.evaluate(alpha);
        let world = &self.sim.state().world;
        self.anim
            .actors()
            .filter_map(move |(entity, model, animator)| {
                let node = world.node(entity)?;
                let pose = world.interpolated(entity, alpha)?;
                Some(ActorView {
                    entity,
                    model,
                    transform: Mat4::from_scale_rotation_translation(
                        Vec3::splat(pose.scale),
                        pose.rotation,
                        pose.position,
                    ),
                    palette: animator.palette(),
                    faces: animator.faces(),
                    visible: node.visible,
                })
            })
    }

    /// The HUD and the engine's layer over it. The fade and the cinema bars
    /// are drawn at step values whatever `alpha` is, never between two: a
    /// teleport under a full fade is never seen through a fade drawn
    /// halfway.
    pub fn draw_hud(&mut self, alpha: f32) -> &DrawList {
        self.hud.draw(alpha)
    }
}
