// SPDX-License-Identifier: Apache-2.0

//! Carries out one command against the state. Commands from the game and
//! from the cutscene sequencer both come through here.

use glam::Vec3;
use sariema_core::STEP_SECONDS;
use sariema_format::script::{Bus, Command, Ease, Entity, Event, Kind, Property};

use crate::camera::{CameraPose, CameraSettings, Framing};
use crate::ease::ease;
use crate::events::Happening;
use crate::follow;
use crate::motion::Goal;
use crate::outbox::{
    ClipRequest, HostOut, LookAt, MusicRequest, PhysicsIntent, SoundRequest, UiOp,
};
use crate::state::{CutsceneRequest, State};
use crate::tween::{self, Target, steps_of};
use crate::world::{Pose, Reject};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Game,
    Cutscene,
}

/// Why a command was refused: a code for the `rejected` event and the
/// entity it was about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: &'static str,
    pub entity: Entity,
}

fn refuse(entity: Entity) -> impl Fn(Reject) -> Refusal {
    move |r| Refusal {
        code: r.code(),
        entity,
    }
}

fn v3(v: [f32; 3]) -> Vec3 {
    Vec3::from_array(v)
}

/// The early refusal of the no-jump rule, asked by every command that moves
/// something; [`hold_jumps`] is the guarantee. `moved` is how far `e` goes
/// in one step, infinite for a placement. Refused, on a screen neither fully
/// faded nor cut, when an actor that was visible when the step began and is
/// not placed yet this step would go farther than `motion.max_visible_speed`
/// allows: `e`, a child of it at any depth (going as far), or a follower
/// (going its gain of it).
pub(crate) fn may_jump(s: &State, e: Entity, moved: f32) -> Result<(), Refusal> {
    if moved <= step_limit(s) || s.screen.covered() || s.camera.cut() {
        return Ok(());
    }
    let mut budget = s.world.len() as usize + s.followers.len();
    if seen_moving(s, e, moved, &mut budget) {
        return Err(refuse(e)(Reject::TeleportVisible));
    }
    Ok(())
}

/// The farthest a visible actor may go in one step.
fn step_limit(s: &State) -> f32 {
    s.settings.motion.max_visible_speed * STEP_SECONDS
}

/// Room for the rounding of composed placements when measuring a step.
const SLACK: f32 = 1.0 + 1e-4;

/// The no-jump rule measured once every system has moved things this step.
/// A visible actor that went farther than `motion.max_visible_speed` or
/// turned faster than `motion.turn_rate` since the previous step, with the
/// screen fully faded at neither end of the step and no camera cut, is put
/// back where it was and refused; what moved it stops: the tweens, springs,
/// follow and walk of the actor and of everything that carries it. Returns
/// whether anything was put back.
pub(crate) fn hold_jumps(s: &mut State) -> bool {
    if s.screen.covered() || s.screen.covered_before() || s.camera.cut() {
        return false;
    }
    let jumped = s.world.jumps(step_limit(s) * SLACK, turn_limit(s) * SLACK);
    for &e in &jumped {
        let mut at = e;
        while let Some(node) = s.world.node(at) {
            let parent = node.parent;
            stop(s, at);
            at = parent;
        }
        s.world.hold(e);
        let local = s.world.node(e).expect("a jumped actor is alive").local;
        s.outbox.physics.push(PhysicsIntent::Teleport {
            entity: e,
            position: local.position,
            yaw: local.yaw,
        });
        s.events.push(Event::Rejected {
            opcode: 0,
            code: Reject::TeleportVisible.code(),
            entity: e,
        });
    }
    !jumped.is_empty()
}

fn stop(s: &mut State, e: Entity) {
    s.tweens.forget(e);
    s.followers.unfollow(e);
    if s.motion.get(e).is_some_and(|m| m.goal != Goal::Idle) {
        s.motion.mover_mut(e).goal = Goal::Idle;
        s.outbox.physics.push(PhysicsIntent::Stop { entity: e });
    }
}

/// `budget` bounds the walk, which a cycle of followers would not end.
fn seen_moving(s: &State, e: Entity, moved: f32, budget: &mut usize) -> bool {
    if *budget == 0 || moved <= step_limit(s) {
        return false;
    }
    *budget -= 1;
    let Some(node) = s.world.node(e) else {
        return false;
    };
    if node.kind == Kind::Actor && s.world.was_visible(e) && !s.world.snapped_this_step(e) {
        return true;
    }
    s.world
        .children(e)
        .any(|c| seen_moving(s, c, moved, budget))
        || s.followers
            .of(e)
            .any(|(f, gain)| seen_moving(s, f, moved * gain, budget))
}

/// How far going from `a` to `b` in one step moves: infinite when it turns
/// faster than `motion.turn_rate`.
fn step_move(s: &State, a: Pose, b: Pose) -> f32 {
    if a.rotation.angle_between(b.rotation).to_degrees() > turn_limit(s) {
        f32::INFINITY
    } else {
        a.position.distance(b.position)
    }
}

fn turn_limit(s: &State) -> f32 {
    s.settings.motion.turn_rate * STEP_SECONDS
}

/// How far a change of `property` by `change` in one step moves its entity.
fn property_move(s: &State, property: Property, change: f32) -> f32 {
    match property {
        Property::PositionX | Property::PositionY | Property::PositionZ => change,
        Property::Yaw | Property::Pitch | Property::Roll if change > turn_limit(s) => f32::INFINITY,
        _ => 0.0,
    }
}

/// The farthest one step of a tween of `t` to `to` moves it.
fn tween_move(s: &State, t: &Target, to: f32, duration: f32, curve: Ease) -> f32 {
    let Ok(from) = tween::read(&s.world, &s.store, t) else {
        return 0.0;
    };
    let steps = steps_of(duration);
    let (mut most, mut last) = (0.0f32, 0.0);
    for k in 1..=steps {
        let now = if k == steps {
            1.0
        } else {
            ease(curve, k as f32 / steps as f32)
        };
        most = most.max((now - last).abs());
        last = now;
    }
    property_move(s, t.property, most * (to - from).abs())
}

/// How far placing `e` at local `position` and `yaw` moves it; infinite for
/// an actor, since placing an actor is never a walk.
fn placement_move(s: &State, e: Entity, position: Vec3, yaw: f32) -> f32 {
    let Some(node) = s.world.node(e) else {
        return 0.0;
    };
    let turn = ((yaw - node.local.yaw + 180.0).rem_euclid(360.0) - 180.0).abs();
    if node.kind == Kind::Actor || turn > turn_limit(s) {
        return f32::INFINITY;
    }
    let scale = s.world.current_pose(node.parent).map_or(1.0, |p| p.scale);
    node.local.position.distance(position) * scale
}

/// How far `e` walking to `goal` with its speed scaled by `scale` goes in
/// one step.
fn walk_move(s: &State, e: Entity, goal: Goal, scale: f32) -> f32 {
    match goal {
        Goal::Idle => 0.0,
        Goal::Velocity(v) => v.length() * scale * STEP_SECONDS,
        Goal::To { target, speed } => {
            let at = s.world.node(e).map_or(target, |n| n.local.position);
            (speed.abs() * scale * STEP_SECONDS).min(at.distance(target))
        }
    }
}

fn place(s: &mut State, e: Entity, position: Vec3, yaw: f32) -> Result<(), Reject> {
    let node = s.world.node_mut(e)?;
    node.local.position = position;
    node.local.yaw = yaw;
    s.world.snap(e);
    if s.settings.motion.builtin {
        let m = s.motion.mover_mut(e);
        m.goal = Goal::Idle;
        m.face = None;
    }
    s.outbox.physics.push(PhysicsIntent::Teleport {
        entity: e,
        position,
        yaw,
    });
    Ok(())
}

fn forget(s: &mut State, e: Entity) {
    s.clips.forget(e);
    s.tweens.forget(e);
    s.followers.forget(e);
    s.motion.forget(e);
    if s.control.hero == e {
        s.control.hero = Entity::NONE;
    }
}

pub fn apply(s: &mut State, command: &Command<'_>, source: Source) -> Result<(), Refusal> {
    use Command as C;
    match *command {
        C::Spawn {
            entity,
            kind,
            asset,
        } => s.world.spawn(entity, kind, asset).map_err(refuse(entity))?,
        C::Despawn { entity } => {
            s.world.despawn(entity).map_err(refuse(entity))?;
            forget(s, entity);
            if source == Source::Cutscene {
                s.events.push(Event::Despawned { entity });
            }
        }
        C::SetTransform {
            entity,
            position,
            yaw,
        } => {
            s.world.check(entity).map_err(refuse(entity))?;
            may_jump(s, entity, placement_move(s, entity, v3(position), yaw))?;
            place(s, entity, v3(position), yaw).map_err(refuse(entity))?;
        }
        C::SetParent { entity, parent } => {
            let before = s.world.current_pose(entity);
            let old = s.world.node(entity).map_or(Entity::NONE, |n| n.parent);
            s.world.set_parent(entity, parent).map_err(refuse(entity))?;
            let after = s.world.current_pose(entity);
            if let (Some(a), Some(b)) = (before, after) {
                let moved = step_move(s, a, b);
                if let Err(r) = may_jump(s, entity, moved) {
                    s.world
                        .set_parent(entity, old)
                        .expect("the previous parent was valid");
                    return Err(r);
                }
                if moved > step_limit(s) {
                    s.world.snap(entity);
                }
            }
        }
        C::SetVisible { entity, visible } => {
            s.world.node_mut(entity).map_err(refuse(entity))?.visible = visible
        }
        C::SetScale { entity, scale } => {
            s.world
                .node_mut(entity)
                .map_err(refuse(entity))?
                .local
                .scale = scale
        }
        C::SetTag { entity, tag, on } => {
            let tags = &mut s.world.node_mut(entity).map_err(refuse(entity))?.tags;
            match (tags.binary_search(&tag), on) {
                (Err(i), true) => tags.insert(i, tag),
                (Ok(i), false) => {
                    tags.remove(i);
                }
                _ => {}
            }
        }
        C::SetHero { entity, speed } => {
            actor(s, entity)?;
            let hero = Goal::Velocity(Vec3::X * speed);
            may_jump(
                s,
                entity,
                walk_move(s, entity, hero, speed_scale(s, entity)),
            )?;
            s.control.hero = entity;
            s.control.speed = speed;
        }
        C::Control { by } => s.hand_over(by),
        C::MoveTo {
            entity,
            target,
            speed,
        } => {
            actor(s, entity)?;
            let goal = Goal::To {
                target: v3(target),
                speed,
            };
            may_jump(
                s,
                entity,
                walk_move(s, entity, goal, speed_scale(s, entity)),
            )?;
            s.motion.mover_mut(entity).goal = goal;
            s.outbox.physics.push(PhysicsIntent::MoveTo {
                entity,
                target: v3(target),
                speed,
            });
        }
        C::MoveIntent { entity, velocity } => {
            actor(s, entity)?;
            let goal = Goal::Velocity(v3(velocity));
            may_jump(
                s,
                entity,
                walk_move(s, entity, goal, speed_scale(s, entity)),
            )?;
            s.motion.mover_mut(entity).goal = Goal::Velocity(v3(velocity));
            s.outbox.physics.push(PhysicsIntent::Velocity {
                entity,
                velocity: v3(velocity),
            });
        }
        C::Stop { entity } => {
            actor(s, entity)?;
            s.motion.mover_mut(entity).goal = Goal::Idle;
            s.outbox.physics.push(PhysicsIntent::Stop { entity });
        }
        C::Teleport {
            entity,
            position,
            yaw,
            force,
        } => {
            s.world.check(entity).map_err(refuse(entity))?;
            if !force {
                may_jump(s, entity, placement_move(s, entity, v3(position), yaw))?;
            }
            place(s, entity, v3(position), yaw).map_err(refuse(entity))?;
        }
        C::Face { entity, yaw } => {
            actor(s, entity)?;
            s.motion.mover_mut(entity).face = Some(yaw);
        }
        C::LookAt { entity, target } => {
            actor(s, entity)?;
            s.outbox.look_at.push(LookAt { entity, target });
        }
        C::SetSpeedScale { entity, scale } => {
            actor(s, entity)?;
            let goal = s.motion.get(entity).map_or(Goal::Idle, |m| m.goal);
            let mut moved = walk_move(s, entity, goal, scale);
            if s.control.hero == entity {
                let hero = Goal::Velocity(Vec3::X * s.control.speed);
                moved = moved.max(walk_move(s, entity, hero, scale));
            }
            may_jump(s, entity, moved)?;
            s.motion.mover_mut(entity).speed_scale = scale;
            s.outbox
                .physics
                .push(PhysicsIntent::SpeedScale { entity, scale });
        }
        C::Knockback { entity, impulse } => {
            actor(s, entity)?;
            s.outbox.physics.push(PhysicsIntent::Knockback {
                entity,
                impulse: v3(impulse),
            });
        }
        C::SetCollider { entity, enabled } => {
            s.world.check(entity).map_err(refuse(entity))?;
            s.outbox
                .physics
                .push(PhysicsIntent::Collider { entity, enabled });
        }
        C::PlayClip {
            entity,
            clip,
            mode,
            blend,
            speed,
        } => {
            actor(s, entity)?;
            s.outbox.clips.push(ClipRequest {
                entity,
                clip,
                mode,
                blend,
                speed,
            });
            s.clips.play(entity, clip, speed);
        }
        C::PlaySound {
            sound,
            bus,
            emitter,
            volume,
        } => {
            if !emitter.is_none() {
                s.world.check(emitter).map_err(refuse(emitter))?;
            }
            s.outbox.sounds.push(SoundRequest {
                sound,
                bus,
                emitter,
                volume,
            });
        }
        C::PlayMusic { music, fade } => s.outbox.music.push(MusicRequest::Play { music, fade }),
        C::StopMusic { fade } => s.outbox.music.push(MusicRequest::Stop { fade }),
        C::SetBusVolume { bus, volume, fade } => {
            s.outbox
                .music
                .push(MusicRequest::BusVolume { bus, volume, fade })
        }
        C::UiVisible { node, visible } => s.outbox.ui.push(UiOp::Visible { node, visible }),
        C::UiText { node, text } => s.outbox.ui.push(UiOp::Text {
            node,
            text: text.into(),
        }),
        C::UiValue { node, value } => s.outbox.ui.push(UiOp::Value { node, value }),
        C::Say {
            line,
            speaker,
            text,
            voice,
            advance,
            duration,
        } => {
            if !speaker.is_none() {
                s.world.check(speaker).map_err(refuse(speaker))?;
            }
            if let Some(replaced) = s
                .dialogue
                .say(line, speaker, text, voice, advance, duration)
            {
                s.end_line(replaced);
            }
            if voice != 0 {
                s.outbox.sounds.push(SoundRequest {
                    sound: voice,
                    bus: Bus::Voice,
                    emitter: speaker,
                    volume: 1.0,
                });
            }
        }
        C::AdvanceLine {} => {
            if let Some(line) = s.dialogue.advance() {
                s.end_line(line);
            }
        }
        C::Fade {
            to,
            duration,
            color,
        } => {
            s.screen.fade.set(to.clamp(0.0, 1.0), duration);
            s.screen.fade_color = v3(color);
        }
        C::Letterbox { to, duration } => s.screen.letterbox.set(to.clamp(0.0, 1.0), duration),
        C::TimeScale { scale, blend } => s.time_scale.set(
            scale
                .max(s.settings.min_time_scale)
                .min(s.settings.max_time_scale),
            blend,
        ),
        C::Tween {
            tween,
            entity,
            property,
            var,
            to,
            duration,
            ease,
        } => {
            let target = Target {
                entity,
                property,
                var,
            };
            tweenable(s, &target)?;
            let moved = tween_move(s, &target, to, duration, ease);
            may_jump(s, entity, moved)?;
            s.tweens
                .tween(&s.world, &s.store, tween, target, to, duration, ease)
                .map_err(refuse(entity))?;
            if steps_of(duration) == 1 && moved > step_limit(s) {
                s.world.snap(entity);
            }
        }
        C::Spring {
            tween,
            entity,
            property,
            var,
            to,
            frequency,
            damping,
        } => {
            let target = Target {
                entity,
                property,
                var,
            };
            tweenable(s, &target)?;
            s.tweens
                .spring(&s.world, &s.store, tween, target, to, frequency, damping)
                .map_err(refuse(entity))?;
        }
        C::CancelTween { tween } => s.tweens.cancel(tween),
        C::SetVar { var, value } => {
            s.store.vars.insert(var, value);
        }
        C::Follow {
            entity,
            target,
            offset,
            lag,
        } => {
            let gain = follow::gain(lag);
            let moved = match (s.world.current_pose(entity), s.world.current_pose(target)) {
                (Some(me), Some(to)) => me.position.distance(to.position + v3(offset)) * gain,
                _ => 0.0,
            };
            may_jump(s, entity, moved)?;
            s.followers
                .follow(&s.world, entity, target, v3(offset), lag)
                .map_err(refuse(entity))?;
            if gain >= 1.0 && moved > step_limit(s) {
                s.world.snap(entity);
            }
        }
        C::Unfollow { entity } => s.followers.unfollow(entity),
        C::Timer {
            timer,
            duration,
            repeat,
        } => s.timers.set(timer, duration, repeat),
        C::CancelTimer { timer } => s.timers.cancel(timer),
        C::Zone {
            zone,
            center,
            radius,
        } => s.zones.set(zone, v3(center), radius),
        C::RemoveZone { zone } => s.zones.remove(zone),
        C::RigFollow {
            rig,
            subject,
            distance,
            pitch,
            yaw,
            height,
            fov,
        } => {
            s.world.check(subject).map_err(refuse(subject))?;
            s.camera.set_framing(
                rig,
                Framing::Follow {
                    subject,
                    distance,
                    pitch,
                    yaw,
                    height,
                    fov,
                },
            );
        }
        C::RigFrame {
            rig,
            a,
            b,
            pitch,
            yaw,
            height,
            fov,
            margin,
        } => {
            s.world.check(a).map_err(refuse(a))?;
            s.world.check(b).map_err(refuse(b))?;
            s.camera.set_framing(
                rig,
                Framing::Frame {
                    a,
                    b,
                    pitch,
                    yaw,
                    height,
                    fov,
                    margin,
                },
            );
        }
        C::RigFixed {
            rig,
            position,
            look_at,
            fov,
        } => s.camera.set_framing(
            rig,
            Framing::Fixed(CameraPose::looking(v3(position), v3(look_at), fov)),
        ),
        C::RigTuning {
            rig,
            dead_zone,
            frequency,
            damping,
            look_ahead,
        } => s
            .camera
            .set_tuning(
                rig,
                CameraSettings {
                    dead_zone,
                    frequency,
                    damping,
                    look_ahead,
                    ..s.settings.camera
                },
            )
            .map_err(|e| Refusal {
                code: e.code(),
                entity: Entity::NONE,
            })?,
        C::RigBounds { rig, min, max } => {
            s.camera
                .set_bounds(rig, v3(min), v3(max))
                .map_err(|e| Refusal {
                    code: e.code(),
                    entity: Entity::NONE,
                })?
        }
        C::CameraUse { rig, blend, ease } => {
            s.camera.use_rig(rig, blend, ease).map_err(|e| Refusal {
                code: e.code(),
                entity: Entity::NONE,
            })?
        }
        C::CameraShake {
            amplitude,
            frequency,
            duration,
        } => s.camera.shake(amplitude, frequency, duration, &mut s.rng),
        C::RigAnchor {
            rig,
            subject,
            anchor,
            name,
        } => {
            s.world.check(subject).map_err(refuse(subject))?;
            s.camera
                .set_anchor(rig, subject, anchor, name)
                .map_err(|e| Refusal {
                    code: e.code(),
                    entity: Entity::NONE,
                })?
        }
        C::CutsceneBind { role, entity } => s.requests.push(CutsceneRequest::Bind { role, entity }),
        C::CutscenePlay { cutscene } => s.requests.push(CutsceneRequest::Play(cutscene)),
        C::CutsceneSkip {} => s.requests.push(CutsceneRequest::Skip),
        C::Signal { signal } => s.happen(Happening::Signal(signal)),
        C::SetFlag { flag, on } => {
            if s.store.set_flag(flag, on) && source == Source::Cutscene {
                s.events.push(Event::FlagChanged { flag, on });
            }
        }
        C::Consume { action } => {
            s.input.consume(action);
        }
        C::HostReady {} => s.outbox.host.push(HostOut::Ready),
        C::HostExit {} => s.outbox.host.push(HostOut::Exit),
        C::HostOrientation { orientation } => s.outbox.host.push(HostOut::Orientation(orientation)),
        C::Save { key, value } => s.outbox.host.push(HostOut::Save {
            key: key.into(),
            value: value.into(),
        }),
        C::SaveLoad {} => s.outbox.host.push(HostOut::LoadSaves),
        C::HostSend { topic, payload } => s.outbox.host.push(HostOut::Message {
            topic: topic.into(),
            payload: payload.into(),
        }),
    }
    Ok(())
}

fn speed_scale(s: &State, e: Entity) -> f32 {
    s.motion.get(e).map_or(1.0, |m| m.speed_scale)
}

fn actor(s: &State, e: Entity) -> Result<(), Refusal> {
    match s.world.node(e) {
        None => Err(refuse(e)(Reject::Stale)),
        Some(n) if n.kind != Kind::Actor => Err(refuse(e)(Reject::NotActor)),
        Some(_) => Ok(()),
    }
}

fn tweenable(s: &State, t: &Target) -> Result<(), Refusal> {
    if t.property == Property::Var {
        return Ok(());
    }
    s.world.check(t.entity).map_err(refuse(t.entity))
}
