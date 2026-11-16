// SPDX-License-Identifier: Apache-2.0

//! One actor's move through the static world in one step: sweep and slide,
//! stepping up ledges, staying on the ground, sliding along walls and being
//! helped round corners.
//!
//! Approach after Rapier's `KinematicCharacterController` (step-up only on
//! walls, ground snap only when the move started on the ground) and Godot's
//! `CharacterBody3D::move_and_slide` (walls block only horizontally while on
//! the floor, constant horizontal speed on slopes, stop in a crease between
//! two walls).

use glam::Vec3;

use crate::actor::{ActorCollider, Probe};
use crate::math::flat;
use crate::scene::{Filter, Scene};
use crate::settings::Settings;

/// Values derived once from [`Settings`] so a step does no trigonometry.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Derived {
    /// Ground whose normal has at least this `y` is walkable.
    pub cos_max_slope: f32,
    /// Sine of the corner-assist angle: input whose slide keeps less than
    /// this fraction of its length is pushing straight into the wall.
    pub head_on: f32,
    /// Slides keeping at least this fraction get the full preservation.
    pub full_slide: f32,
    /// Share of the remaining angle the facing covers in one step.
    pub yaw_blend: f32,
}

impl Derived {
    pub fn new(s: &Settings) -> Derived {
        let c = &s.controller;
        Derived {
            cos_max_slope: libm::cosf(c.max_slope_deg.to_radians()),
            head_on: libm::sinf(c.corner_assist_angle_deg.to_radians()),
            full_slide: libm::cosf(c.wall_slide_full_angle_deg.to_radians()),
            yaw_blend: 1.0 - libm::expf(-c.yaw_rate * s.step_seconds),
        }
    }
}

pub(crate) struct Mover<'a> {
    pub scene: &'a Scene,
    pub settings: &'a Settings,
    pub derived: &'a Derived,
    pub collider: &'a ActorCollider,
    pub yaw: f32,
    pub filter: Filter,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ground {
    pub normal: Vec3,
    pub collider: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Moved {
    pub position: Vec3,
    /// Walkable ground touched during the move.
    pub ground: Option<Ground>,
    /// First wall pressed against: collider and normal.
    pub wall: Option<(u32, Vec3)>,
    /// The move ended an upward motion against a ceiling.
    pub ceiling: bool,
}

/// Squared length below which a motion is nothing.
const NOTHING: f32 = 1e-12;
/// A normal pointing at least this far down is a ceiling.
const CEILING: f32 = -1e-3;

/// `h` (horizontal) redirected along ground of normal `n`, keeping its horizontal part.
fn along_ground(h: Vec3, n: Vec3) -> Vec3 {
    Vec3::new(h.x, -(h.x * n.x + h.z * n.z) / n.y, h.z)
}

/// `h` without its part pointing into a wall of horizontal normal `n`.
fn off_wall(h: Vec3, n: Vec3) -> Vec3 {
    let into = h.dot(n);
    if into < 0.0 { h - n * into } else { h }
}

impl Mover<'_> {
    fn probe(&self, feet: Vec3) -> Probe {
        self.collider.probe(feet, self.yaw, 0.0)
    }

    fn walkable(&self, n: Vec3) -> bool {
        n.y >= self.derived.cos_max_slope
    }

    /// Moves from `start` by the horizontal `h` and, off the ground, the
    /// vertical `v`. Standing on `ground`, the vertical part follows it.
    /// `assist` enables wall-slide preservation and corner assist, which
    /// belong to stick and scripted motion, not to pushes.
    pub fn walk(
        &self,
        start: Vec3,
        mut h: Vec3,
        mut v: f32,
        ground: Option<Ground>,
        assist: bool,
    ) -> Moved {
        let skin = self.settings.collision.skin;
        let mut on = ground;
        let mut out = Moved {
            position: start,
            ground: None,
            wall: None,
            ceiling: false,
        };
        let mut walls = [Vec3::ZERO; 8];
        let mut wall_count = 0;
        let mut helped = !assist;
        for _ in 0..self.settings.controller.slide_iterations {
            let motion = match on {
                Some(g) => along_ground(h, g.normal),
                None => h + Vec3::Y * v,
            };
            if motion.length_squared() <= NOTHING {
                break;
            }
            let Some(hit) = self
                .scene
                .sweep(&self.probe(out.position), motion, self.filter)
            else {
                out.position += motion;
                break;
            };
            let len = motion.length();
            let dir = motion / len;
            let closing = -dir.dot(hit.normal);
            let back = if closing > 1e-6 { skin / closing } else { len };
            let travel = (hit.t * len - back).max(0.0);
            out.position += dir * travel;
            let left = 1.0 - travel / len;
            h *= left;
            v *= left;
            let n = hit.normal;
            if self.walkable(n) {
                on = Some(Ground {
                    normal: n,
                    collider: hit.collider,
                });
                out.ground = on;
                v = 0.0;
                continue;
            }
            if n.y <= CEILING {
                out.ceiling |= v > 0.0;
                v = v.min(0.0);
                let nf = flat(n).normalize_or_zero();
                if nf == Vec3::ZERO && on.is_some() {
                    break;
                }
                h = off_wall(h, nf);
                continue;
            }
            if on.is_some()
                && let Some(stepped) = self.step_up(out.position, h)
            {
                out.position = stepped.0;
                out.ground = Some(stepped.1);
                break;
            }
            let nf = flat(n).normalize_or_zero();
            out.wall.get_or_insert((hit.collider, n));
            let pressed = h;
            h = off_wall(h, nf);
            if !helped {
                helped = true;
                h = self.wall_response(out.position, pressed, h, nf);
            }
            if walls[..wall_count].iter().any(|w| h.dot(*w) < -1e-6) {
                h = Vec3::ZERO;
            }
            if wall_count < walls.len() {
                walls[wall_count] = nf;
                wall_count += 1;
            }
            if on.is_none() && v < 0.0 {
                let down = Vec3::Y * v;
                let slid = down - n * down.dot(n);
                h += flat(slid);
                v = slid.y.min(0.0);
            }
        }
        out
    }

    /// Keeps more speed than the plain projection when the input meets the
    /// wall at a shallow angle, and looks for a corner to slip round when it
    /// pushes straight in.
    fn wall_response(&self, at: Vec3, pressed: Vec3, slid: Vec3, nf: Vec3) -> Vec3 {
        let len = pressed.length();
        if len * len <= NOTHING {
            return slid;
        }
        let kept = slid.length() / len;
        let d = self.derived;
        if kept < d.head_on {
            return self.corner_assist(at, pressed, nf).unwrap_or(slid);
        }
        let ramp = ((kept - d.head_on) / (d.full_slide - d.head_on).max(1e-6)).clamp(0.0, 1.0);
        let f = kept.max(self.settings.controller.wall_slide_preservation * ramp);
        slid.normalize_or_zero() * (len * f)
    }

    /// A sideways motion towards the nearer free side of a convex corner
    /// within reach, if there is one.
    fn corner_assist(&self, at: Vec3, pressed: Vec3, nf: Vec3) -> Option<Vec3> {
        let c = &self.settings.controller;
        let len = pressed.length();
        let dir = pressed / len;
        let tangent = Vec3::Y.cross(nf);
        let prefer = if pressed.dot(tangent) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let probes = c.corner_assist_probes.max(1);
        for k in 1..=probes {
            let offset = c.corner_assist_distance * k as f32 / probes as f32;
            for side in [prefer, -prefer] {
                let lateral = tangent * (side * offset);
                if self
                    .scene
                    .sweep(&self.probe(at), lateral, self.filter)
                    .is_some()
                {
                    continue;
                }
                if self
                    .scene
                    .sweep(&self.probe(at + lateral), dir * len, self.filter)
                    .is_some()
                {
                    continue;
                }
                return Some(tangent * (side * (c.corner_assist_speed * len).min(offset)));
            }
        }
        None
    }

    /// Up by at most the step height, across by `h`, then down onto ground
    /// that supports the actor.
    fn step_up(&self, at: Vec3, h: Vec3) -> Option<(Vec3, Ground)> {
        let skin = self.settings.collision.skin;
        let len = h.length();
        if len * len <= NOTHING {
            return None;
        }
        let rise_max = self.settings.controller.step_height - skin;
        if rise_max <= skin {
            return None;
        }
        let up = Vec3::Y * rise_max;
        let rise = match self.scene.sweep(&self.probe(at), up, self.filter) {
            None => rise_max,
            Some(hit) => (hit.t * rise_max - skin).max(0.0),
        };
        if rise <= skin {
            return None;
        }
        let raised = at + Vec3::Y * rise;
        if self
            .scene
            .sweep(&self.probe(raised), h, self.filter)
            .is_some()
        {
            return None;
        }
        let ahead = raised + h;
        let drop = rise + 2.0 * skin;
        let down = self
            .scene
            .sweep(&self.probe(ahead), Vec3::NEG_Y * drop, self.filter)?;
        let (normal, floor) = self.support(down.point, h / len, down.normal)?;
        let landed = ahead - Vec3::Y * (down.t * drop - skin).max(0.0);
        let climb = floor - (at.y - skin);
        (landed.y >= at.y - skin && climb <= self.settings.controller.step_height).then_some((
            landed,
            Ground {
                normal,
                collider: down.collider,
            },
        ))
    }

    /// The walkable normal and height of the surface under a downward
    /// contact at `point`: the contact's own, or for a contact on an edge,
    /// the surface found just past the edge in direction `toward`.
    fn support(&self, point: Vec3, toward: Vec3, normal: Vec3) -> Option<(Vec3, f32)> {
        if self.walkable(normal) {
            return Some((normal, point.y));
        }
        let skin = self.settings.collision.skin;
        let from = point + flat(toward).normalize_or_zero() * skin + Vec3::Y * skin;
        let ray = self
            .scene
            .raycast(from, Vec3::NEG_Y, 3.0 * skin, self.filter)?;
        self.walkable(ray.normal)
            .then_some((ray.normal, from.y - ray.distance))
    }

    /// Ground within the snap distance below `at` that supports the actor,
    /// and where the actor rests on it.
    pub fn snap(&self, at: Vec3) -> Option<(Vec3, Ground)> {
        let skin = self.settings.collision.skin;
        let reach = self.settings.controller.ground_snap;
        let hit = self
            .scene
            .sweep(&self.probe(at), Vec3::NEG_Y * reach, self.filter)?;
        let (normal, _) = self.support(hit.point, flat(at - hit.point), hit.normal)?;
        let gap = if hit.t > 0.0 {
            hit.t * reach
        } else {
            hit.start_separation
        };
        let rest = at - Vec3::Y * (gap - skin);
        Some((
            rest,
            Ground {
                normal,
                collider: hit.collider,
            },
        ))
    }

    /// Moves by `motion` sliding along whatever it meets, with no ground
    /// rules: platform carry and pushes between actors.
    pub fn push(&self, start: Vec3, mut motion: Vec3) -> Vec3 {
        let skin = self.settings.collision.skin;
        let mut at = start;
        for _ in 0..self.settings.controller.slide_iterations {
            if motion.length_squared() <= NOTHING {
                break;
            }
            let Some(hit) = self.scene.sweep(&self.probe(at), motion, self.filter) else {
                return at + motion;
            };
            let len = motion.length();
            let dir = motion / len;
            let closing = -dir.dot(hit.normal);
            let back = if closing > 1e-6 { skin / closing } else { len };
            let travel = (hit.t * len - back).max(0.0);
            at += dir * travel;
            motion *= 1.0 - travel / len;
            motion -= hit.normal * motion.dot(hit.normal).min(0.0);
        }
        at
    }

    /// The actor at `at` pushed out of what `of` sees it overlap, level only
    /// when `level`, each push a sweep against everything so the body never
    /// passes through anything: `at` itself when `of` sees no overlap; `None`
    /// when the pushes leave it inside what `of` sees or exceed the
    /// depenetration limits. Overlaps `of` does not see are left as they were.
    pub fn push_out(&self, at: Vec3, of: Filter, level: bool) -> Option<Vec3> {
        let c = &self.settings.collision;
        let mut pos = at;
        let mut moved = 0.0;
        for _ in 0..c.depenetration_iterations {
            let Some((_, deepest)) = self.scene.deepest(&self.probe(pos), of) else {
                break;
            };
            let n = if level {
                flat(deepest.normal)
            } else {
                deepest.normal
            };
            let reach = n.length();
            let distance = (c.skin - deepest.separation) / reach;
            moved += distance;
            if moved > c.max_depenetration {
                return None;
            }
            pos = self.push(pos, n / reach * distance);
        }
        self.scene
            .deepest(&self.probe(pos), of)
            .is_none()
            .then_some(pos)
    }

    /// Pushes the actor at `at` out of everything it overlaps; `None` when
    /// that takes more passes or distance than the settings allow.
    pub fn depenetrate(&self, at: Vec3) -> Option<Vec3> {
        let c = &self.settings.collision;
        let mut pos = at;
        let mut moved = 0.0;
        for _ in 0..c.depenetration_iterations {
            let Some((_, deepest)) = self.scene.deepest(&self.probe(pos), self.filter) else {
                return Some(pos);
            };
            let push = deepest.normal * (c.skin - deepest.separation);
            moved += push.length();
            if moved > c.max_depenetration {
                return None;
            }
            pos += push;
        }
        self.scene
            .deepest(&self.probe(pos), self.filter)
            .is_none()
            .then_some(pos)
    }
}
