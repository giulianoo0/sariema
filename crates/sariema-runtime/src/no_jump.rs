// SPDX-License-Identifier: Apache-2.0

//! The no-jump rule: no visible actor is drawn going faster than
//! `motion.max_visible_speed` or turning faster than `motion.turn_rate`
//! from one step to the next, unless the step is covered or cut or a forced
//! teleport placed it. [`measure`] holds it, once a step, on the poses the
//! frames will draw. The `may_*` checks refuse, when it is given, a command
//! that surely breaks it, so the game hears of it with the command's own
//! opcode and nothing changes.

use glam::Vec3;
use sariema_core::STEP_SECONDS;
use sariema_format::script::{BlockReason, Ease, Entity, Event, Kind, Property};

use crate::apply::Refusal;
use crate::ease::ease;
use crate::events::Happening;
use crate::follow;
use crate::motion::Goal;
use crate::outbox::PhysicsIntent;
use crate::state::State;
use crate::tween::{self, Target, steps_of};
use crate::world::{Limits, Pose, Reject};

/// Room for the rounding of composed placements when measuring a step.
const SLACK: f32 = 1.0 + 1e-4;

/// The farthest a visible actor may go in one step.
fn step_limit(s: &State) -> f32 {
    s.settings.motion.max_visible_speed * STEP_SECONDS
}

fn turn_limit(s: &State) -> f32 {
    s.settings.motion.turn_rate * STEP_SECONDS
}

/// Once every system has moved things this step and the world is resolved:
/// each visible actor that went past the limits since the previous step is
/// put back where it was, unless the screen was fully faded when the step
/// began or ended, the camera cut in it, or a forced teleport placed the
/// actor. Parents go first, so a child is judged after what carries it was
/// put back. Then the world settles where the frames draw each entity from.
pub(crate) fn measure(s: &mut State) {
    let limits = Limits::new(step_limit(s) * SLACK, turn_limit(s) * SLACK);
    let exempt = s.screen.covered() || s.screen.covered_before() || s.camera.cut();
    if !exempt {
        let (mut from, mut held) = (0, false);
        while let Some((at, e)) = s.world.next_jump(from, limits, held) {
            let moved = s.world.went_past(e, limits);
            put_back(s, e, limits, moved);
            (from, held) = (at + 1, true);
        }
        if held {
            s.world.resolve();
        }
    }
    s.world.settle(limits, exempt);
}

/// Puts `e` back where it was. When this step's motion carried it off
/// (`moved`, rather than putting back what carries it), what carried it
/// stops: each level of it (itself, then each carrier up) is judged by how
/// far its own change this step carries `e`; the levels that alone go past
/// the limits stop, and when none does, every level that changed stops, if
/// together they do. `rejected` (opcode 0) names `e`, then each carrier
/// stopped.
fn put_back(s: &mut State, e: Entity, limits: Limits, moved: bool) {
    s.events.push(rejected(e));
    if moved {
        stop_what_carried(s, e, limits);
    }
    s.world.hold(e, s.settings.motion.min_carrier_scale);
    let local = s.world.node(e).expect("a held actor is alive").local;
    s.outbox.physics.push(PhysicsIntent::Teleport {
        entity: e,
        position: local.position,
        yaw: local.yaw,
    });
}

fn stop_what_carried(s: &mut State, e: Entity, limits: Limits) {
    let (mut alone, mut sum) = (false, (0.0, 0.0));
    let mut level = e;
    while let Some(node) = s.world.node(level) {
        let (metres, degrees) = s.world.carried_by(e, level);
        alone |= limits.passed((metres, degrees));
        sum = (sum.0 + metres, sum.1 + degrees);
        level = node.parent;
    }
    let together = !alone && limits.passed(sum);
    let mut level = e;
    while let Some(node) = s.world.node(level) {
        let parent = node.parent;
        let blamed = if alone {
            limits.passed(s.world.carried_by(e, level))
        } else {
            together && s.world.changed(level)
        };
        if blamed && stop(s, level) && level != e {
            s.events.push(rejected(level));
        }
        level = parent;
    }
}

fn rejected(entity: Entity) -> Event<'static> {
    Event::Rejected {
        opcode: 0,
        code: Reject::TeleportVisible.code(),
        entity,
    }
}

/// Stops `e`'s own motion: tweens and springs of its placement, its follow,
/// its walk, which ends with `blocked`. Whether there was any.
fn stop(s: &mut State, e: Entity) -> bool {
    let mut stopped = s.tweens.forget(e);
    stopped |= s.followers.unfollow(e);
    if s.motion.get(e).is_some_and(|m| m.goal != Goal::Idle) {
        s.motion.mover_mut(e).goal = Goal::Idle;
        s.outbox.physics.push(PhysicsIntent::Stop { entity: e });
        s.events.push(Event::Blocked {
            entity: e,
            reason: BlockReason::Stopped,
        });
        s.happen(Happening::Blocked(e));
        stopped = true;
    }
    stopped
}

/// The early refusal. `moved` is how far a command takes `e` in its next
/// step, infinite for a placement. Refused, on a screen neither fully faded
/// nor cut, when it would carry past `motion.max_visible_speed` an actor
/// visible when the step began that no forced teleport placed: `e`, a child
/// of it at any depth (going as far), or a follower (going its gain of it).
fn may_jump(s: &State, e: Entity, moved: f32) -> Result<(), Refusal> {
    if moved <= step_limit(s) * SLACK || s.screen.covered() || s.camera.cut() {
        return Ok(());
    }
    let mut budget = s.world.len() as usize + s.followers.len();
    if seen_moving(s, e, moved, &mut budget) {
        return Err(Refusal {
            code: Reject::TeleportVisible.code(),
            entity: e,
        });
    }
    Ok(())
}

/// `budget` bounds the walk, which a cycle of followers would not end.
fn seen_moving(s: &State, e: Entity, moved: f32, budget: &mut usize) -> bool {
    if *budget == 0 || moved <= step_limit(s) * SLACK {
        return false;
    }
    *budget -= 1;
    let Some(node) = s.world.node(e) else {
        return false;
    };
    if node.kind == Kind::Actor && s.world.was_visible(e) && !s.world.forced(e) {
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

/// Placing `e` at local `position` and `yaw`: always too far for an actor,
/// since placing an actor is never a walk.
pub(crate) fn may_place(s: &State, e: Entity, position: Vec3, yaw: f32) -> Result<(), Refusal> {
    let Some(node) = s.world.node(e) else {
        return Ok(());
    };
    let turn = ((yaw - node.local.yaw + 180.0).rem_euclid(360.0) - 180.0).abs();
    let moved = if node.kind == Kind::Actor || turn > turn_limit(s) {
        f32::INFINITY
    } else {
        let scale = s.world.current_pose(node.parent).map_or(1.0, |p| p.scale);
        node.local.position.distance(position) * scale
    };
    may_jump(s, e, moved)
}

/// Giving `e` the parent `parent`, which moves it as far as its world
/// placement changes.
pub(crate) fn may_reparent(s: &State, e: Entity, parent: Entity) -> Result<(), Refusal> {
    match (s.world.current_pose(e), s.world.pose_under(e, parent)) {
        (Some(a), Some(b)) => may_jump(s, e, step_move(s, a, b)),
        _ => Ok(()),
    }
}

/// A tween of `t` to `to`, by its fastest step.
pub(crate) fn may_tween(
    s: &State,
    t: &Target,
    to: f32,
    duration: f32,
    curve: Ease,
) -> Result<(), Refusal> {
    let Ok(from) = tween::read(&s.world, &s.store, &s.scenery, t) else {
        return Ok(());
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
    let change = most * (to - from).abs();
    let moved = match t.property {
        Property::PositionX | Property::PositionY | Property::PositionZ => change,
        Property::Yaw | Property::Pitch | Property::Roll if change > turn_limit(s) => f32::INFINITY,
        _ => 0.0,
    };
    may_jump(s, t.entity, moved)
}

/// `e` following `target` plus `offset` with `lag`, by its first step.
pub(crate) fn may_follow(
    s: &State,
    e: Entity,
    target: Entity,
    offset: Vec3,
    lag: f32,
) -> Result<(), Refusal> {
    let moved = match (s.world.current_pose(e), s.world.current_pose(target)) {
        (Some(me), Some(to)) => me.position.distance(to.position + offset) * follow::gain(lag),
        _ => 0.0,
    };
    may_jump(s, e, moved)
}

/// `e`'s root moved along a clip path, by its farthest step.
pub(crate) fn may_path(s: &State, e: Entity, moved: f32) -> Result<(), Refusal> {
    may_jump(s, e, moved)
}

/// The most a visible actor may turn in a step, in degrees.
pub(crate) fn turn_step(s: &State) -> f32 {
    turn_limit(s) * SLACK
}

/// `e` walking to `goal`, a velocity at its speed scale.
pub(crate) fn may_walk(s: &State, e: Entity, goal: Goal) -> Result<(), Refusal> {
    may_jump(s, e, walk_move(s, e, goal, speed_scale(s, e)))
}

/// Scaling `e`'s speed by `scale`, for its walk and, if it is the hero, the
/// player's axis.
pub(crate) fn may_rescale(s: &State, e: Entity, scale: f32) -> Result<(), Refusal> {
    let goal = s.motion.get(e).map_or(Goal::Idle, |m| m.goal);
    let mut moved = walk_move(s, e, goal, scale);
    if s.control.hero == e {
        let hero = Goal::Velocity(Vec3::X * s.control.speed);
        moved = moved.max(walk_move(s, e, hero, scale));
    }
    may_jump(s, e, moved)
}

fn speed_scale(s: &State, e: Entity) -> f32 {
    s.motion.get(e).map_or(1.0, |m| m.speed_scale)
}

/// How far `e` going for `goal` goes in one step: a velocity scaled by
/// `scale`, a walk to a point at its own speed in world units.
fn walk_move(s: &State, e: Entity, goal: Goal, scale: f32) -> f32 {
    match goal {
        Goal::Idle => 0.0,
        Goal::Velocity(v) => v.length() * scale.abs() * STEP_SECONDS,
        Goal::To { target, speed } => {
            let at = s.world.node(e).map_or(target, |n| n.local.position);
            (speed.abs() * STEP_SECONDS).min(at.distance(target))
        }
    }
}
