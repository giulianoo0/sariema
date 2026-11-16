// SPDX-License-Identifier: Apache-2.0

//! Movement intents and the built-in kinematic mover that carries them out
//! when no collision module does: straight lines at a capped speed, turning
//! at a capped rate, arriving exactly on the target. Nothing snaps.

use glam::Vec3;
use sariema_core::STEP_SECONDS;
use serde::{Deserialize, Serialize};

use crate::world::{Entity, World};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MotionSettings {
    /// Whether the runtime moves actors itself. Off when a collision module
    /// consumes the intents and reports `arrived`.
    pub builtin: bool,
    /// Degrees per second an actor turns.
    pub turn_rate: f32,
    /// Metres from the target that count as arrived.
    pub arrive_distance: f32,
    /// The hero's move axis turns with the camera's view.
    pub camera_relative: bool,
    /// Metres per second a visible actor may move; faster is a jump.
    pub max_visible_speed: f32,
    /// A carrier scaled below this cannot hold an actor put back under it.
    pub min_carrier_scale: f32,
}

impl Default for MotionSettings {
    fn default() -> MotionSettings {
        MotionSettings {
            builtin: true,
            turn_rate: 540.0,
            arrive_distance: 0.01,
            camera_relative: true,
            max_visible_speed: 20.0,
            min_carrier_scale: 1e-6,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    Idle,
    /// m/s, held until changed.
    Velocity(Vec3),
    To {
        target: Vec3,
        speed: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mover {
    pub goal: Goal,
    /// Yaw in degrees to turn to.
    pub face: Option<f32>,
    pub speed_scale: f32,
}

const IDLE: Mover = Mover {
    goal: Goal::Idle,
    face: None,
    speed_scale: 1.0,
};

/// Degrees about +Y of a horizontal direction; 0 is −Z.
pub fn yaw_of(d: Vec3) -> f32 {
    libm::atan2f(-d.x, -d.z).to_degrees()
}

/// Signed shortest turn from `from` to `to`, in (−180, 180].
pub fn angle_between(from: f32, to: f32) -> f32 {
    let d = (to - from).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

#[derive(Clone, Debug, Default)]
pub struct Motion {
    movers: Vec<Option<(Entity, Mover)>>,
}

impl Motion {
    pub fn mover_mut(&mut self, e: Entity) -> &mut Mover {
        let i = e.index as usize;
        if i >= self.movers.len() {
            self.movers.resize(i + 1, None);
        }
        let slot = &mut self.movers[i];
        if slot.is_none_or(|(owner, _)| owner != e) {
            *slot = Some((e, IDLE));
        }
        &mut slot.as_mut().expect("just set").1
    }

    pub fn get(&self, e: Entity) -> Option<&Mover> {
        match self.movers.get(e.index as usize) {
            Some(Some((owner, m))) if *owner == e => Some(m),
            _ => None,
        }
    }

    pub fn forget(&mut self, e: Entity) {
        if let Some(slot) = self.movers.get_mut(e.index as usize)
            && slot.is_some_and(|(owner, _)| owner == e)
        {
            *slot = None;
        }
    }

    pub fn turning(&self, e: Entity) -> bool {
        self.get(e).is_some_and(|m| m.face.is_some())
    }

    pub fn walking(&self, e: Entity) -> bool {
        self.get(e)
            .is_some_and(|m| matches!(m.goal, Goal::To { .. }))
    }

    /// One step of the built-in mover; `arrived` receives each actor that
    /// reached its target this step.
    pub fn step(
        &mut self,
        world: &mut World,
        settings: &MotionSettings,
        mut arrived: impl FnMut(Entity),
    ) {
        let max_turn = settings.turn_rate * STEP_SECONDS;
        for slot in &mut self.movers {
            let Some((e, m)) = slot else { continue };
            let e = *e;
            let Ok(node) = world.node_mut(e) else {
                *slot = None;
                continue;
            };
            let local = &mut node.local;
            match m.goal {
                Goal::Idle => {}
                Goal::Velocity(v) => {
                    let v = Vec3::new(v.x, 0.0, v.z) * m.speed_scale;
                    local.position += v * STEP_SECONDS;
                    if v.length_squared() > 1e-8 {
                        m.face = Some(yaw_of(v));
                    }
                }
                Goal::To { target, speed } => {
                    let gap = target - local.position;
                    let distance = gap.length();
                    let reach = speed * m.speed_scale * STEP_SECONDS;
                    if distance <= reach.max(settings.arrive_distance) {
                        local.position = target;
                        m.goal = Goal::Idle;
                        arrived(e);
                    } else {
                        local.position += gap / distance * reach;
                        if gap.x * gap.x + gap.z * gap.z > 1e-8 {
                            m.face = Some(yaw_of(gap));
                        }
                    }
                }
            }
            if let Some(face) = m.face {
                let turn = angle_between(local.yaw, face);
                if turn.abs() <= max_turn {
                    local.yaw = face;
                    m.face = None;
                } else {
                    local.yaw = (local.yaw + turn.signum() * max_turn).rem_euclid(360.0);
                }
            }
        }
    }
}
