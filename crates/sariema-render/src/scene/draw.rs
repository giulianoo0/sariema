// SPDX-License-Identifier: Apache-2.0

//! Recording the scene's draws: the background, then the room's visible
//! draws in order, or their depth alone.

use sariema_core::Camera;
use sariema_format::AlphaMode;

use super::{Scene, Targets};
use crate::cull;
use crate::resources::Room;

impl Scene {
    /// `keep_depth` stores the depth for post-processing; otherwise it is
    /// discarded, which spares tile-based GPUs writing it out.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, room: Option<&Room>, keep_depth: bool) {
        let Targets {
            multisampled,
            resolved,
            depth,
        } = &self.targets;
        let depth_store = match keep_depth {
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
            ..Default::default()
        });
        pass.set_bind_group(0, &self.frame_group, &[]);
        pass.set_pipeline(&self.background);
        pass.draw(0..3, 0..1);
        let Some(room) = room else {
            return;
        };
        if self.order.is_empty() {
            return;
        }
        room.bind_meshes(&mut pass);
        let mut current = None;
        for &(_, _, i) in &self.order {
            let i = i as usize;
            let draw = &room.draws[i];
            if current != Some(draw.kind) {
                pass.set_pipeline(&self.pipelines[draw.kind.index()]);
                current = Some(draw.kind);
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
    }

    pub fn draw_depth(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        room: Option<&Room>,
        depth: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("depth only"),
            depth_stencil_attachment: Some(depth_attachment(depth, wgpu::StoreOp::Store)),
            ..Default::default()
        });
        let Some(room) = room else {
            return;
        };
        let covering = || {
            (self.order.iter())
                .map(|&(_, _, i)| i as usize)
                .filter(|&i| room.draws[i].kind.alpha != AlphaMode::Additive)
        };
        if covering().next().is_none() {
            return;
        }
        pass.set_bind_group(0, &self.frame_group, &[]);
        room.bind_meshes(&mut pass);
        let mut current = None;
        for i in covering() {
            let double_sided = room.draws[i].kind.double_sided;
            if current != Some(double_sided) {
                pass.set_pipeline(&self.depth_only[double_sided as usize]);
                current = Some(double_sided);
            }
            room.draw(&mut pass, i);
        }
    }
}

impl Scene {
    /// Orders the room's visible draws for `camera`: opaque front to back,
    /// then blended back to front, then additive. With
    /// `Quality::frustum_culling`, draws whose bounds lie outside the view
    /// are left out; returns how many.
    pub(super) fn order(&mut self, room: &Room, camera: &Camera) -> u32 {
        let planes = cull::planes(camera.proj * camera.view);
        let eye = camera.position;
        let mut culled = 0;
        self.order.clear();
        for (i, d) in room.draws.iter().enumerate() {
            if !d.visible || d.index_count == 0 {
                continue;
            }
            if self.frustum_culling && !cull::box_visible(&planes, d.min, d.max) {
                culled += 1;
                continue;
            }
            let distance2 = eye.distance_squared(d.center());
            self.order.push(match d.kind.alpha {
                AlphaMode::Opaque => (0, distance2, i as u32),
                AlphaMode::Blend => (1, -distance2, i as u32),
                AlphaMode::Additive => (2, 0.0, i as u32),
            });
        }
        // The index breaks ties, so an unstable sort, which needs no buffer,
        // gives the order a stable one would.
        self.order.sort_unstable_by(|a, b| {
            (a.0.cmp(&b.0))
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
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
