// SPDX-License-Identifier: Apache-2.0

//! Contact and sweep between a rounded segment (the query: a capsule, a
//! sphere, or a point with radius 0) and one part of a collider: a triangle or
//! another rounded segment.
//!
//! Sweeps use conservative advancement on the exact distance. The distance
//! between two convex shapes, one translating linearly, is a convex function
//! of time, so each Newton step `t += gap / closing_speed` lands at or before
//! the first contact, never past it, and a closing speed of zero or less
//! means the gap never shrinks again (Mirtich, "Impulse-based dynamic
//! simulation of rigid body systems", 1996, for the advancement bound).

use glam::Vec3;

use crate::geom::{closest_segment_triangle, closest_segments};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tri {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    /// Unit face normal, `(b − a) × (c − a)` normalised.
    pub n: Vec3,
}

impl Tri {
    /// `None` when the three points are (nearly) on one line.
    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> Option<Tri> {
        let n = (b - a).cross(c - a);
        let len = n.length();
        (len > 1e-10).then(|| Tri {
            a,
            b,
            c,
            n: n / len,
        })
    }
}

/// The points within `r` of segment `ab`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Round {
    pub a: Vec3,
    pub b: Vec3,
    pub r: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Tri(Tri),
    Round(Round),
}

/// How close two shapes are. `normal` points from the part to the query and
/// `point` is on the part's surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    /// Gap between the surfaces; negative when they overlap.
    pub separation: f32,
    pub normal: Vec3,
    pub point: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    /// Fraction of the motion travelled before contact.
    pub t: f32,
    pub normal: Vec3,
    pub point: Vec3,
    /// Separation at the start of the motion.
    pub start_separation: f32,
}

/// Tolerances for [`sweep`], from [`crate::CollisionSettings`].
#[derive(Clone, Copy, Debug)]
pub struct Tolerance {
    pub contact: f32,
    pub sweep: f32,
    pub iterations: u32,
    /// A touching query is stopped only if its motion points into the part
    /// by more than this cosine; below it, rounding in the normal is taken
    /// for sliding along.
    pub tangent: f32,
    /// Cosine within which a triangle contact takes the face normal; see [`contact`].
    pub flat: f32,
}

/// Contact of the query `pq` (radius `r`) with `part`. A triangle contact
/// whose normal is within `tol_flat` (a cosine) of the face normal takes the
/// face normal: near a seam or vertex of a flat surface the offset to the
/// closest edge point is the face normal up to rounding, and that rounding
/// would otherwise tilt slides from step to step.
pub fn contact(p: Vec3, q: Vec3, r: f32, part: &Part, tol_flat: f32) -> Contact {
    let (on_query, on_part, part_r) = match part {
        Part::Tri(t) => {
            let (s, x) = closest_segment_triangle(p, q, t.a, t.b, t.c, t.n);
            (s, x, 0.0)
        }
        Part::Round(o) => {
            let (s, x) = closest_segments(p, q, o.a, o.b);
            (s, x, o.r)
        }
    };
    let diff = on_query - on_part;
    let d = diff.length();
    if d > 1e-9 {
        let mut normal = diff / d;
        if let Part::Tri(t) = part {
            let along = normal.dot(t.n);
            if along.abs() >= tol_flat {
                normal = if along < 0.0 { -t.n } else { t.n };
            }
        }
        return Contact {
            separation: d - r - part_r,
            normal,
            point: on_part + normal * part_r,
        };
    }
    match part {
        Part::Tri(t) => {
            let side = ((p + q) * 0.5 - t.a).dot(t.n);
            let normal = if side < 0.0 { -t.n } else { t.n };
            let deepest = (p - t.a).dot(normal).min((q - t.a).dot(normal)).min(0.0);
            Contact {
                separation: deepest - r,
                normal,
                point: on_part,
            }
        }
        Part::Round(o) => Contact {
            separation: -r - o.r,
            normal: Vec3::Y,
            point: on_part + Vec3::Y * o.r,
        },
    }
}

/// First contact of the query `pq` (radius `r`) moving by `motion` with
/// `part`. A query that starts within `tol.contact` of the part hits at
/// `t = 0` if it moves into it and passes otherwise, so a query can always
/// slide along or leave what it touches.
pub fn sweep(p: Vec3, q: Vec3, r: f32, motion: Vec3, part: &Part, tol: Tolerance) -> Option<Sweep> {
    if let Part::Tri(t) = part {
        let reach = r + tol.contact;
        let (sp, sq) = ((p - t.a).dot(t.n), (q - t.a).dot(t.n));
        let m = motion.dot(t.n);
        let (lo, hi) = (sp.min(sq), sp.max(sq));
        if (lo > reach && lo + m > reach) || (hi < -reach && hi + m < -reach) {
            return None;
        }
    }
    let first = contact(p, q, r, part, tol.flat);
    if first.separation < tol.contact {
        return (motion.dot(first.normal) < -tol.tangent * motion.length()).then_some(Sweep {
            t: 0.0,
            normal: first.normal,
            point: first.point,
            start_separation: first.separation,
        });
    }
    let mut t = 0.0;
    let mut c = first;
    for _ in 0..tol.iterations {
        if c.separation <= tol.sweep {
            break;
        }
        let closing = -motion.dot(c.normal);
        if closing <= 0.0 {
            return None;
        }
        t += c.separation / closing;
        if t > 1.0 {
            return None;
        }
        let off = motion * t;
        c = contact(p + off, q + off, r, part, tol.flat);
    }
    Some(Sweep {
        t,
        normal: c.normal,
        point: c.point,
        start_separation: first.separation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: Tolerance = Tolerance {
        contact: 1e-4,
        sweep: 1e-6,
        iterations: 32,
        tangent: 1e-4,
        flat: 0.99996,
    };

    fn floor() -> Part {
        Part::Tri(
            Tri::new(
                Vec3::new(-5.0, 0.0, -5.0),
                Vec3::new(0.0, 0.0, 5.0),
                Vec3::new(5.0, 0.0, -5.0),
            )
            .unwrap(),
        )
    }

    #[test]
    fn sphere_falls_onto_a_triangle_face() {
        let p = Vec3::new(0.0, 2.0, 0.0);
        let hit = sweep(p, p, 0.5, Vec3::new(0.0, -4.0, 0.0), &floor(), TOL).unwrap();
        assert!((hit.t - 1.5 / 4.0).abs() < 1e-5, "{}", hit.t);
        assert!(hit.normal.y > 0.999);
    }

    #[test]
    fn capsule_against_a_triangle_edge_stops_at_the_exact_gap() {
        let wall = Part::Tri(
            Tri::new(
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(0.0, 2.0, 0.0),
                Vec3::new(0.0, 0.0, 2.0),
            )
            .unwrap(),
        );
        // A vertical capsule moving along -x towards the triangle's edge x = 0, z in [0, 2].
        let (p, q) = (Vec3::new(1.0, 0.5, 1.0), Vec3::new(1.0, 1.5, 1.0));
        let hit = sweep(p, q, 0.3, Vec3::new(-2.0, 0.0, 0.0), &wall, TOL).unwrap();
        let off = Vec3::new(-2.0, 0.0, 0.0) * hit.t;
        let gap = contact(p + off, q + off, 0.3, &wall, TOL.flat).separation;
        assert!(gap.abs() < 1e-5, "{gap}");
        assert!((hit.t - 0.35).abs() < 1e-4, "{}", hit.t);
    }

    #[test]
    fn moving_away_or_along_while_touching_passes() {
        let p = Vec3::new(0.0, 0.5, 0.0);
        assert_eq!(
            sweep(p, p, 0.5, Vec3::new(1.0, 0.0, 0.0), &floor(), TOL),
            None
        );
        assert_eq!(
            sweep(p, p, 0.5, Vec3::new(0.0, 1.0, 0.0), &floor(), TOL),
            None
        );
        let into = sweep(p, p, 0.5, Vec3::new(0.0, -1.0, 0.0), &floor(), TOL).unwrap();
        assert_eq!(into.t, 0.0);
    }

    #[test]
    fn grazing_miss_is_not_a_hit() {
        let post = Part::Round(Round {
            a: Vec3::new(0.0, 0.0, 0.0),
            b: Vec3::new(0.0, 2.0, 0.0),
            r: 0.1,
        });
        let p = Vec3::new(-2.0, 1.0, 0.41);
        assert_eq!(sweep(p, p, 0.3, Vec3::new(4.0, 0.0, 0.0), &post, TOL), None);
        let p = Vec3::new(-2.0, 1.0, 0.39);
        assert!(sweep(p, p, 0.3, Vec3::new(4.0, 0.0, 0.0), &post, TOL).is_some());
    }

    #[test]
    fn fast_motion_through_a_thin_triangle_is_caught() {
        let wall = Part::Tri(
            Tri::new(
                Vec3::new(0.0, -1.0, -1.0),
                Vec3::new(0.0, 3.0, -1.0),
                Vec3::new(0.0, -1.0, 3.0),
            )
            .unwrap(),
        );
        let p = Vec3::new(-0.4, 0.5, 0.0);
        let hit = sweep(p, p, 0.3, Vec3::new(30.0, 0.0, 0.0), &wall, TOL).unwrap();
        assert!((hit.t * 30.0 - 0.1).abs() < 1e-4, "{}", hit.t * 30.0);
    }
}
