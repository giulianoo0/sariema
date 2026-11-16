// SPDX-License-Identifier: Apache-2.0

//! The cutscene sequencer. Every block turns into the same commands a game
//! could send, applied at once so later blocks of the same step see their
//! effect.
//!
//! - Before the body runs, every role is brought to its declared start: a
//!   hidden actor is placed, a near one eased over `start_blend`, a far one
//!   walked there; then it turns to the declared yaw. Nothing snaps.
//! - Skipping fades out, runs the rest of the body to its end state at once
//!   (walks land on their targets, lines and waits end, sounds are dropped)
//!   and fades back in, so the world ends as if the cutscene had played.
//! - At the end the hero goes back to whoever had it, the input buffer is
//!   cleared, and the camera blends back to the rig the game was using.

use std::collections::BTreeMap;

use glam::Vec3;
use sariema_format::script::cutscene::{Await, Block, Cutscene, Place, Shot, Toward};
use sariema_format::script::{
    Advance, Bus, ClipMode, Command, Controller, Ease, Entity, Event, Kind, Property, name_id,
};

use crate::apply::{Source, apply};
use crate::events::Happening;
use crate::motion::yaw_of;
use crate::state::{CutsceneRequest, State};
use crate::tween::steps_of;

/// The sequencer's camera rig and dialogue line ids.
pub const RIG: u32 = u32::MAX;
pub const LINE: u32 = u32::MAX;
/// Tween ids from here up belong to the sequencer.
pub const TWEENS: u32 = 0xFFFF_0000;

// A macro rather than a function so the borrow of `run` stays per field.
macro_rules! ctx {
    ($s:expr, $cutscene:expr, $run:expr, $skip:expr) => {
        Ctx {
            s: $s,
            cutscene: $cutscene,
            roles: &$run.roles,
            id: $run.id,
            skip: $skip,
            final_fade: &mut $run.final_fade,
            final_color: &mut $run.final_color,
        }
    };
}

#[derive(Debug)]
enum Running {
    List(List),
    Parallel(Vec<(List, bool)>),
    Branch(List, bool),
    Wait(u32),
    WaitFor {
        since: u64,
        left: Option<u32>,
    },
    Walk {
        since: u64,
        yaw: f32,
        to: Vec3,
        turning: bool,
    },
    Turn {
        yaw: f32,
    },
    Line {
        since: u64,
    },
    Fade {
        to: f32,
    },
}

#[derive(Debug, Default)]
struct List {
    at: usize,
    current: Option<Box<Running>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Placing {
    Done,
    Easing,
    Walking,
    Turning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Placing,
    Playing,
    SkipOut,
    SkipIn,
}

#[derive(Debug)]
struct Run {
    index: usize,
    id: u32,
    roles: Vec<Entity>,
    placing: Vec<Placing>,
    phase: Phase,
    body: List,
    rig_before: Option<u32>,
    control_before: Controller,
    final_fade: f32,
    final_color: [f32; 3],
}

struct Ctx<'a> {
    s: &'a mut State,
    cutscene: &'a Cutscene,
    roles: &'a [Entity],
    id: u32,
    skip: bool,
    final_fade: &'a mut f32,
    final_color: &'a mut [f32; 3],
}

impl Ctx<'_> {
    fn issue(&mut self, command: Command<'_>) {
        if let Err(r) = apply(self.s, &command, Source::Cutscene) {
            self.s.events.push(Event::Rejected {
                opcode: command.opcode(),
                code: r.code,
                entity: r.entity,
            });
        }
    }

    fn role(&self, r: u16) -> Entity {
        self.roles[r as usize]
    }

    fn position(&self, e: Entity) -> Vec3 {
        self.s
            .world
            .node(e)
            .map_or(Vec3::ZERO, |n| n.local.position)
    }

    fn yaw(&self, e: Entity) -> f32 {
        self.s.world.node(e).map_or(0.0, |n| n.local.yaw)
    }

    fn place(&mut self, e: Entity, position: Vec3, yaw: f32) {
        self.issue(Command::Teleport {
            entity: e,
            position: position.to_array(),
            yaw,
            force: true,
        });
    }

    fn builtin(&self) -> bool {
        self.s.settings.motion.builtin
    }
}

pub struct Sequencer {
    cutscenes: Vec<Cutscene>,
    by_id: BTreeMap<u32, usize>,
    bindings: BTreeMap<u32, Entity>,
    run: Option<Run>,
}

fn reject(s: &mut State, code: &'static str, entity: Entity) {
    s.events.push(Event::Rejected {
        opcode: Command::CutscenePlay { cutscene: 0 }.opcode(),
        code,
        entity,
    });
}

impl Sequencer {
    pub fn new(cutscenes: Vec<Cutscene>) -> Sequencer {
        let by_id = cutscenes
            .iter()
            .enumerate()
            .map(|(i, c)| (name_id(&c.name), i))
            .collect();
        Sequencer {
            cutscenes,
            by_id,
            bindings: BTreeMap::new(),
            run: None,
        }
    }

    pub fn playing(&self) -> Option<u32> {
        self.run.as_ref().map(|r| r.id)
    }

    pub fn step(&mut self, s: &mut State) {
        for request in std::mem::take(&mut s.requests) {
            match request {
                CutsceneRequest::Bind { role, entity } => {
                    self.bindings.insert(role, entity);
                }
                CutsceneRequest::Play(id) => self.start(s, id),
                CutsceneRequest::Skip => self.skip(s),
            }
        }
        let skip_action = s.input.settings().button(&s.settings.cutscene.skip_action);
        if skip_action.is_some_and(|b| s.input.pressed(b)) {
            self.skip(s);
        }
        let Some(run) = &mut self.run else { return };
        let cutscene = &self.cutscenes[run.index];
        let mut ended = None;
        match run.phase {
            Phase::Placing => {
                if place_step(s, cutscene, run) {
                    run.phase = Phase::Playing;
                }
            }
            Phase::SkipOut if s.screen.fade.done() => {
                for (i, role) in cutscene.roles.iter().enumerate() {
                    if let (Some(start), true) = (role.start, run.placing[i] != Placing::Done) {
                        (0..3).for_each(|k| s.tweens.cancel(TWEENS + 3 * i as u32 + k));
                        let e = run.roles[i];
                        ctx!(s, cutscene, run, true).place(
                            e,
                            Vec3::from_array(start.position),
                            start.yaw,
                        );
                    }
                }
                run.placing.iter_mut().for_each(|p| *p = Placing::Done);
                let duration = s.settings.cutscene.skip_fade;
                let mut ctx = ctx!(s, cutscene, run, true);
                tick_list(&mut run.body, &cutscene.body, &mut ctx);
                let (to, color) = (*ctx.final_fade, *ctx.final_color);
                ctx.issue(Command::Fade {
                    to,
                    duration,
                    color,
                });
                run.phase = Phase::SkipIn;
            }
            Phase::SkipIn if s.screen.fade.done() => ended = Some(true),
            _ => {}
        }
        if run.phase == Phase::Playing {
            let mut ctx = ctx!(s, cutscene, run, false);
            if tick_list(&mut run.body, &cutscene.body, &mut ctx) {
                ended = Some(false);
            }
        }
        if let Some(skipped) = ended {
            self.end(s, skipped);
        }
    }

    fn start(&mut self, s: &mut State, id: u32) {
        if self.run.is_some() {
            return reject(s, "E_CUTSCENE_BUSY", Entity::NONE);
        }
        let Some(&index) = self.by_id.get(&id) else {
            return reject(s, "E_CUTSCENE_UNKNOWN", Entity::NONE);
        };
        let cutscene = &self.cutscenes[index];
        let mut roles = Vec::with_capacity(cutscene.roles.len());
        for role in &cutscene.roles {
            let bound = self.bindings.get(&name_id(&role.name)).copied();
            match (bound, role.start) {
                (Some(e), Some(_)) if s.world.is_alive(e) => roles.push(e),
                (Some(e), None) if !s.world.is_alive(e) => roles.push(e),
                (e, _) => return reject(s, "E_CUTSCENE_UNBOUND", e.unwrap_or(Entity::NONE)),
            }
        }
        self.bindings.clear();
        let control_before = s.control.by;
        s.hand_over(Controller::Cutscene);
        let mut run = Run {
            index,
            id,
            placing: vec![Placing::Done; roles.len()],
            roles,
            phase: Phase::Placing,
            body: List::default(),
            rig_before: s.camera.active(),
            control_before,
            final_fade: s.screen.fade.target(),
            final_color: s.screen.fade_color.to_array(),
        };
        begin_placing(s, cutscene, &mut run);
        s.events.push(Event::CutsceneStarted { cutscene: id });
        self.run = Some(run);
    }

    fn skip(&mut self, s: &mut State) {
        let Some(run) = &mut self.run else { return };
        if !self.cutscenes[run.index].skippable
            || !matches!(run.phase, Phase::Placing | Phase::Playing)
        {
            return;
        }
        if s.dialogue.line.as_ref().is_some_and(|l| l.line == LINE) {
            s.dialogue.line = None;
            s.end_line(LINE);
        }
        run.final_fade = s.screen.fade.target();
        run.final_color = s.screen.fade_color.to_array();
        let duration = s.settings.cutscene.skip_fade;
        let cutscene = &self.cutscenes[run.index];
        ctx!(s, cutscene, run, true).issue(Command::Fade {
            to: 1.0,
            duration,
            color: [0.0; 3],
        });
        run.phase = Phase::SkipOut;
    }

    fn end(&mut self, s: &mut State, skipped: bool) {
        let Some(run) = self.run.take() else { return };
        s.hand_over(run.control_before);
        s.input.clear_buffer();
        if let Some(rig) = run.rig_before {
            let (blend, ease) = (s.settings.cutscene.end_blend, s.settings.cutscene.end_ease);
            let _ = s.camera.use_rig(rig, blend, ease);
        }
        s.events.push(Event::CutsceneEnded {
            cutscene: run.id,
            skipped,
        });
    }
}

fn begin_placing(s: &mut State, cutscene: &Cutscene, run: &mut Run) {
    let settings = s.settings.cutscene.clone();
    for (i, role) in cutscene.roles.iter().enumerate() {
        let Some(Place { position, yaw }) = role.start else {
            continue;
        };
        let e = run.roles[i];
        let to = Vec3::from_array(position);
        let mut ctx = ctx!(s, cutscene, run, false);
        let at = ctx.position(e);
        let visible = ctx.s.world.node(e).is_some_and(|n| n.visible);
        let distance = at.distance(to);
        let placing = if !visible {
            ctx.place(e, to, yaw);
            Placing::Done
        } else if distance <= settings.start_tolerance {
            ctx.issue(Command::Face { entity: e, yaw });
            Placing::Turning
        } else if distance <= settings.start_blend_max {
            for (k, property) in [
                Property::PositionX,
                Property::PositionY,
                Property::PositionZ,
            ]
            .into_iter()
            .enumerate()
            {
                ctx.issue(Command::Tween {
                    tween: TWEENS + 3 * i as u32 + k as u32,
                    entity: e,
                    property,
                    var: 0,
                    to: to[k],
                    duration: settings.start_blend,
                    ease: Ease::Smoothstep,
                });
            }
            ctx.issue(Command::Face { entity: e, yaw });
            Placing::Easing
        } else {
            ctx.issue(Command::MoveTo {
                entity: e,
                target: position,
                speed: settings.start_walk_speed,
            });
            Placing::Walking
        };
        run.placing[i] = placing;
    }
}

/// Advances every role towards its start; whether all are there.
fn place_step(s: &mut State, cutscene: &Cutscene, run: &mut Run) -> bool {
    let mut all = true;
    for i in 0..run.placing.len() {
        let e = run.roles[i];
        let yaw = cutscene.roles[i].start.map_or(0.0, |p| p.yaw);
        let easing = (0..3).any(|k| s.tweens.active(TWEENS + 3 * i as u32 + k));
        let next = match run.placing[i] {
            Placing::Easing if easing => Placing::Easing,
            Placing::Walking if s.motion.walking(e) => Placing::Walking,
            Placing::Easing | Placing::Walking => {
                ctx!(s, cutscene, run, false).issue(Command::Face { entity: e, yaw });
                Placing::Turning
            }
            Placing::Turning if s.motion.turning(e) && s.settings.motion.builtin => {
                Placing::Turning
            }
            Placing::Turning | Placing::Done => Placing::Done,
        };
        run.placing[i] = next;
        all &= next == Placing::Done;
    }
    all
}

fn tick_list(list: &mut List, blocks: &[Block], ctx: &mut Ctx<'_>) -> bool {
    while let Some(block) = blocks.get(list.at) {
        if !tick_block(&mut list.current, block, ctx) {
            return false;
        }
        list.current = None;
        list.at += 1;
    }
    true
}

fn tick_block(current: &mut Option<Box<Running>>, block: &Block, ctx: &mut Ctx<'_>) -> bool {
    if current.is_none() {
        match start(block, ctx) {
            Some(r) => *current = Some(Box::new(r)),
            None => return true,
        }
    }
    let running = current.as_mut().expect("just started");
    let skip = ctx.skip;
    match (&mut **running, block) {
        (Running::List(l), Block::Sequence(blocks)) => tick_list(l, blocks, ctx),
        (Running::Parallel(branches), Block::Parallel(lists)) => {
            let mut all = true;
            for ((l, done), blocks) in branches.iter_mut().zip(lists) {
                if !*done {
                    *done = tick_list(l, blocks, ctx);
                }
                all &= *done;
            }
            all
        }
        (
            Running::Branch(l, then),
            Block::Branch {
                then: t, otherwise, ..
            },
        ) => tick_list(l, if *then { t } else { otherwise }, ctx),
        (Running::Wait(left), _)
        | (
            Running::WaitFor {
                left: Some(left), ..
            },
            Block::WaitFor { .. },
        ) if skip || *left <= 1 => true,
        (Running::Wait(left), _) => {
            *left -= 1;
            false
        }
        (Running::WaitFor { since, left }, Block::WaitFor { on, .. }) => {
            let e = |r: u16| ctx.role(r);
            let h = match *on {
                Await::Signal(id) => Happening::Signal(id),
                Await::Arrived(r) => Happening::Arrived(e(r)),
                Await::ClipFinished(r) => Happening::ClipFinished(e(r)),
            };
            if skip || ctx.s.saw(h, *since) {
                return true;
            }
            if let Some(left) = left {
                *left -= 1;
            }
            false
        }
        (
            Running::Walk {
                since,
                yaw,
                to,
                turning,
            },
            Block::Walk { role, .. },
        ) => {
            let e = ctx.role(*role);
            if skip {
                ctx.issue(Command::Stop { entity: e });
                ctx.place(e, *to, *yaw);
                return true;
            }
            if !*turning {
                let arrived = if ctx.builtin() {
                    !ctx.s.motion.walking(e)
                } else {
                    ctx.s.saw(Happening::Arrived(e), *since)
                        || ctx.s.saw(Happening::Blocked(e), *since)
                };
                if !arrived {
                    return false;
                }
                ctx.issue(Command::Face {
                    entity: e,
                    yaw: *yaw,
                });
                *turning = true;
            }
            !(ctx.builtin() && ctx.s.motion.turning(e))
        }
        (Running::Turn { yaw }, Block::Face { role, .. }) => {
            let e = ctx.role(*role);
            if skip {
                let at = ctx.position(e);
                ctx.place(e, at, *yaw);
                return true;
            }
            !(ctx.builtin() && ctx.s.motion.turning(e))
        }
        (Running::Line { since }, Block::Line { advance, .. }) => {
            if skip {
                return true;
            }
            if ctx.s.saw(Happening::LineDone(LINE), *since) {
                return true;
            }
            let action = ctx
                .s
                .input
                .settings()
                .button(&ctx.s.settings.cutscene.advance_action);
            if Advance::from_u8(*advance) == Some(Advance::Tap)
                && action.is_some_and(|b| ctx.s.input.pressed(b))
            {
                ctx.issue(Command::AdvanceLine {});
            }
            false
        }
        (Running::Fade { to }, _) => {
            if skip {
                *ctx.final_fade = *to;
                return true;
            }
            ctx.s.screen.fade.done() && ctx.s.screen.fade.target() == *to
        }
        (running, block) => unreachable!("{running:?} does not run {block:?}"),
    }
}

/// Starts a block; `None` when it ended at once.
fn start(block: &Block, ctx: &mut Ctx<'_>) -> Option<Running> {
    let skip = ctx.skip;
    let since = ctx.s.happenings;
    match block {
        Block::Sequence(_) => Some(Running::List(List::default())),
        Block::Parallel(lists) => Some(Running::Parallel(
            lists.iter().map(|_| (List::default(), false)).collect(),
        )),
        Block::Branch { flag, .. } => {
            Some(Running::Branch(List::default(), ctx.s.store.flag(*flag)))
        }
        Block::Wait(t) => (!skip && *t > 0.0).then(|| Running::Wait(steps_of(*t) + 1)),
        Block::WaitFor { timeout, .. } => (!skip).then(|| Running::WaitFor {
            since,
            left: timeout.map(|t| steps_of(t) + 1),
        }),
        Block::Shot { shot, blend, ease } => {
            let r = |i: u16| ctx.role(i);
            let framing = match *shot {
                Shot::Follow {
                    subject,
                    distance,
                    pitch,
                    yaw,
                    height,
                    fov,
                } => Command::RigFollow {
                    rig: RIG,
                    subject: r(subject),
                    distance,
                    pitch,
                    yaw,
                    height,
                    fov,
                },
                Shot::Frame {
                    a,
                    b,
                    pitch,
                    yaw,
                    height,
                    fov,
                    margin,
                } => Command::RigFrame {
                    rig: RIG,
                    a: r(a),
                    b: r(b),
                    pitch,
                    yaw,
                    height,
                    fov,
                    margin,
                },
                Shot::Fixed {
                    position,
                    look_at,
                    fov,
                } => Command::RigFixed {
                    rig: RIG,
                    position,
                    look_at,
                    fov,
                },
            };
            ctx.issue(framing);
            ctx.issue(Command::CameraUse {
                rig: RIG,
                blend: if skip { 0.0 } else { *blend },
                ease: Ease::from_u8(*ease).unwrap_or(Ease::Linear),
            });
            None
        }
        Block::Walk {
            role,
            to,
            speed,
            face,
        } => {
            let e = ctx.role(*role);
            let to = Vec3::from_array(*to);
            let at = ctx.position(e);
            let gap = to - at;
            let yaw = face.unwrap_or(if gap.x * gap.x + gap.z * gap.z > 1e-8 {
                yaw_of(gap)
            } else {
                ctx.yaw(e)
            });
            if !skip {
                ctx.issue(Command::MoveTo {
                    entity: e,
                    target: to.to_array(),
                    speed: *speed,
                });
            }
            Some(Running::Walk {
                since,
                yaw,
                to,
                turning: false,
            })
        }
        Block::Clip {
            role,
            clip,
            mode,
            blend,
        } => {
            ctx.issue(Command::PlayClip {
                entity: ctx.role(*role),
                clip: *clip,
                mode: ClipMode::from_u8(*mode).unwrap_or(ClipMode::Once),
                blend: *blend,
                speed: 1.0,
            });
            None
        }
        Block::Face { role, toward } => {
            let e = ctx.role(*role);
            let yaw = match *toward {
                Toward::Yaw(y) => y,
                Toward::Role(t) => {
                    let gap = ctx.position(ctx.role(t)) - ctx.position(e);
                    if gap.x * gap.x + gap.z * gap.z > 1e-8 {
                        yaw_of(gap)
                    } else {
                        ctx.yaw(e)
                    }
                }
            };
            if !skip {
                ctx.issue(Command::Face { entity: e, yaw });
            }
            Some(Running::Turn { yaw })
        }
        Block::LookAt { role, target } => {
            let target = target.map_or(Entity::NONE, |t| ctx.role(t));
            ctx.issue(Command::LookAt {
                entity: ctx.role(*role),
                target,
            });
            None
        }
        Block::Line {
            speaker,
            text,
            voice,
            advance,
            duration,
        } => {
            if skip {
                return None;
            }
            ctx.issue(Command::Say {
                line: LINE,
                speaker: ctx.role(*speaker),
                text,
                voice: *voice,
                advance: Advance::from_u8(*advance).unwrap_or(Advance::Tap),
                duration: *duration,
            });
            Some(Running::Line {
                since: ctx.s.happenings,
            })
        }
        Block::Sound {
            sound,
            bus,
            at,
            volume,
        } => {
            if !skip {
                ctx.issue(Command::PlaySound {
                    sound: *sound,
                    bus: Bus::from_u8(*bus).unwrap_or(Bus::Effects),
                    emitter: at.map_or(Entity::NONE, |r| ctx.role(r)),
                    volume: *volume,
                });
            }
            None
        }
        Block::Music { music, fade } => {
            ctx.issue(match music {
                Some(m) => Command::PlayMusic {
                    music: *m,
                    fade: *fade,
                },
                None => Command::StopMusic { fade: *fade },
            });
            None
        }
        Block::Fade {
            to,
            duration,
            color,
        } => {
            if skip {
                *ctx.final_fade = *to;
                *ctx.final_color = *color;
                return None;
            }
            ctx.issue(Command::Fade {
                to: *to,
                duration: *duration,
                color: *color,
            });
            Some(Running::Fade { to: *to })
        }
        Block::Letterbox { to, duration } => {
            ctx.issue(Command::Letterbox {
                to: *to,
                duration: if skip { 0.0 } else { *duration },
            });
            None
        }
        Block::SetFlag { flag, on } => {
            ctx.issue(Command::SetFlag {
                flag: *flag,
                on: *on,
            });
            None
        }
        Block::Spawn { role, at } => {
            let e = ctx.role(*role);
            let actor = ctx.cutscene.roles[*role as usize].actor;
            ctx.issue(Command::Spawn {
                entity: e,
                kind: Kind::Actor,
                asset: actor,
            });
            ctx.place(e, Vec3::from_array(at.position), at.yaw);
            None
        }
        Block::Despawn { role } => {
            ctx.issue(Command::Despawn {
                entity: ctx.role(*role),
            });
            None
        }
        Block::Show { role } | Block::Hide { role } => {
            let visible = matches!(block, Block::Show { .. });
            ctx.issue(Command::SetVisible {
                entity: ctx.role(*role),
                visible,
            });
            None
        }
        Block::Teleport { role, to, .. } => {
            let e = ctx.role(*role);
            ctx.issue(Command::Teleport {
                entity: e,
                position: to.position,
                yaw: to.yaw,
                force: false,
            });
            None
        }
        Block::Call { call } => {
            let id = ctx.id;
            ctx.s.events.push(Event::CutsceneCall {
                cutscene: id,
                call: *call,
            });
            None
        }
    }
}
