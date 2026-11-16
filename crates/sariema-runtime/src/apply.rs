// SPDX-License-Identifier: Apache-2.0

//! Carries out one command against the state. Commands from the game and
//! from the cutscene sequencer both come through here.

use glam::Vec3;
use sariema_core::STEP_SECONDS;
use sariema_format::script::{Bus, Command, Entity, Event, Kind, Property};

use crate::camera::{CameraPose, CameraSettings, Framing};
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

/// The one owner of the no-jump rule; every command that moves something at
/// once asks it first. Refused when `e`, or an actor it carries (a child at
/// any depth, a rigid follower), was visible when the step began and is not
/// placed yet this step, on a screen neither fully faded nor cut.
fn may_jump(s: &State, e: Entity) -> Result<(), Refusal> {
    if s.screen.covered() || s.camera.cut() {
        return Ok(());
    }
    let mut budget = s.world.len() as usize + s.followers.len();
    if seen_moving(s, e, &mut budget) {
        return Err(refuse(e)(Reject::TeleportVisible));
    }
    Ok(())
}

/// `budget` bounds the walk, which a cycle of rigid followers would not end.
fn seen_moving(s: &State, e: Entity, budget: &mut usize) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    let Some(node) = s.world.node(e) else {
        return false;
    };
    if node.kind == Kind::Actor && s.world.was_visible(e) && !s.world.snapped_this_step(e) {
        return true;
    }
    s.world.children(e).any(|c| seen_moving(s, c, budget))
        || s.followers.rigid(e).any(|f| seen_moving(s, f, budget))
}

/// Whether going from `a` to `b` in one step is more than walking or
/// turning could do.
fn sudden(s: &State, a: Pose, b: Pose) -> bool {
    a.position.distance(b.position) > s.settings.motion.arrive_distance
        || a.rotation.angle_between(b.rotation).to_degrees()
            > s.settings.motion.turn_rate * STEP_SECONDS
}

/// Whether a tween of `t` to `to` that ends in one step moves it suddenly.
fn sudden_tween(s: &State, t: &Target, to: f32, duration: f32) -> bool {
    if steps_of(duration) > 1 {
        return false;
    }
    let Ok(from) = tween::read(&s.world, &s.store, t) else {
        return false;
    };
    let limit = match t.property {
        Property::PositionX | Property::PositionY | Property::PositionZ => {
            s.settings.motion.arrive_distance
        }
        Property::Yaw | Property::Pitch | Property::Roll => {
            s.settings.motion.turn_rate * STEP_SECONDS
        }
        Property::Scale | Property::Var => return false,
    };
    (to - from).abs() > limit
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
            may_jump(s, entity)?;
            place(s, entity, v3(position), yaw).map_err(refuse(entity))?;
        }
        C::SetParent { entity, parent } => {
            let before = s.world.current_pose(entity);
            let old = s.world.node(entity).map_or(Entity::NONE, |n| n.parent);
            s.world.set_parent(entity, parent).map_err(refuse(entity))?;
            let after = s.world.current_pose(entity);
            if let (Some(a), Some(b)) = (before, after)
                && sudden(s, a, b)
            {
                if let Err(r) = may_jump(s, entity) {
                    s.world
                        .set_parent(entity, old)
                        .expect("the previous parent was valid");
                    return Err(r);
                }
                s.world.snap(entity);
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
            s.motion.mover_mut(entity).goal = Goal::To {
                target: v3(target),
                speed,
            };
            s.outbox.physics.push(PhysicsIntent::MoveTo {
                entity,
                target: v3(target),
                speed,
            });
        }
        C::MoveIntent { entity, velocity } => {
            actor(s, entity)?;
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
                may_jump(s, entity)?;
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
            let sudden = sudden_tween(s, &target, to, duration);
            if sudden {
                may_jump(s, entity)?;
            }
            s.tweens
                .tween(&s.world, &s.store, tween, target, to, duration, ease)
                .map_err(refuse(entity))?;
            if sudden {
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
            let sudden = follow::gain(lag) >= 1.0
                && match (s.world.current_pose(entity), s.world.current_pose(target)) {
                    (Some(me), Some(to)) => {
                        me.position.distance(to.position + v3(offset))
                            > s.settings.motion.arrive_distance
                    }
                    _ => false,
                };
            if sudden {
                may_jump(s, entity)?;
            }
            s.followers
                .follow(&s.world, entity, target, v3(offset), lag)
                .map_err(refuse(entity))?;
            if sudden {
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
