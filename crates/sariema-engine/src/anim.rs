// SPDX-License-Identifier: Apache-2.0

//! The animation stage of a step: each actor drawn with an actor pack gets
//! an animator whose clip follows the runtime's clip clock; its markers and
//! ends go back to the game, and the bones and sockets the camera aims at
//! are resolved for it.

use glam::{Mat4, Vec3};
use sariema_anim::{
    AnimSettings, Animator, ClipId, EventKind, JointId, Play, PlayMode, Rig, SocketId,
};
use sariema_format::actor::ActorPack;
use sariema_format::script::{Anchor, ClipMode, Entity, Event, Kind, name_id};
use sariema_runtime::events::Happening;
use sariema_runtime::state::State;

/// An actor pack, read once however many entities show it.
pub struct Model {
    pub name: String,
    pub asset: u32,
    pub rig: Rig,
    clips: Vec<u32>,
    markers: Vec<Vec<u32>>,
    sockets: Vec<(u32, SocketId)>,
    joints: Vec<(u32, JointId)>,
}

impl Model {
    pub fn new(name: &str, pack: &ActorPack) -> Result<Model, String> {
        let rig = Rig::new(pack)?;
        let clips = (0..rig.clips())
            .map(|c| name_id(&rig.clip_info(ClipId(c as u32)).name))
            .collect();
        let markers = (0..rig.clips())
            .map(|c| {
                let info = rig.clip_info(ClipId(c as u32));
                info.markers.iter().map(|(m, _)| name_id(m)).collect()
            })
            .collect();
        let sockets = (0..rig.sockets())
            .map(|s| {
                (
                    name_id(rig.socket_name(SocketId(s as u32))),
                    SocketId(s as u32),
                )
            })
            .collect();
        let joints = (0..rig.joints())
            .map(|j| {
                (
                    name_id(rig.joint_name(JointId(j as u32))),
                    JointId(j as u32),
                )
            })
            .collect();
        Ok(Model {
            name: name.to_owned(),
            asset: name_id(name),
            rig,
            clips,
            markers,
            sockets,
            joints,
        })
    }

    /// Each clip's name id, by clip index.
    pub fn clip_ids(&self) -> &[u32] {
        &self.clips
    }

    fn clip(&self, id: u32) -> Option<ClipId> {
        self.clips
            .iter()
            .position(|&c| c == id)
            .map(|c| ClipId(c as u32))
    }
}

struct Animated {
    entity: Entity,
    model: usize,
    animator: Animator,
    /// The runtime clip it plays.
    clip: Option<u32>,
    /// Whether its pose has been evaluated at the end of this step.
    evaluated: bool,
}

pub struct Animation {
    pub models: Vec<Model>,
    settings: AnimSettings,
    /// By entity slot.
    actors: Vec<Option<Animated>>,
    anchors: Vec<(Entity, Anchor, u32)>,
}

impl Animation {
    pub fn new(models: Vec<Model>, settings: AnimSettings) -> Animation {
        Animation {
            models,
            settings,
            actors: Vec::new(),
            anchors: Vec::new(),
        }
    }

    pub fn step(&mut self, s: &mut State) {
        self.track(s);
        for request in &s.outbox.clips {
            let Some(a) = self
                .actors
                .get_mut(request.entity.index as usize)
                .and_then(Option::as_mut)
                .filter(|a| a.entity == request.entity)
            else {
                continue;
            };
            let model = &self.models[a.model];
            let Some(clip) = model.clip(request.clip) else {
                continue;
            };
            let mode = match request.mode {
                ClipMode::Loop => PlayMode::Loop,
                ClipMode::Once => PlayMode::Once,
                ClipMode::Hold => PlayMode::HoldLast,
            };
            let play = Play {
                mode: Some(mode),
                speed: request.speed,
                fade_s: Some(request.blend.max(0.0)),
                start_s: request.from,
                restart: true,
                ..Play::default()
            };
            a.animator.play(&model.rig, 0, clip, play);
            a.clip = Some(request.clip);
        }
        for slot in &mut self.actors {
            let Some(a) = slot else { continue };
            let model = &self.models[a.model];
            if let (Some(clip), Some((now_clip, now))) = (a.clip, s.clip_time(a.entity))
                && clip == now_clip
            {
                a.animator.seek(0, f64::from(now));
            }
            a.animator.step(&model.rig);
            a.evaluated = false;
            for event in a.animator.events() {
                let clip = model.clips[event.clip.0 as usize];
                match event.kind {
                    EventKind::Marker(i) => {
                        let marker = model.markers[event.clip.0 as usize][i as usize];
                        s.events.push(Event::ClipMarker {
                            entity: a.entity,
                            clip,
                            marker,
                        });
                        s.happen(Happening::ClipMarker {
                            entity: a.entity,
                            clip,
                            marker,
                        });
                    }
                    EventKind::Finished => {
                        s.events.push(Event::ClipFinished {
                            entity: a.entity,
                            clip,
                        });
                        s.happen(Happening::ClipFinished(a.entity));
                    }
                    EventKind::Looped => {}
                }
            }
        }
        self.anchors.clear();
        self.anchors.extend(s.camera.anchors());
        for k in 0..self.anchors.len() {
            let (entity, anchor, name) = self.anchors[k];
            if let Some(at) = self.point(entity, anchor, name) {
                s.camera.set_anchor_point(entity, at);
            }
        }
    }

    /// Keeps one animator per live actor drawn with a model, and none for
    /// anything else.
    fn track(&mut self, s: &State) {
        for slot in &mut self.actors {
            if slot.as_ref().is_some_and(|a| !s.world.is_alive(a.entity)) {
                *slot = None;
            }
        }
        for (entity, node) in s.world.entities() {
            if node.kind != Kind::Actor {
                continue;
            }
            let i = entity.index as usize;
            if self
                .actors
                .get(i)
                .is_some_and(|a| a.as_ref().is_some_and(|a| a.entity == entity))
            {
                continue;
            }
            let Some(model) = self.models.iter().position(|m| m.asset == node.asset) else {
                continue;
            };
            if self.actors.len() <= i {
                self.actors.resize_with(i + 1, || None);
            }
            self.actors[i] = Some(Animated {
                entity,
                model,
                animator: Animator::new(&self.models[model].rig, self.settings.clone()),
                clip: None,
                evaluated: false,
            });
        }
    }

    /// Where `e`'s bone or socket `name` is at the end of this step, in its
    /// own frame; a socket is looked up first when `anchor` says either.
    pub fn point(&mut self, e: Entity, anchor: Anchor, name: u32) -> Option<Vec3> {
        let a = self.actors.get_mut(e.index as usize)?.as_mut()?;
        if a.entity != e {
            return None;
        }
        let model = &self.models[a.model];
        if !a.evaluated {
            a.animator.evaluate(&model.rig, 1.0);
            a.evaluated = true;
        }
        let socket = model.sockets.iter().find(|(n, _)| *n == name);
        let joint = model.joints.iter().find(|(n, _)| *n == name);
        let matrix: Mat4 = match (anchor, socket, joint) {
            (Anchor::Socket, Some(&(_, s)), _) => a.animator.socket(&model.rig, s),
            (Anchor::Bone, _, Some(&(_, j))) => a.animator.joint_model(j),
            (Anchor::Origin, Some(&(_, s)), _) => a.animator.socket(&model.rig, s),
            (Anchor::Origin, None, Some(&(_, j))) => a.animator.joint_model(j),
            _ => return None,
        };
        Some(matrix.w_axis.truncate())
    }

    /// Each animated actor, the index of its model and its animator.
    pub fn actors(&self) -> impl Iterator<Item = (Entity, usize, &Animator)> + '_ {
        self.actors
            .iter()
            .flatten()
            .map(|a| (a.entity, a.model, &a.animator))
    }

    /// Poses every animator at `alpha` between the last two steps.
    pub fn evaluate(&mut self, alpha: f32) {
        for a in self.actors.iter_mut().flatten() {
            a.animator.evaluate(&self.models[a.model].rig, alpha);
            a.evaluated = false;
        }
    }

    /// Each animated actor's clip and the time its animator stands at, for
    /// the state hash.
    pub fn clip_times(&self) -> impl Iterator<Item = (Entity, Option<ClipId>, f32)> + '_ {
        self.actors
            .iter()
            .flatten()
            .map(|a| (a.entity, a.animator.clip(0), a.animator.clip_time(0)))
    }
}
