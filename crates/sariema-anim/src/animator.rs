// SPDX-License-Identifier: Apache-2.0

//! One actor's playback: layers of clips stepped at a fixed rate, blended
//! into a local pose, and interpolated between the last two steps for
//! rendering.

use glam::{Affine3A, Mat4};

use crate::mask::JointMask;
use crate::pose::{Transform, apply_delta, delta};
use crate::rig::{ClipId, FaceId, JointId, Rig, SocketId};

#[derive(Clone, Debug, PartialEq)]
pub struct AnimSettings {
    /// Seconds of one simulation step.
    pub step_s: f32,
    /// Crossfade length when a play gives none: the game's 0.13 s.
    pub fade_s: f32,
    pub easing: Easing,
    /// Layer 0 is the base; the others blend over it in order.
    pub layers: usize,
    /// An event within this many seconds of the end of a step belongs to
    /// that step, so instants on step boundaries do not depend on rounding.
    pub event_tolerance_s: f64,
}

impl Default for AnimSettings {
    fn default() -> AnimSettings {
        AnimSettings {
            step_s: 1.0 / 60.0,
            fade_s: 0.13,
            easing: Easing::Smooth,
            layers: 2,
            event_tolerance_s: 1e-5,
        }
    }
}

/// How a fade's weight goes from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Linear,
    /// Smoothstep: no jump in speed where the fade starts or ends.
    Smooth,
}

impl Easing {
    pub fn apply(self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match self {
            Easing::Linear => x,
            Easing::Smooth => x * x * (3.0 - 2.0 * x),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    Loop,
    /// Plays to the end, then the layer fades out over the play's fade.
    Once,
    /// Plays to the end and stays on the last frame.
    HoldLast,
}

impl PlayMode {
    /// Where in a clip of `duration` seconds playback is after `time`
    /// seconds: wrapped both ways for [`PlayMode::Loop`], clamped otherwise.
    pub fn clip_time(self, time: f64, duration: f32) -> f64 {
        let duration = f64::from(duration);
        match self {
            PlayMode::Loop if duration > 0.0 => time.rem_euclid(duration),
            _ => time.clamp(0.0, duration),
        }
    }
}

/// How to start a clip; [`Play::default`] gives the clip's own loop flag,
/// speed 1, the start of the clip and the settings' fade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Play {
    /// `None` loops a clip whose pack marks it looping and holds the last
    /// frame of any other.
    pub mode: Option<PlayMode>,
    /// At least 0; a negative speed counts as 0.
    pub speed: f32,
    /// Seconds into the clip to start from.
    pub start_s: f32,
    pub fade_s: Option<f32>,
    pub easing: Option<Easing>,
    /// Starts the clip over even when the layer already plays it.
    pub restart: bool,
}

impl Default for Play {
    fn default() -> Play {
        Play {
            mode: None,
            speed: 1.0,
            start_s: 0.0,
            fade_s: None,
            easing: None,
            restart: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerBlend {
    /// Replaces the pose below, by the layer's weight and mask.
    Override,
    /// Adds the clip's motion relative to its first frame.
    Additive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// A [`PlayMode::Once`] or [`PlayMode::HoldLast`] clip reached its end.
    Finished,
    /// A looping clip went past its end and started over.
    Looped,
    /// Playback reached the marker with this index in [`crate::ClipInfo::markers`].
    Marker(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub layer: usize,
    pub clip: ClipId,
    pub kind: EventKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipState {
    clip: ClipId,
    time: f64,
    speed: f32,
    mode: PlayMode,
    /// No step has advanced it yet, so an event at its start time is due.
    fresh: bool,
    /// Where the next step takes it, instead of its speed.
    seek: Option<f64>,
    done: bool,
    fade_s: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    /// The pose of the layers below; the rest pose under layer 0.
    Below,
    Clip(ClipState),
    /// The layer's own pose when a play interrupted its fade.
    Frozen,
}

struct Layer {
    blend: LayerBlend,
    mask: Option<Vec<f32>>,
    weight: f32,
    from: Slot,
    to: Slot,
    fade_elapsed: f32,
    fade_s: f32,
    easing: Easing,
    /// Its pose at the last step, before mask and weight.
    out: Vec<Transform>,
    frozen: Vec<Transform>,
}

impl Layer {
    fn fading(&self) -> bool {
        self.fade_elapsed < self.fade_s
    }

    fn idle(&self) -> bool {
        self.to == Slot::Below && !self.fading()
    }
}

/// Steps and blends one actor's clips. Every buffer is sized by
/// [`Animator::new`]; nothing after it allocates, except the event list
/// growing past the most events any step produced before.
pub struct Animator {
    settings: AnimSettings,
    layers: Vec<Layer>,
    prev: Vec<Transform>,
    curr: Vec<Transform>,
    from_pose: Vec<Transform>,
    to_pose: Vec<Transform>,
    pose: Vec<Transform>,
    model: Vec<Affine3A>,
    palette: Vec<Mat4>,
    faces: Vec<u32>,
    face_set: Vec<u32>,
    events: Vec<Event>,
}

impl Animator {
    /// An animator in `rig`'s rest pose. A setting of 0 layers gives one.
    pub fn new(rig: &Rig, settings: AnimSettings) -> Animator {
        let joints = rig.joints();
        let rest = rig.rest_pose().to_vec();
        let layers = (0..settings.layers.max(1))
            .map(|_| Layer {
                blend: LayerBlend::Override,
                mask: None,
                weight: 1.0,
                from: Slot::Below,
                to: Slot::Below,
                fade_elapsed: 0.0,
                fade_s: 0.0,
                easing: settings.easing,
                out: rest.clone(),
                frozen: rest.clone(),
            })
            .collect::<Vec<_>>();
        let markers = (0..rig.clips())
            .map(|c| rig.clip_info(ClipId(c as u32)).markers.len())
            .max()
            .unwrap_or(0);
        let face_set: Vec<u32> = (0..rig.faces()).map(|f| rig.face_start(f)).collect();
        let mut animator = Animator {
            events: Vec::with_capacity(layers.len() * (markers + 2) * 2),
            layers,
            prev: rest.clone(),
            curr: rest.clone(),
            from_pose: rest.clone(),
            to_pose: rest.clone(),
            pose: rest,
            model: vec![Affine3A::IDENTITY; joints],
            palette: vec![Mat4::IDENTITY; joints],
            faces: face_set.clone(),
            face_set,
            settings,
        };
        animator.evaluate(rig, 1.0);
        animator
    }

    pub fn settings(&self) -> &AnimSettings {
        &self.settings
    }

    /// How `layer` blends, which joints it drives (all without a mask) and
    /// how much; meant for setup, since it copies the mask.
    pub fn set_layer(&mut self, layer: usize, blend: LayerBlend, mask: Option<&JointMask>) {
        let layer = &mut self.layers[layer];
        layer.blend = blend;
        layer.mask = mask.map(|m| m.weights().to_vec());
    }

    /// Clamped to 0..1.
    pub fn set_layer_weight(&mut self, layer: usize, weight: f32) {
        self.layers[layer].weight = weight.clamp(0.0, 1.0);
    }

    /// Fades `layer` from its current pose to `clip`. Playing the clip the
    /// layer already plays, without `restart`, changes nothing.
    pub fn play(&mut self, rig: &Rig, layer: usize, clip: ClipId, play: Play) {
        let info = rig.clip_info(clip);
        let mode = play.mode.unwrap_or(if info.looping {
            PlayMode::Loop
        } else {
            PlayMode::HoldLast
        });
        let fade_s = play.fade_s.unwrap_or(self.settings.fade_s).max(0.0);
        let layer = &mut self.layers[layer];
        if let Slot::Clip(current) = layer.to
            && current.clip == clip
            && !play.restart
        {
            return;
        }
        let state = ClipState {
            clip,
            time: mode.clip_time(f64::from(play.start_s), info.duration),
            speed: play.speed.max(0.0),
            mode,
            fresh: true,
            seek: None,
            done: false,
            fade_s,
        };
        layer.from = if layer.fading() {
            layer.frozen.copy_from_slice(&layer.out);
            Slot::Frozen
        } else {
            layer.to
        };
        layer.to = Slot::Clip(state);
        layer.fade_elapsed = 0.0;
        layer.fade_s = fade_s;
        layer.easing = play.easing.unwrap_or(self.settings.easing);
    }

    /// Fades `layer` out to the layers below, over `fade_s` or the settings' fade.
    pub fn stop(&mut self, layer: usize, fade_s: Option<f32>) {
        let fade_s = fade_s.unwrap_or(self.settings.fade_s).max(0.0);
        let layer = &mut self.layers[layer];
        if layer.to == Slot::Below {
            return;
        }
        if layer.fading() {
            layer.frozen.copy_from_slice(&layer.out);
            layer.from = Slot::Frozen;
        } else {
            layer.from = layer.to;
        }
        layer.to = Slot::Below;
        layer.fade_elapsed = 0.0;
        layer.fade_s = fade_s;
    }

    pub fn set_speed(&mut self, layer: usize, speed: f32) {
        if let Slot::Clip(state) = &mut self.layers[layer].to {
            state.speed = speed.max(0.0);
        }
    }

    /// The next step takes the clip `layer` plays to `time` seconds instead
    /// of advancing it by its speed, with the markers on the way. A looping
    /// clip goes forward to `time` wrapped into it, through its end at most
    /// once; any other clip goes to `time` clamped.
    pub fn seek(&mut self, layer: usize, time: f64) {
        if let Slot::Clip(state) = &mut self.layers[layer].to {
            state.seek = Some(time);
        }
    }

    /// The clip `layer` plays or fades to, if any.
    pub fn clip(&self, layer: usize) -> Option<ClipId> {
        match self.layers[layer].to {
            Slot::Clip(state) => Some(state.clip),
            _ => None,
        }
    }

    /// Seconds into the clip `layer` plays; 0 without one.
    pub fn clip_time(&self, layer: usize) -> f32 {
        match self.layers[layer].to {
            Slot::Clip(state) => state.time as f32,
            _ => 0.0,
        }
    }

    pub fn is_fading(&self, layer: usize) -> bool {
        self.layers[layer].fading()
    }

    /// Shows `cell` on `face` whenever no playing clip has a track for it.
    pub fn set_face(&mut self, face: FaceId, cell: u32) {
        self.face_set[face.0 as usize] = cell;
    }

    /// Advances every layer by one step and blends the new pose. The events
    /// of the step replace those of the one before.
    pub fn step(&mut self, rig: &Rig) {
        self.events.clear();
        std::mem::swap(&mut self.prev, &mut self.curr);
        let dt = self.settings.step_s;
        let tolerance = self.settings.event_tolerance_s;
        self.curr.copy_from_slice(rig.rest_pose());
        for (index, layer) in self.layers.iter_mut().enumerate() {
            if let Slot::Clip(state) = &mut layer.from {
                advance(rig, state, dt, tolerance, None);
            }
            if let Slot::Clip(state) = &mut layer.to {
                let finished = advance(rig, state, dt, tolerance, Some((index, &mut self.events)));
                if finished && state.mode == PlayMode::Once {
                    let fade_s = state.fade_s;
                    layer.from = layer.to;
                    layer.to = Slot::Below;
                    layer.fade_elapsed = 0.0;
                    layer.fade_s = fade_s;
                }
            }
            layer.fade_elapsed = (layer.fade_elapsed + dt).min(layer.fade_s);
            if !layer.fading() {
                layer.from = Slot::Below;
            }
            blend_layer(
                rig,
                layer,
                &self.curr,
                &mut self.from_pose,
                &mut self.to_pose,
            );
            apply_layer(layer, &mut self.curr);
        }
        for face in 0..self.faces.len() {
            let from_clip = self.layers.iter().rev().find_map(|layer| match layer.to {
                Slot::Clip(state) => rig.face_cell(state.clip, face, state.time as f32),
                _ => None,
            });
            self.faces[face] = from_clip.unwrap_or(self.face_set[face]);
        }
    }

    /// The events of the last step, by layer and then in the order they happened.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Blends the pose between the last two steps, `alpha` from 0 (the step
    /// before) to 1 (the last), and computes the matrices from it.
    pub fn evaluate(&mut self, rig: &Rig, alpha: f32) {
        let alpha = alpha.clamp(0.0, 1.0);
        for ((out, a), b) in self.pose.iter_mut().zip(&self.prev).zip(&self.curr) {
            *out = a.blend(b, alpha);
        }
        rig.model_space_affine(&self.pose, &mut self.model);
        rig.palette(&self.model, &mut self.palette);
    }

    /// The local pose [`Animator::evaluate`] last blended.
    pub fn pose(&self) -> &[Transform] {
        &self.pose
    }

    /// Bind pose to model space for each joint, from the last [`Animator::evaluate`].
    pub fn palette(&self) -> &[Mat4] {
        &self.palette
    }

    /// A joint in model space, from the last [`Animator::evaluate`].
    pub fn joint_model(&self, joint: JointId) -> Mat4 {
        Mat4::from(self.model[joint.0 as usize])
    }

    /// A socket in model space, from the last [`Animator::evaluate`].
    pub fn socket(&self, rig: &Rig, socket: SocketId) -> Mat4 {
        rig.socket_in_model(socket, &self.model)
    }

    /// The cell each face shows after the last step.
    pub fn faces(&self) -> &[u32] {
        &self.faces
    }
}

/// Moves `state` one step on and, with `events`, records what it passed.
/// True when it reached the end of a clip that does not loop.
fn advance(
    rig: &Rig,
    state: &mut ClipState,
    dt: f32,
    tolerance: f64,
    mut events: Option<(usize, &mut Vec<Event>)>,
) -> bool {
    if state.done {
        return false;
    }
    let info = rig.clip_info(state.clip);
    let duration = f64::from(info.duration);
    if duration <= 0.0 && state.mode == PlayMode::Loop {
        return false;
    }
    let mut push = |kind| {
        if let Some((layer, events)) = events.as_mut() {
            events.push(Event {
                layer: *layer,
                clip: state.clip,
                kind,
            });
        }
    };
    let mut start = state.time;
    let mut end = match state.seek.take() {
        Some(time) if state.mode == PlayMode::Loop => {
            start + (time.rem_euclid(duration) - start).rem_euclid(duration)
        }
        Some(time) => time,
        None => start + f64::from(dt) * f64::from(state.speed),
    };
    let mut include_start = std::mem::take(&mut state.fresh);
    loop {
        let wraps = state.mode == PlayMode::Loop && duration > 0.0 && end + tolerance >= duration;
        let segment_end = if wraps { duration } else { end.min(duration) };
        for (i, &(_, at)) in info.markers.iter().enumerate() {
            let at = f64::from(at);
            let after_start = if include_start {
                at + tolerance >= start
            } else {
                at > start + tolerance
            };
            let before_end = at <= segment_end + tolerance;
            if after_start && before_end {
                push(EventKind::Marker(i as u32));
            }
        }
        if wraps {
            push(EventKind::Looped);
            start = 0.0;
            end -= duration;
            include_start = true;
            continue;
        }
        if state.mode != PlayMode::Loop && end + tolerance >= duration {
            state.time = duration;
            state.done = true;
            push(EventKind::Finished);
            return true;
        }
        state.time = end.max(0.0);
        return false;
    }
}

/// The layer's own pose, before its mask and weight, into `layer.out`.
fn blend_layer(
    rig: &Rig,
    layer: &mut Layer,
    below: &[Transform],
    from_pose: &mut [Transform],
    to_pose: &mut [Transform],
) {
    if layer.idle() {
        return;
    }
    let additive = layer.blend == LayerBlend::Additive;
    slot_pose(rig, &layer.to, below, &layer.frozen, additive, to_pose);
    if !layer.fading() {
        layer.out.copy_from_slice(to_pose);
        return;
    }
    slot_pose(rig, &layer.from, below, &layer.frozen, additive, from_pose);
    let t = layer.easing.apply(layer.fade_elapsed / layer.fade_s);
    for ((out, a), b) in layer
        .out
        .iter_mut()
        .zip(from_pose.iter())
        .zip(to_pose.iter())
    {
        *out = a.blend(b, t);
    }
}

fn slot_pose(
    rig: &Rig,
    slot: &Slot,
    below: &[Transform],
    frozen: &[Transform],
    additive: bool,
    out: &mut [Transform],
) {
    match slot {
        Slot::Below if additive => out.fill(Transform::IDENTITY),
        Slot::Below => out.copy_from_slice(below),
        Slot::Frozen => out.copy_from_slice(frozen),
        Slot::Clip(state) => {
            rig.sample(state.clip, state.time as f32, out);
            if additive {
                for (pose, reference) in out.iter_mut().zip(rig.reference(state.clip)) {
                    *pose = delta(pose, reference);
                }
            }
        }
    }
}

/// Puts `layer.out` over `pose` by the layer's weight and mask.
fn apply_layer(layer: &Layer, pose: &mut [Transform]) {
    if layer.idle() || layer.weight <= 0.0 {
        return;
    }
    for (j, (pose, out)) in pose.iter_mut().zip(&layer.out).enumerate() {
        let w = layer.weight * layer.mask.as_ref().map_or(1.0, |m| m[j]);
        if w <= 0.0 {
            continue;
        }
        *pose = match layer.blend {
            LayerBlend::Override if w >= 1.0 => *out,
            LayerBlend::Override => pose.blend(out, w),
            LayerBlend::Additive => apply_delta(pose, out, w),
        };
    }
}
