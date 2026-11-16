// SPDX-License-Identifier: Apache-2.0

//! Recording the scene's draws: the background, then the room's visible
//! draws in order, or their depth alone.

use glam::Vec3;
use sariema_core::Camera;

use super::{Kind, Scene, Targets};
use crate::resources::Room;

impl Scene {
    /// `keep_depth` stores the depth for post-processing; otherwise it is
    /// discarded, which spares tile-based GPUs writing it out.
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        room: Option<&Room>,
        camera: &Camera,
        keep_depth: bool,
    ) {
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
        let order = draw_order(room, camera.position);
        if order.is_empty() {
            return;
        }
        room.bind_meshes(&mut pass);
        let mut current = None;
        for i in order {
            let draw = &room.draws[i];
            let pipeline = (draw.double_sided, draw.kind);
            if current != Some(pipeline) {
                pass.set_pipeline(&self.pipelines[draw.double_sided as usize][draw.kind as usize]);
                current = Some(pipeline);
            }
            let material = &room.materials[draw.material as usize];
            if matches!(draw.kind, Kind::LitBlend | Kind::UnlitBlend) {
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
        camera: &Camera,
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
        let order: Vec<usize> = draw_order(room, camera.position)
            .into_iter()
            .filter(|&i| room.draws[i].kind != Kind::Additive)
            .collect();
        if order.is_empty() {
            return;
        }
        pass.set_pipeline(&self.depth_only);
        pass.set_bind_group(0, &self.frame_group, &[]);
        room.bind_meshes(&mut pass);
        for i in order {
            room.draw(&mut pass, i);
        }
    }
}

/// Visible draws: opaque front to back, then blended back to front, then additive.
fn draw_order(room: &Room, eye: Vec3) -> Vec<usize> {
    let mut order: Vec<(u8, f32, usize)> = room
        .draws
        .iter()
        .enumerate()
        .filter(|(_, d)| d.visible && d.index_count > 0)
        .map(|(i, d)| {
            let distance2 = eye.distance_squared(d.center());
            match d.kind {
                Kind::Additive => (2, 0.0, i),
                kind if kind.is_opaque() => (0, distance2, i),
                _ => (1, -distance2, i),
            }
        })
        .collect();
    order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    order.into_iter().map(|(_, _, i)| i).collect()
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
