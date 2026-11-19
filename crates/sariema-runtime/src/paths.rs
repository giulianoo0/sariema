// SPDX-License-Identifier: Apache-2.0

//! Root motion bound to a clip's time (`clip_path`): an actor's root moves
//! only within spans of the clip it plays, placed each step from the clip's
//! time, so a slowed span, slow motion or a held first frame keep body and
//! root together. The runtime moves the root; a collision module poses the
//! actor there with its collider off and keeps it posed, between spans too,
//! until the last span ends and gets it back on, pushed out of any overlap;
//! a path that holds its end leaves it posed there, held.

use glam::Vec3;
use sariema_format::script::{BlockReason, ClipSpan, Ease, Entity, Event};

use crate::ease::ease;
use crate::events::Happening;
use crate::outbox::PhysicsIntent;
use crate::state::State;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    from: f32,
    until: f32,
    to: Option<Vec3>,
    yaw: Option<f32>,
    ease: Ease,
    arc: f32,
}

#[derive(Clone, Debug)]
struct Path {
    entity: Entity,
    clip: u32,
    spans: Vec<Span>,
    /// The span walked now or next.
    at: usize,
    /// Where the root stood when span `at` began, once it has.
    start: Option<(Vec3, f32)>,
    /// Held where the last span leaves the root instead of landing.
    hold_end: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Paths {
    list: Vec<Path>,
    /// Ended paths, kept for their buffers.
    spare: Vec<Path>,
}

impl Paths {
    /// `entity` moves along `spans` of `clip` from now, replacing any path
    /// it had; held at its end with `hold_end`.
    pub fn start(
        &mut self,
        entity: Entity,
        clip: u32,
        spans: impl Iterator<Item = ClipSpan>,
        hold_end: bool,
    ) {
        self.forget(entity);
        let mut path = self.spare.pop().unwrap_or(Path {
            entity,
            clip,
            spans: Vec::new(),
            at: 0,
            start: None,
            hold_end,
        });
        path.entity = entity;
        path.clip = clip;
        path.at = 0;
        path.start = None;
        path.hold_end = hold_end;
        path.spans.clear();
        path.spans.extend(spans.map(|s| Span {
            from: s.from,
            until: s.until,
            to: s.to.map(Vec3::from_array),
            yaw: s.yaw,
            ease: s.ease,
            arc: s.arc,
        }));
        self.list.push(path);
    }

    /// Drops `entity`'s path where its root stands; whether it had one.
    pub fn forget(&mut self, entity: Entity) -> bool {
        match self.list.iter().position(|p| p.entity == entity) {
            Some(k) => {
                self.spare.push(self.list.swap_remove(k));
                true
            }
            None => false,
        }
    }

    pub fn moving(&self, entity: Entity) -> bool {
        self.list.iter().any(|p| p.entity == entity)
    }

    fn holds_end(&self, entity: Entity) -> bool {
        self.list.iter().any(|p| p.entity == entity && p.hold_end)
    }

    /// Where `entity`'s path ends from `at` facing `yaw`.
    pub fn end(&self, entity: Entity, at: Vec3, yaw: f32) -> Option<(Vec3, f32)> {
        let path = self.list.iter().find(|p| p.entity == entity)?;
        let to = (path.spans.iter().rev()).find_map(|s| s.to).unwrap_or(at);
        let yaw = (path.spans.iter().rev()).find_map(|s| s.yaw).unwrap_or(yaw);
        Some((to, yaw))
    }
}

/// The root `u` of the way through `span`, from `start`.
fn along(span: &Span, start: (Vec3, f32), u: f32) -> (Vec3, f32) {
    let e = ease(span.ease, u);
    let position = match span.to {
        Some(to) => start.0 + (to - start.0) * e + Vec3::Y * (4.0 * span.arc * u * (1.0 - u)),
        None => start.0,
    };
    let yaw = match span.yaw {
        Some(yaw) => start.1 + ((yaw - start.1 + 180.0).rem_euclid(360.0) - 180.0) * e,
        None => start.1,
    };
    (position, yaw)
}

/// The farthest a path moves its root in one step of world time, starting
/// from `start`, each span lasting `(until − from) / rate` seconds;
/// infinite when it turns more than `turn` degrees in a step.
pub(crate) fn fastest(
    start: (Vec3, f32),
    spans: impl Iterator<Item = ClipSpan>,
    step: f32,
    turn: f32,
) -> f32 {
    let mut at = start;
    let mut most = 0.0f32;
    for s in spans {
        let span = Span {
            from: s.from,
            until: s.until,
            to: s.to.map(Vec3::from_array),
            yaw: s.yaw,
            ease: s.ease,
            arc: s.arc,
        };
        let steps = ((s.until - s.from) / s.rate / step).ceil().max(1.0) as u32;
        let mut last = at;
        for k in 1..=steps {
            let now = along(&span, at, k as f32 / steps as f32);
            if (now.1 - last.1).abs() > turn {
                return f32::INFINITY;
            }
            most = most.max(now.0.distance(last.0));
            last = now;
        }
        at = along(&span, at, 1.0);
    }
    most
}

/// Places every path's root for the clip time its actor is at. A path
/// whose actor plays another clip, or is gone, stops where it is and the
/// game hears `blocked`; one past its last span ends with `arrived`.
pub fn step(s: &mut State) {
    let mut k = 0;
    while k < s.paths.list.len() {
        let path = &mut s.paths.list[k];
        let entity = path.entity;
        let playing = s.clips.time(entity).filter(|&(c, _)| c == path.clip);
        let (Some((_, t)), Some(node)) = (playing, s.world.node(entity)) else {
            s.paths.forget(entity);
            end(s, entity, false);
            s.events.push(Event::Blocked {
                entity,
                reason: BlockReason::Stopped,
            });
            s.happen(Happening::Blocked(entity));
            continue;
        };
        let mut root = (node.local.position, node.local.yaw);
        let moved = root;
        while let Some(span) = path.spans.get(path.at) {
            if t < span.from {
                break;
            }
            let start = *path.start.get_or_insert(root);
            let u = ((t - span.from) / (span.until - span.from)).clamp(0.0, 1.0);
            root = along(span, start, u);
            if t < span.until {
                break;
            }
            path.at += 1;
            path.start = None;
        }
        let ended = path.at == path.spans.len();
        let hold_end = path.hold_end;
        if root != moved
            && let Ok(node) = s.world.node_mut(entity)
        {
            (node.local.position, node.local.yaw) = root;
        }
        if ended {
            s.paths.forget(entity);
            end(s, entity, hold_end);
            s.events.push(Event::Arrived { entity });
            s.happen(Happening::Arrived(entity));
        } else {
            k += 1;
        }
    }
}

/// Lands `entity`'s path at its end, as if walked.
pub fn finish(s: &mut State, entity: Entity) {
    let Some(node) = s.world.node(entity) else {
        return;
    };
    let (at, yaw) = (node.local.position, node.local.yaw);
    let Some(last) = s.paths.end(entity, at, yaw) else {
        return;
    };
    let hold_end = s.paths.holds_end(entity);
    s.paths.forget(entity);
    if let Ok(node) = s.world.node_mut(entity) {
        (node.local.position, node.local.yaw) = last;
    }
    end(s, entity, hold_end);
}

/// The collider on again where the root stands, or the actor held there.
fn end(s: &mut State, entity: Entity, hold_end: bool) {
    if hold_end {
        s.motion.mover_mut(entity).held = true;
    }
    if let Some(node) = s.world.node(entity) {
        let (position, yaw) = (node.local.position, node.local.yaw);
        s.outbox.physics.push(if hold_end {
            PhysicsIntent::Hold {
                entity,
                position,
                yaw,
            }
        } else {
            PhysicsIntent::Teleport {
                entity,
                position,
                yaw,
            }
        });
    }
}
