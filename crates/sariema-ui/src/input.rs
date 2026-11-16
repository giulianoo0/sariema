// SPDX-License-Identifier: Apache-2.0

//! Pointers, hit testing, focus, and the widgets they drive.
//!
//! Pointer positions are physical pixels, like the draw list. The topmost
//! interactive node under a pointer when it goes down captures it: every
//! move and the release go to that node, wherever the pointer is, so a
//! joystick and a button can each follow their own finger.

use glam::{Affine2, Vec2};

use crate::anim::Ease;
use crate::node::{NodeId, Prop};
use crate::ui::{Animation, Ui};
use crate::widget::{Widget, stick_curve};

/// What happened, in the order it did. Positions are in the node's own
/// units, from its top-left before its transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Press {
        node: NodeId,
        pointer: u64,
        x: f32,
        y: f32,
    },
    Drag {
        node: NodeId,
        pointer: u64,
        x: f32,
        y: f32,
    },
    Release {
        node: NodeId,
        pointer: u64,
        x: f32,
        y: f32,
    },
    /// Released over the node it pressed, or activated while focused.
    Click {
        node: NodeId,
    },
    /// A joystick's output changed: each axis in [−1, 1], y down.
    Stick {
        node: NodeId,
        x: f32,
        y: f32,
    },
    /// A button released before its ring filled.
    Tap {
        node: NodeId,
    },
    /// A button held until its ring filled.
    Held {
        node: NodeId,
    },
    Focus {
        node: NodeId,
    },
}

pub(crate) struct Pointer {
    pub id: u64,
    pub node: u32,
}

impl Ui {
    /// The transform from a node's units to pixels at interpolation `alpha`.
    pub(crate) fn world(&self, index: u32, alpha: f32) -> Affine2 {
        let n = &self.nodes[index as usize];
        let parent = match n.parent {
            Some(p) => self.world(p, alpha),
            None => Affine2::from_scale(Vec2::splat(self.screen.scale)),
        };
        parent * n.local(alpha).0
    }

    /// The node's box after every transform, in pixels: x, y, width, height
    /// of its axis-aligned bounds.
    pub fn screen_rect(&mut self, id: NodeId) -> Option<[f32; 4]> {
        let i = self.live(id).ok()?;
        if self.layout_dirty {
            self.layout();
        }
        let [_, _, w, h] = self.nodes[i as usize].rect;
        let xf = self.world(i, 1.0);
        Some(bounds(xf, w, h))
    }

    fn hit(&self, index: u32, parent: Affine2, p: Vec2) -> Option<(u32, Vec2)> {
        let n = &self.nodes[index as usize];
        if !n.visible || n.get(Prop::Opacity) <= 0.0 {
            return None;
        }
        let (local, [w, h]) = n.local(1.0);
        let xf = parent * local;
        if xf.matrix2.determinant().abs() < 1e-12 {
            return None;
        }
        let q = xf.inverse().transform_point2(p);
        let inside = q.x >= 0.0 && q.y >= 0.0 && q.x <= w && q.y <= h;
        if n.clip && !inside {
            return None;
        }
        for &child in n.children.iter().rev() {
            if let Some(hit) = self.hit(child, xf, p) {
                return Some(hit);
            }
        }
        (n.interactive && inside).then_some((index, q))
    }

    /// The topmost interactive node under a pixel, and the point in its units.
    pub fn hit_test(&mut self, x: f32, y: f32) -> Option<(NodeId, [f32; 2])> {
        if self.layout_dirty {
            self.layout();
        }
        let root = Affine2::from_scale(Vec2::splat(self.screen.scale));
        self.hit(self.root, root, Vec2::new(x, y))
            .map(|(i, q)| (self.id(i), q.into()))
    }

    fn local_point(&self, index: u32, x: f32, y: f32) -> Vec2 {
        self.world(index, 1.0)
            .inverse()
            .transform_point2(Vec2::new(x, y))
    }

    pub fn pointer_down(&mut self, pointer: u64, x: f32, y: f32) {
        if self.pointers.len() >= self.settings.pointers
            || self.pointers.iter().any(|p| p.id == pointer)
        {
            return;
        }
        let Some((id, [lx, ly])) = self.hit_test(x, y) else {
            return;
        };
        let index = id.index;
        self.pointers.push(crate::input::Pointer {
            id: pointer,
            node: index,
        });
        self.pending.push(Event::Press {
            node: id,
            pointer,
            x: lx,
            y: ly,
        });
        self.widget_down(index, pointer, Vec2::new(lx, ly));
    }

    pub fn pointer_move(&mut self, pointer: u64, x: f32, y: f32) {
        let Some(p) = self.pointers.iter().find(|p| p.id == pointer) else {
            return;
        };
        let index = p.node;
        let q = self.local_point(index, x, y);
        self.pending.push(Event::Drag {
            node: self.id(index),
            pointer,
            x: q.x,
            y: q.y,
        });
        self.widget_move(index, pointer, q);
    }

    pub fn pointer_up(&mut self, pointer: u64, x: f32, y: f32) {
        self.release(pointer, Some((x, y)));
    }

    /// The system took the pointer away: no click, no tap.
    pub fn pointer_cancel(&mut self, pointer: u64) {
        self.release(pointer, None);
    }

    /// Lets go of every pointer, as for a pause or a scene change.
    pub fn cancel_pointers(&mut self) {
        while let Some(p) = self.pointers.last() {
            let id = p.id;
            self.release(id, None);
        }
    }

    fn release(&mut self, pointer: u64, at: Option<(f32, f32)>) {
        let Some(k) = self.pointers.iter().position(|p| p.id == pointer) else {
            return;
        };
        let index = self.pointers.swap_remove(k).node;
        let id = self.id(index);
        if let Some((x, y)) = at {
            let q = self.local_point(index, x, y);
            self.pending.push(Event::Release {
                node: id,
                pointer,
                x: q.x,
                y: q.y,
            });
            if self.hit_test(x, y).is_some_and(|(hit, _)| hit == id) {
                self.pending.push(Event::Click { node: id });
            }
        }
        self.widget_up(index, pointer, at.is_some());
    }

    fn widget_down(&mut self, index: u32, pointer: u64, q: Vec2) {
        match &mut self.nodes[index as usize].widget {
            Some(Widget::Joystick(j)) if j.pointer.is_none() => {
                j.pointer = Some(pointer);
                j.center = q.into();
                j.output = [0.0; 2];
                let (base, pin) = (j.base, j.pin);
                let d = j.desc.clone();
                let rest = self.rest_center(base, index);
                self.set_index(base, Prop::X, q.x - rest.x);
                self.set_index(base, Prop::Y, q.y - rest.y);
                self.set_index(pin, Prop::X, 0.0);
                self.set_index(pin, Prop::Y, 0.0);
                let appear = |to| Animation::tween(to, d.appear_s, Ease::OUT);
                self.anim(base, Prop::Opacity, appear(d.held_opacity));
                self.anim(base, Prop::ScaleX, appear(1.0).from(d.appear_scale));
                self.anim(base, Prop::ScaleY, appear(1.0).from(d.appear_scale));
            }
            Some(Widget::Button(b)) if b.pointer.is_none() => {
                b.pointer = Some(pointer);
                b.held = false;
                let (face, rings, d) = (b.face, b.rings.clone(), b.desc.clone());
                let press = Animation::tween(d.press_scale, d.press_s, Ease::OUT);
                self.anim(face, Prop::ScaleX, press);
                self.anim(face, Prop::ScaleY, press);
                for ring in rings {
                    let left = 1.0
                        - self.nodes[ring as usize]
                            .get(Prop::Progress)
                            .clamp(0.0, 1.0);
                    self.anim(
                        ring,
                        Prop::Progress,
                        Animation::tween(1.0, d.hold_s * left, Ease::LINEAR),
                    );
                }
            }
            _ => {}
        }
    }

    fn widget_move(&mut self, index: u32, pointer: u64, q: Vec2) {
        let id = self.id(index);
        let Some(Widget::Joystick(j)) = &mut self.nodes[index as usize].widget else {
            return;
        };
        if j.pointer != Some(pointer) {
            return;
        }
        let mut d = q - Vec2::from(j.center);
        let radius = j.desc.radius;
        if d.length() > radius {
            d = d.normalize() * radius;
        }
        let out = stick_curve(d.into(), radius, j.desc.dead_zone, j.desc.saturation);
        let changed = out != j.output;
        j.output = out;
        let pin = j.pin;
        self.set_index(pin, Prop::X, d.x);
        self.set_index(pin, Prop::Y, d.y);
        if changed {
            self.pending.push(Event::Stick {
                node: id,
                x: out[0],
                y: out[1],
            });
        }
    }

    fn widget_up(&mut self, index: u32, pointer: u64, completed: bool) {
        let id = self.id(index);
        match &mut self.nodes[index as usize].widget {
            Some(Widget::Joystick(j)) if j.pointer == Some(pointer) => {
                j.pointer = None;
                let was = std::mem::take(&mut j.output);
                let (base, pin, d) = (j.base, j.pin, j.desc.clone());
                let pin_back = Animation::spring(0.0, d.pin_return.response, d.pin_return.damping);
                let base_back =
                    Animation::spring(0.0, d.base_return.response, d.base_return.damping);
                self.anim(pin, Prop::X, pin_back);
                self.anim(pin, Prop::Y, pin_back);
                self.anim(base, Prop::X, base_back);
                self.anim(base, Prop::Y, base_back);
                self.anim(
                    base,
                    Prop::Opacity,
                    Animation::tween(d.rest_opacity, d.appear_s, Ease::OUT),
                );
                if was != [0.0; 2] {
                    self.pending.push(Event::Stick {
                        node: id,
                        x: 0.0,
                        y: 0.0,
                    });
                }
            }
            Some(Widget::Button(b)) if b.pointer == Some(pointer) => {
                b.pointer = None;
                let (face, rings, held, d) = (b.face, b.rings.clone(), b.held, b.desc.clone());
                if !held {
                    if completed {
                        self.pending.push(Event::Tap { node: id });
                    }
                    for ring in rings {
                        let p = self.nodes[ring as usize]
                            .get(Prop::Progress)
                            .clamp(0.0, 1.0);
                        self.anim(
                            ring,
                            Prop::Progress,
                            Animation::tween(0.0, d.empty_s * p, Ease::OUT),
                        );
                    }
                }
                let back = Animation::tween(1.0, d.press_s, Ease::OUT);
                self.anim(face, Prop::ScaleX, back);
                self.anim(face, Prop::ScaleY, back);
            }
            _ => {}
        }
    }

    /// Fires holds whose ring has filled; runs once per step.
    pub(crate) fn step_widgets(&mut self) {
        for k in 0..self.widgets.len() {
            let index = self.widgets[k];
            let Some(Widget::Button(b)) = &self.nodes[index as usize].widget else {
                continue;
            };
            let (Some(_), false, Some(&ring)) = (b.pointer, b.held, b.rings.first()) else {
                continue;
            };
            if self.nodes[ring as usize].get(Prop::Progress) < 1.0 {
                continue;
            }
            let (face, press_s) = (b.face, b.desc.press_s);
            if let Some(Widget::Button(b)) = &mut self.nodes[index as usize].widget {
                b.held = true;
            }
            let id = self.id(index);
            self.events.push(Event::Held { node: id });
            let back = Animation::tween(1.0, press_s, Ease::OUT);
            self.anim(face, Prop::ScaleX, back);
            self.anim(face, Prop::ScaleY, back);
        }
    }

    /// Widget animations; past the animation limit the property is set.
    fn anim(&mut self, index: u32, prop: Prop, a: Animation) {
        if self.animate(self.id(index), prop, a).is_err() {
            self.set_index(index, prop, a.to);
        }
    }

    /// Centre of a node's layout box in the units of its ancestor `zone`.
    fn rest_center(&self, index: u32, zone: u32) -> Vec2 {
        let [_, _, w, h] = self.nodes[index as usize].rect;
        let mut at = Vec2::new(w, h) * 0.5;
        let mut i = index;
        while i != zone {
            let n = &self.nodes[i as usize];
            at += Vec2::new(n.rect[0], n.rect[1]);
            match n.parent {
                Some(p) => i = p,
                None => break,
            }
        }
        at
    }

    /// A joystick's output, each axis in [−1, 1], y down.
    pub fn stick(&self, id: NodeId) -> Option<[f32; 2]> {
        match &self.nodes[self.live(id).ok()? as usize].widget {
            Some(Widget::Joystick(j)) => Some(j.output),
            _ => None,
        }
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focus.map(|i| self.id(i))
    }

    /// Moves focus to the next visible node in focus order, wrapping.
    pub fn focus_next(&mut self) {
        self.move_focus(true);
    }

    pub fn focus_prev(&mut self) {
        self.move_focus(false);
    }

    fn move_focus(&mut self, forward: bool) {
        let key = |i: u32| self.nodes[i as usize].focus.map(|f| (f, i));
        let current = self.focus.and_then(key);
        let mut best: Option<(u32, u32)> = None;
        let mut wrap: Option<(u32, u32)> = None;
        for (i, n) in self.nodes.iter().enumerate() {
            let Some(f) = n.focus.filter(|_| n.live && n.visible) else {
                continue;
            };
            let k = (f, i as u32);
            let ahead = current.is_none_or(|c| if forward { k > c } else { k < c });
            let better =
                |b: Option<(u32, u32)>| b.is_none_or(|b| if forward { k < b } else { k > b });
            if ahead && better(best) {
                best = Some(k);
            }
            if better(wrap) {
                wrap = Some(k);
            }
        }
        if let Some((_, i)) = best.or(wrap) {
            self.focus = Some(i);
            self.pending.push(Event::Focus { node: self.id(i) });
        }
    }

    /// Clicks the focused node.
    pub fn activate(&mut self) {
        if let Some(i) = self.focus {
            self.pending.push(Event::Click { node: self.id(i) });
        }
    }
}

/// Axis-aligned bounds of a `w × h` box under `xf`: x, y, width, height.
pub(crate) fn bounds(xf: Affine2, w: f32, h: f32) -> [f32; 4] {
    let corners = [
        Vec2::ZERO,
        Vec2::new(w, 0.0),
        Vec2::new(0.0, h),
        Vec2::new(w, h),
    ]
    .map(|c| xf.transform_point2(c));
    let lo = corners.iter().fold(Vec2::INFINITY, |a, &c| a.min(c));
    let hi = corners.iter().fold(Vec2::NEG_INFINITY, |a, &c| a.max(c));
    [lo.x, lo.y, hi.x - lo.x, hi.y - lo.y]
}
