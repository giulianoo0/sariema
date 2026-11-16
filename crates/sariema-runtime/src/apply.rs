// SPDX-License-Identifier: Apache-2.0

//! Carries out one command against the state. Commands from the game and
//! from the cutscene sequencer both come through here.

use glam::Vec3;
use sariema_format::script::{Bus, Command, Entity, Event, Kind, Property};

use crate::camera::{CameraPose, CameraSettings, Framing};
use crate::events::Happening;
use crate::motion::Goal;
use crate::no_jump;
use crate::outbox::{
    ClipRequest, HostOut, LookAt, MusicRequest, PhysicsIntent, SoundRequest, UiOp,
};
use crate::scenery::SceneReject;
use crate::state::{CutsceneRequest, State};
use crate::tween::Target;
use crate::world::Reject;

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

fn place(s: &mut State, e: Entity, position: Vec3, yaw: f32, force: bool) -> Result<(), Reject> {
    let node = s.world.node_mut(e)?;
    node.local.position = position;
    node.local.yaw = yaw;
    s.world.place(e);
    if force {
        s.world.force(e);
    }
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
        } => {
            s.world.spawn(entity, kind, asset).map_err(refuse(entity))?;
            if matches!(kind, Kind::RoomNode | Kind::Light) {
                s.scenery.bind(&mut s.world, entity, kind, asset);
            }
        }
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
            no_jump::may_place(s, entity, v3(position), yaw)?;
            place(s, entity, v3(position), yaw, false).map_err(refuse(entity))?;
        }
        C::SetParent { entity, parent } => {
            s.world.can_parent(entity, parent).map_err(refuse(entity))?;
            no_jump::may_reparent(s, entity, parent)?;
            s.world.set_parent(entity, parent).map_err(refuse(entity))?;
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
            no_jump::may_walk(s, entity, Goal::Velocity(Vec3::X * speed))?;
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
            no_jump::may_walk(s, entity, goal)?;
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
            no_jump::may_walk(s, entity, goal)?;
            s.motion.mover_mut(entity).goal = goal;
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
                no_jump::may_place(s, entity, v3(position), yaw)?;
            }
            place(s, entity, v3(position), yaw, force).map_err(refuse(entity))?;
        }
        C::Face { entity, yaw } => {
            actor(s, entity)?;
            s.motion.mover_mut(entity).face = Some(yaw);
            s.outbox.physics.push(PhysicsIntent::Face { entity, yaw });
        }
        C::Traverse {
            entity,
            traversal,
            duration,
        } => {
            actor(s, entity)?;
            if s.settings.motion.builtin {
                s.events.push(Event::Blocked { entity });
                s.happen(Happening::Blocked(entity));
            } else {
                s.motion.mover_mut(entity).goal = Goal::Idle;
                s.outbox.physics.push(PhysicsIntent::Traverse {
                    entity,
                    traversal,
                    duration,
                });
            }
        }
        C::LookAt { entity, target } => {
            actor(s, entity)?;
            s.outbox.look_at.push(LookAt { entity, target });
        }
        C::SetSpeedScale { entity, scale } => {
            actor(s, entity)?;
            no_jump::may_rescale(s, entity, scale)?;
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
        C::UiPlay { animation } => s.outbox.ui.push(UiOp::Play { animation }),
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
            no_jump::may_tween(s, &target, to, duration, ease)?;
            s.tweens
                .tween(
                    (&s.world, &s.store, &s.scenery),
                    tween,
                    target,
                    to,
                    duration,
                    ease,
                )
                .map_err(refuse(entity))?;
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
                .spring(
                    (&s.world, &s.store, &s.scenery),
                    tween,
                    target,
                    to,
                    frequency,
                    damping,
                )
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
            no_jump::may_follow(s, entity, target, v3(offset), lag)?;
            s.followers
                .follow(&s.world, entity, target, v3(offset), lag)
                .map_err(refuse(entity))?;
        }
        C::Unfollow { entity } => {
            s.followers.unfollow(entity);
        }
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
        C::RoomPreload { room } => s.scenery.preload(room).map_err(scene)?,
        C::RoomSwitch {
            room,
            transition,
            duration,
        } => {
            if s.scenery
                .switch_to(room, transition, duration)
                .map_err(scene)?
            {
                s.events.push(Event::RoomSwitched { room });
            }
        }
        C::RoomGroup { group, on } => s.scenery.set_group(group, on).map_err(scene)?,
        C::PieceState { piece, state } => s.scenery.set_piece(piece, state).map_err(scene)?,
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
    match t.property {
        Property::Var => Ok(()),
        Property::Scene => s.scenery.param(t.var).map(drop).map_err(scene),
        _ => s.world.check(t.entity).map_err(refuse(t.entity)),
    }
}

fn scene(r: SceneReject) -> Refusal {
    Refusal {
        code: r.code(),
        entity: Entity::NONE,
    }
}
