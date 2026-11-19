// SPDX-License-Identifier: Apache-2.0

//! The collision part of `check`: every action spot must hold its actor's
//! collider clear of the room and over walkable ground, every route must have
//! a path, every traversal must end on free floor with its contact points on
//! a surface and within the actor's reach, and every actor's model should fit
//! its collider.

use glam::Vec3;
use sariema_format::collision::CollisionPack;
use sariema_physics::{
    ActorCollider, ActorId, LocalBounds, Settings, World, actor_collider, spot_actor, visual_bounds,
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
    /// Metres a declared contact point may lie off a surface, or inside one.
    pub contact_tolerance: f32,
}

impl Default for SpotCheckSettings {
    fn default() -> SpotCheckSettings {
        SpotCheckSettings {
            penetration_tolerance: 0.01,
            ground_depth: 0.05,
            bounds_margin: 0.05,
            contact_tolerance: 0.02,
        }
    }
}

/// `physics` are the game's settings of the collision module, which walks
/// the routes and judges the ground as the game will.
pub fn check_room(
    room: &str,
    pack: &CollisionPack,
    physics: &Settings,
    settings: &SpotCheckSettings,
    findings: &mut Vec<Finding>,
) -> Result<(), Failure> {
    let physics = physics.clone();
    let limits = Limits {
        max_slope_deg: physics.controller.max_slope_deg,
        cos_max_slope: physics.controller.max_slope_deg.to_radians().cos(),
        lift: physics.controller.step_height,
        settings: *settings,
    };
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
        let at = format!("room {room}: spot {}: {}", spot.name, body.name);
        match placement(
            &world,
            &actor_collider(body),
            spot.position.into(),
            spot.yaw,
            &limits,
        ) {
            Some(Misplaced::Inside(why)) => {
                findings.push(error("E_SPOT_BLOCKED", format!("{at}'s {why}")))
            }
            Some(Misplaced::NoGround(why)) => {
                findings.push(error("E_SPOT_NO_GROUND", format!("{at} {why}")))
            }
            None => {}
        }
    }

    for t in &pack.traversals {
        let from = &pack.spots[t.from as usize];
        let body = &pack.actors[from.actor as usize];
        let at = format!(
            "room {room}: traversal {}: at the end, {}",
            t.name, body.name
        );
        let end = (!t.hold_end)
            .then(|| {
                placement(
                    &world,
                    &actor_collider(body),
                    t.end().into(),
                    t.yaw,
                    &limits,
                )
            })
            .flatten();
        match end {
            Some(Misplaced::Inside(why)) => {
                findings.push(error("E_TRAVERSAL_END", format!("{at}'s {why}")))
            }
            Some(Misplaced::NoGround(why)) => {
                findings.push(error("E_TRAVERSAL_END", format!("{at} {why}")))
            }
            None => {}
        }
        let tolerance = settings.contact_tolerance;
        let (bounds, whose) = match visual_bounds(body) {
            Some(b) => (b, "visual bounds"),
            None => (actor_collider(body).bounds(), "collider bounds"),
        };
        let stands: Vec<Vec3> = std::iter::once(from.position)
            .chain(t.points.iter().copied())
            .map(Vec3::from)
            .collect();
        for contact in &t.contacts {
            let point = Vec3::from(contact.point);
            let off = match world.closest_point(point, tolerance) {
                None => Some(format!("is not within {tolerance} m of a surface")),
                Some(c) if c.distance < -tolerance => Some(format!(
                    "is {:.3} m inside {}",
                    -c.distance,
                    world.collider_name(c.collider)
                )),
                Some(_) => None,
            };
            if let Some(off) = off {
                findings.push(error(
                    "E_CONTACT_OFF_SURFACE",
                    format!(
                        "room {room}: traversal {}: contact {} {off}",
                        t.name, contact.name
                    ),
                ));
            }
            if !stands
                .iter()
                .any(|&feet| within_any_facing(bounds, point - feet, tolerance))
            {
                findings.push(error(
                    "E_CONTACT_OUT_OF_REACH",
                    format!(
                        "room {room}: traversal {}: contact {} lies outside {}'s {whose} at every point of the traversal",
                        t.name, contact.name, body.name
                    ),
                ));
            }
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

struct Limits {
    max_slope_deg: f32,
    cos_max_slope: f32,
    /// How far above a sunk placement to look for where it would rest.
    lift: f32,
    settings: SpotCheckSettings,
}

/// Why an actor cannot stand somewhere, ending a sentence about it.
enum Misplaced {
    /// Follows "<actor>'s".
    Inside(String),
    /// Follows "<actor>".
    NoGround(String),
}

/// Whether the actor's collider at `feet` is clear of the room and stands on
/// walkable ground.
fn placement(
    world: &World,
    collider: &ActorCollider,
    feet: Vec3,
    yaw: f32,
    limits: &Limits,
) -> Option<Misplaced> {
    let settings = &limits.settings;
    if let Some((c, separation)) = world.actor_overlap(collider, feet, yaw)
        && separation < -settings.penetration_tolerance
    {
        let clear = resting_height(world, collider, feet, yaw, limits)
            .map_or_else(String::new, |y| format!("; it stands clear at y {y:.3}"));
        return Some(Misplaced::Inside(format!(
            "collider is {:.3} m inside {}{clear}",
            -separation,
            world.collider_name(c)
        )));
    }
    let bottom = feet + Vec3::Y * collider.radius;
    let ground = world.shapecast(
        bottom,
        bottom,
        collider.radius,
        Vec3::NEG_Y * settings.ground_depth,
    );
    match ground {
        Some(hit) if hit.normal.y >= limits.cos_max_slope => None,
        Some(hit) => Some(Misplaced::NoGround(format!(
            "stands on {}, steeper than {}°",
            world.collider_name(hit.collider),
            limits.max_slope_deg
        ))),
        None => Some(Misplaced::NoGround(format!(
            "stands on nothing within {} m",
            settings.ground_depth
        ))),
    }
}

/// The feet height, within the lift above `feet`, where the collider dropped
/// straight down comes to rest clear of the room, as an actor settles on a
/// slope or a ledge; `None` when the overlap is not one a lift resolves.
fn resting_height(
    world: &World,
    collider: &ActorCollider,
    feet: Vec3,
    yaw: f32,
    limits: &Limits,
) -> Option<f32> {
    let (lift, settings) = (limits.lift, &limits.settings);
    let top = feet + Vec3::Y * lift;
    let (a, b) = (
        top + Vec3::Y * collider.radius,
        top + Vec3::Y * (collider.height - collider.radius),
    );
    let drop = lift + settings.ground_depth;
    let hit = world.shapecast(a, b, collider.radius, Vec3::NEG_Y * drop)?;
    let rest = Vec3::new(feet.x, top.y - hit.fraction * drop, feet.z);
    world
        .actor_overlap(collider, rest, yaw)
        .is_none_or(|(_, separation)| separation >= -settings.penetration_tolerance)
        .then_some(rest.y)
}

/// Whether `offset`, from the feet, lies within `bounds` turned to some
/// facing (the traversal does not say which), give or take `tolerance`.
fn within_any_facing(bounds: LocalBounds, offset: Vec3, tolerance: f32) -> bool {
    let radius = [bounds.min.x, bounds.max.x]
        .into_iter()
        .flat_map(|x| [bounds.min.z, bounds.max.z].map(|z| x.hypot(z)))
        .fold(0.0, f32::max);
    offset.x.hypot(offset.z) <= radius + tolerance
        && offset.y >= bounds.min.y - tolerance
        && offset.y <= bounds.max.y + tolerance
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
