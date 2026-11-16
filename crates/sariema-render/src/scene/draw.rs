// SPDX-License-Identifier: Apache-2.0

//! Recording the scene's draws: the background, then the drawn rooms'
//! visible draws in order with the actors after their opaque ones, or their
//! depth alone.

use sariema_core::Camera;
use sariema_format::AlphaMode;

use super::{Scene, Targets};
use crate::actors::{Actors, Layer};
use crate::resources::Room;
use crate::{Gpu, Stage, cull};

impl Scene {
    /// `keep_depth` keeps the depth for post-processing, stored or drawn
    /// again into the single-sample depth; otherwise it is discarded, which
    /// spares tile-based GPUs writing it out.
    pub fn draw(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        stage: &Stage,
        actors: &Actors,
        keep_depth: bool,
    ) {
        let Targets {
            multisampled,
            resolved,
            depth,
            single_depth,
        } = &self.targets;
        let depth_store = match keep_depth && single_depth.is_none() {
            true => wgpu::StoreOp::Store,
            false => wgpu::StoreOp::Discard,
        };
        let (view, resolve_target) = match multisampled {
            Some(samples) => (samples, Some(resolved)),
            None => (resolved, None),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: if resolve_target.is_some() {
                        wgpu::StoreOp::Discard
                    } else {
                        wgpu::StoreOp::Store
                    },
                },
            })],
            depth_stencil_attachment: Some(depth_attachment(depth, depth_store)),
            timestamp_writes: gpu.pass("scene"),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.frame_group, &[]);
        pass.set_pipeline(&self.background);
        pass.draw(0..3, 0..1);
        // The background, then the rooms' draws in view.
        gpu.count_draws(1 + self.order.len() as u32);
        let mut bound: Option<u32> = None;
        let mut current = None;
        let mut actors_drawn = false;
        for &(layer, _, slot, i) in &self.order {
            // Actors and their discs go between the opaque and the rest.
            if layer > 0 && !actors_drawn {
                gpu.count_draws(actors.draw(&mut pass, Layer::Opaque));
                gpu.count_draws(actors.draw_discs(&mut pass));
                actors_drawn = true;
                bound = None;
                current = None;
            }
            let room = stage.room(slot);
            if bound != Some(slot) {
                room.bind_meshes(&mut pass);
                bound = Some(slot);
            }
            let i = i as usize;
            let draw = &room.draws[i];
            let pipeline = (draw.dithered(), draw.kind);
            if current != Some(pipeline) {
                pass.set_pipeline(match pipeline {
                    (true, kind) => &self.dithered[kind.dither_index()],
                    (false, kind) => &self.pipelines[kind.index()],
                });
                current = Some(pipeline);
            }
            let material = &room.materials[draw.material as usize];
            if draw.kind.alpha == AlphaMode::Blend {
                let mask = material.drawn as u8 as f64;
                pass.set_blend_constant(wgpu::Color {
                    r: mask,
                    g: mask,
                    b: mask,
                    a: mask,
                });
            }
            pass.set_bind_group(2, &material.group, &[]);
            room.draw(&mut pass, i);
        }
        if !actors_drawn {
            gpu.count_draws(actors.draw(&mut pass, Layer::Opaque));
            gpu.count_draws(actors.draw_discs(&mut pass));
        }
        gpu.count_draws(actors.draw(&mut pass, Layer::Translucent));
        drop(pass);
        if let Some(single) = single_depth.as_ref().filter(|_| keep_depth) {
            let opaque = |alpha| alpha == AlphaMode::Opaque;
            self.draw_depth(gpu, encoder, stage, Some(actors), single, opaque);
        }
    }

    /// The depth of the rooms' draws in view whose alpha mode `covers`
    /// keeps and, if given, of the actors' opaque parts, into a
    /// single-sample `depth` cleared to 1. A dithering draw is left out.
    pub fn draw_depth(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        stage: &Stage,
        actors: Option<&Actors>,
        depth: &wgpu::TextureView,
        covers: impl Fn(AlphaMode) -> bool,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("depth only"),
            depth_stencil_attachment: Some(depth_attachment(depth, wgpu::StoreOp::Store)),
            timestamp_writes: gpu.pass("depth only"),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.frame_group, &[]);
        let mut bound: Option<u32> = None;
        let mut current = None;
        for &(_, _, slot, i) in &self.order {
            let room: &Room = stage.room(slot);
            let i = i as usize;
            let draw = &room.draws[i];
            if !covers(draw.kind.alpha) || draw.dithered() {
                continue;
            }
            if bound != Some(slot) {
                room.bind_meshes(&mut pass);
                bound = Some(slot);
            }
            let double_sided = draw.kind.double_sided;
            if current != Some(double_sided) {
                pass.set_pipeline(&self.depth_only[double_sided as usize]);
                current = Some(double_sided);
            }
            room.draw(&mut pass, i);
            gpu.count_draws(1);
        }
        if let Some(actors) = actors {
            gpu.count_draws(actors.draw_depth(&mut pass));
        }
    }
}

impl Scene {
    /// Orders the drawn rooms' visible draws for `camera`: opaque front to
    /// back, then blended back to front, then additive. With
    /// `Quality::frustum_culling`, draws whose bounds lie outside the view
    /// are left out; returns how many.
    pub(super) fn order(&mut self, stage: &Stage, camera: &Camera) -> u32 {
        let planes = cull::planes(camera.proj * camera.view);
        let eye = camera.position;
        let mut culled = 0;
        self.order.clear();
        for (slot, room) in stage.drawn() {
            for (i, d) in room.draws.iter().enumerate() {
                if !d.visible || d.index_count == 0 || d.presence == 0.0 {
                    continue;
                }
                if self.frustum_culling && !cull::box_visible(&planes, d.min, d.max) {
                    culled += 1;
                    continue;
                }
                let distance2 = eye.distance_squared(d.center());
                let i = i as u32;
                self.order.push(match d.kind.alpha {
                    AlphaMode::Opaque => (0, distance2, slot, i),
                    AlphaMode::Blend => (1, -distance2, slot, i),
                    AlphaMode::Additive => (2, 0.0, slot, i),
                });
            }
        }
        // The slot and index break ties, so an unstable sort, which needs no
        // buffer, gives the order a stable one would.
        self.order.sort_unstable_by(|a, b| {
            (a.0.cmp(&b.0))
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        culled
    }
}

fn depth_attachment(
    view: &wgpu::TextureView,
    store: wgpu::StoreOp,
) -> wgpu::RenderPassDepthStencilAttachment<'_> {
    wgpu::RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(1.0),
            store,
        }),
        stencil_ops: None,
    }
}
