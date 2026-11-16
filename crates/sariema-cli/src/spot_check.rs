// SPDX-License-Identifier: Apache-2.0

//! The collision part of `check`: every action spot must hold its actor's
//! collider clear of the room and over walkable ground, every route must have
//! a path, and every actor's model should fit its collider.

use glam::Vec3;
use sariema_format::collision::CollisionPack;
use sariema_physics::{
    ActorId, LocalBounds, Settings, World, actor_collider, spot_actor, visual_bounds,
};

use crate::check::Finding;
use crate::output::Failure;

#[derive(Clone, Copy, Debug)]
pub struct SpotCheckSettings {
    /// Metres a collider may sit inside the room at a spot, as a resting actor does within its skin.
    pub penetration_tolerance: f32,
    /// Metres below a spot's feet walkable ground must be.
    pub ground_depth: f32,
    /// Metres an actor's model may reach past its collider before a warning.
    pub bounds_margin: f32,
}

impl Default for SpotCheckSettings {
    fn default() -> SpotCheckSettings {
        SpotCheckSettings {
            penetration_tolerance: 0.01,
            ground_depth: 0.05,
            bounds_margin: 0.05,
        }
    }
}

pub fn check_room(
    room: &str,
    pack: &CollisionPack,
    settings: &SpotCheckSettings,
    findings: &mut Vec<Finding>,
) -> Result<(), Failure> {
    let physics = Settings::default();
    let max_slope_deg = physics.controller.max_slope_deg;
    let cos_max_slope = max_slope_deg.to_radians().cos();
    let mut world = World::from_pack(physics, pack)
        .map_err(|e| Failure::new("E_SCHEMA", format!("room {room}: {e}")))?;
    let error = |code, message| Finding {
        code,
        message,
        error: true,
    };

    for body in &pack.actors {
        let Some(model) = visual_bounds(body) else {
            continue;
        };
        let (excess, side) = past(actor_collider(body).bounds(), model);
        if excess > settings.bounds_margin {
            findings.push(Finding {
                code: "W_ACTOR_BOUNDS",
                message: format!(
                    "room {room}: actor {}: the model reaches {excess:.2} m past its collider at the {side} (margin {} m)",
                    body.name, settings.bounds_margin
                ),
                error: false,
            });
        }
    }

    for spot in &pack.spots {
        let body = &pack.actors[spot.actor as usize];
        let collider = actor_collider(body);
        let feet = Vec3::from(spot.position);
        let at = format!("room {room}: spot {}: {}", spot.name, body.name);
        if let Some((c, separation)) = world.actor_overlap(&collider, feet, spot.yaw)
            && separation < -settings.penetration_tolerance
        {
            findings.push(error(
                "E_SPOT_BLOCKED",
                format!(
                    "{at}'s collider is {:.3} m inside {}",
                    -separation,
                    world.collider_name(c)
                ),
            ));
            continue;
        }
        let bottom = feet + Vec3::Y * collider.radius;
        match world.shapecast(
            bottom,
            bottom,
            collider.radius,
            Vec3::NEG_Y * settings.ground_depth,
        ) {
            Some(hit) if hit.normal.y >= cos_max_slope => {}
            Some(hit) => findings.push(error(
                "E_SPOT_NO_GROUND",
                format!(
                    "{at} stands on {}, steeper than {}°",
                    world.collider_name(hit.collider),
                    max_slope_deg
                ),
            )),
            None => findings.push(error(
                "E_SPOT_NO_GROUND",
                format!("{at} stands on nothing within {} m", settings.ground_depth),
            )),
        }
    }

    let mut walkers: Vec<Option<ActorId>> = vec![None; pack.actors.len()];
    let mut path = Vec::new();
    for &[from, to] in &pack.routes {
        let (from, to) = (&pack.spots[from as usize], &pack.spots[to as usize]);
        let walker = &mut walkers[from.actor as usize];
        let id = match *walker {
            Some(id) => id,
            None => {
                let id = world
                    .add_actor(spot_actor(pack, from))
                    .map_err(|e| Failure::new("E_SCHEMA", format!("room {room}: {e}")))?;
                *walker = Some(id);
                id
            }
        };
        if !world.find_path(id, from.position.into(), to.position.into(), &mut path) {
            findings.push(error(
                "E_PATH_NONE",
                format!(
                    "room {room}: no path for {} from spot {} to spot {}",
                    pack.actors[from.actor as usize].name, from.name, to.name
                ),
            ));
        }
    }
    Ok(())
}

/// How far `model` reaches past `collider`, and on which side.
fn past(collider: LocalBounds, model: LocalBounds) -> (f32, &'static str) {
    let over_max = model.max - collider.max;
    let over_min = collider.min - model.min;
    [
        (over_max.x, "left"),
        (over_min.x, "right"),
        (over_max.y, "top"),
        (over_min.y, "bottom"),
        (over_max.z, "front"),
        (over_min.z, "back"),
    ]
    .into_iter()
    .fold((f32::NEG_INFINITY, ""), |worst, side| {
        if side.0 > worst.0 { side } else { worst }
    })
}
