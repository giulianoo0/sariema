// SPDX-License-Identifier: Apache-2.0

//! Recording actors: their parts into the scene pass, their discs, and their
//! depth into shadow slots.

use glam::Vec3;
use sariema_format::AlphaMode;

use super::{Actors, Instance, ModelGpu, PartGpu};

/// Which of an actor's parts a call draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    Opaque,
    /// Blended and additive parts, after the room's.
    Translucent,
}

impl Actors {
    /// Draws the `layer` parts of the frame's actors in view; the pass has
    /// the frame group at 0. Returns the draws recorded.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, layer: Layer) -> u32 {
        let Some(pipelines) = &self.pipelines else {
            return 0;
        };
        let mut drawn = 0;
        let mut current = None;
        for &i in &self.in_view {
            let (instance, model) = self.of(i);
            let parts = visible_parts(instance, model).filter(|p| match layer {
                Layer::Opaque => p.kind.is_opaque(),
                Layer::Translucent => !p.kind.is_opaque(),
            });
            let mut bound = false;
            for part in parts {
                if !bound {
                    bind(pass, instance, model);
                    bound = true;
                }
                if current != Some(part.kind) {
                    pass.set_pipeline(pipelines.kind(part.kind));
                    current = Some(part.kind);
                }
                let material = &model.materials[part.material as usize];
                if part.kind.alpha == AlphaMode::Blend {
                    let mask = material.drawn as u8 as f64;
                    pass.set_blend_constant(wgpu::Color {
                        r: mask,
                        g: mask,
                        b: mask,
                        a: mask,
                    });
                }
                pass.set_bind_group(2, &material.group, &[]);
                draw_part(pass, part);
                drawn += 1;
            }
        }
        drawn
    }

    /// Darkens the floor under the frame's actors that no shadow map holds.
    /// Returns the draws recorded.
    pub fn draw_discs(&self, pass: &mut wgpu::RenderPass<'_>) -> u32 {
        let Some(pipelines) = &self.pipelines else {
            return 0;
        };
        if self.discs.is_empty() {
            return 0;
        }
        pass.set_pipeline(&pipelines.disc);
        for &i in &self.discs {
            let (instance, _) = self.of(i);
            pass.set_bind_group(1, instance.group.as_ref(), &[]);
            pass.draw(0..6, 0..1);
        }
        self.discs.len() as u32
    }

    /// Draws the depth of the casting actors that `reaches` says can be in
    /// the slot whose view is bound at 2 with `view_offset`. Returns the
    /// draws recorded.
    pub fn draw_shadows(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        view: &wgpu::BindGroup,
        view_offset: u32,
        reaches: impl Fn((Vec3, Vec3)) -> bool,
    ) -> u32 {
        let Some(pipelines) = &self.pipelines else {
            return 0;
        };
        let mut drawn = 0;
        let mut pipeline_set = false;
        for instance in self.instances.iter().filter(|i| i.drawn()) {
            let model = &self.models[instance.model];
            if !model.casts_shadow() || !reaches(instance.bounds) {
                continue;
            }
            if !pipeline_set {
                pass.set_pipeline(&pipelines.shadow);
                pass.set_bind_group(2, view, &[view_offset]);
                pipeline_set = true;
            }
            bind(pass, instance, model);
            for part in visible_parts(instance, model).filter(|p| p.casts_shadow) {
                draw_part(pass, part);
                drawn += 1;
            }
        }
        drawn
    }

    fn of(&self, i: u32) -> (&Instance, &ModelGpu) {
        let instance = &self.instances[i as usize];
        (instance, &self.models[instance.model])
    }
}

fn visible_parts<'a>(
    instance: &'a Instance,
    model: &'a ModelGpu,
) -> impl Iterator<Item = &'a PartGpu> {
    (model.parts.iter())
        .zip(&instance.shown)
        .filter(|(p, shown)| **shown && p.index_count > 0)
        .map(|(p, _)| p)
}

fn bind(pass: &mut wgpu::RenderPass<'_>, instance: &Instance, model: &ModelGpu) {
    pass.set_bind_group(1, instance.group.as_ref(), &[]);
    pass.set_vertex_buffer(0, model.vertices.slice(..));
    pass.set_index_buffer(model.indices.slice(..), wgpu::IndexFormat::Uint32);
}

fn draw_part(pass: &mut wgpu::RenderPass<'_>, part: &PartGpu) {
    pass.draw_indexed(
        part.first_index..part.first_index + part.index_count,
        part.base_vertex,
        0..1,
    );
}
