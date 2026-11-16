// SPDX-License-Identifier: Apache-2.0

//! Closest points between segments, points and triangles, and a ray against a
//! triangle. The closest-point routines follow Ericson, *Real-Time Collision
//! Detection* (2005), sections 5.1.5 and 5.1.9.

use glam::Vec3;

/// Below this, a squared length counts as zero: a segment of that length is a point.
const DEGENERATE: f32 = 1e-12;

/// Closest points of segments `p1q1` and `p2q2`, first on `p1q1`.
pub fn closest_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    if a <= DEGENERATE && e <= DEGENERATE {
        return (p1, p2);
    }
    let (s, t);
    if a <= DEGENERATE {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= DEGENERATE {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > 0.0 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// The point of triangle `abc` (unit normal `n`) closest to `p`. Inside the
/// face it is `p` projected along `n`, so the offset from it to `p` is exactly
/// along the normal however large the triangle.
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3, n: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    p - n * (p - a).dot(n)
}

/// Where segment `pq` crosses triangle `abc` with unit normal `n`, if it does.
pub fn segment_crosses_triangle(
    p: Vec3,
    q: Vec3,
    a: Vec3,
    b: Vec3,
    c: Vec3,
    n: Vec3,
) -> Option<Vec3> {
    let dp = (p - a).dot(n);
    let dq = (q - a).dot(n);
    if (dp > 0.0 && dq > 0.0) || (dp < 0.0 && dq < 0.0) || dp == dq {
        return None;
    }
    let x = p + (q - p) * (dp / (dp - dq));
    let inside = (b - a).cross(x - a).dot(n) >= 0.0
        && (c - b).cross(x - b).dot(n) >= 0.0
        && (a - c).cross(x - c).dot(n) >= 0.0;
    inside.then_some(x)
}

/// Closest points of segment `pq` and triangle `abc` (unit normal `n`), first
/// on the segment. A segment through the triangle gives the crossing twice.
pub fn closest_segment_triangle(
    p: Vec3,
    q: Vec3,
    a: Vec3,
    b: Vec3,
    c: Vec3,
    n: Vec3,
) -> (Vec3, Vec3) {
    if let Some(x) = segment_crosses_triangle(p, q, a, b, c, n) {
        return (x, x);
    }
    let mut best = (p, closest_on_triangle(p, a, b, c, n));
    let mut best_d = best.0.distance_squared(best.1);
    let mut consider = |pair: (Vec3, Vec3)| {
        let d = pair.0.distance_squared(pair.1);
        if d < best_d {
            best_d = d;
            best = pair;
        }
    };
    if (q - p).length_squared() > DEGENERATE {
        consider((q, closest_on_triangle(q, a, b, c, n)));
        consider(closest_segments(p, q, a, b));
        consider(closest_segments(p, q, b, c));
        consider(closest_segments(p, q, c, a));
    }
    best
}

/// Distance along `dir` (unit) from `origin` to where the ray enters triangle
/// `abc`, from either side, if within `max`.
pub fn ray_triangle(origin: Vec3, dir: Vec3, max: f32, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    // Möller and Trumbore, "Fast, minimum storage ray-triangle intersection" (1997).
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (0.0..=max).contains(&t).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    const B: Vec3 = Vec3::new(2.0, 0.0, 0.0);
    const C: Vec3 = Vec3::new(0.0, 0.0, 2.0);
    /// `(B − A) × (C − A)` points down.
    const N: Vec3 = Vec3::NEG_Y;

    fn brute_segment_triangle(p: Vec3, q: Vec3) -> f32 {
        let mut best = f32::INFINITY;
        for i in 0..=400 {
            let x = p.lerp(q, i as f32 / 400.0);
            best = best.min(x.distance(closest_on_triangle(x, A, B, C, N)));
        }
        best
    }

    #[test]
    fn point_regions_of_a_triangle() {
        assert_eq!(
            closest_on_triangle(Vec3::new(-1.0, 3.0, -1.0), A, B, C, N),
            A
        );
        assert_eq!(
            closest_on_triangle(Vec3::new(0.5, 3.0, 0.5), A, B, C, N),
            Vec3::new(0.5, 0.0, 0.5)
        );
        assert_eq!(
            closest_on_triangle(Vec3::new(1.0, 0.0, -5.0), A, B, C, N),
            Vec3::new(1.0, 0.0, 0.0)
        );
        let hyp = closest_on_triangle(Vec3::new(3.0, 1.0, 3.0), A, B, C, N);
        assert!((hyp - Vec3::new(1.0, 0.0, 1.0)).length() < 1e-6, "{hyp}");
    }

    #[test]
    fn segments_parallel_crossing_and_degenerate() {
        let (a, b) = closest_segments(
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(0.5, 1.0, -1.0),
            Vec3::new(0.5, 1.0, 1.0),
        );
        assert!(
            (a - Vec3::new(0.5, 0.0, 0.0)).length() < 1e-6
                && (b - Vec3::new(0.5, 1.0, 0.0)).length() < 1e-6
        );
        let (a, b) = closest_segments(
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
        );
        assert!((a.distance(b) - 2.0).abs() < 1e-6);
        let (a, b) = closest_segments(Vec3::Y, Vec3::Y, Vec3::ZERO, Vec3::X * 3.0);
        assert_eq!((a, b), (Vec3::Y, Vec3::ZERO));
    }

    #[test]
    fn segment_triangle_matches_sampling() {
        let cases = [
            (Vec3::new(0.5, 1.0, 0.5), Vec3::new(0.7, 2.0, 0.4)),
            (Vec3::new(-1.0, 0.3, -1.0), Vec3::new(3.0, 0.3, -0.5)),
            (Vec3::new(1.5, -1.0, 1.5), Vec3::new(1.5, 1.0, 1.5)),
            (Vec3::new(0.2, 0.5, 0.2), Vec3::new(0.3, -0.5, 0.3)),
            (Vec3::new(-2.0, 1.0, 1.0), Vec3::new(-0.5, 0.2, 1.0)),
        ];
        for (p, q) in cases {
            let (s, t) = closest_segment_triangle(p, q, A, B, C, N);
            let brute = brute_segment_triangle(p, q);
            assert!(
                (s.distance(t) - brute).abs() < 2e-3,
                "{p} {q}: {} vs {brute}",
                s.distance(t)
            );
        }
    }

    #[test]
    fn ray_hits_from_both_sides_within_range() {
        let down = ray_triangle(Vec3::new(0.5, 2.0, 0.5), Vec3::NEG_Y, 5.0, A, B, C);
        assert!((down.unwrap() - 2.0).abs() < 1e-6);
        let up = ray_triangle(Vec3::new(0.5, -1.0, 0.5), Vec3::Y, 5.0, A, B, C);
        assert!((up.unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(
            ray_triangle(Vec3::new(0.5, 2.0, 0.5), Vec3::NEG_Y, 1.0, A, B, C),
            None
        );
        assert_eq!(
            ray_triangle(Vec3::new(3.0, 2.0, 3.0), Vec3::NEG_Y, 5.0, A, B, C),
            None
        );
    }

    #[test]
    fn a_ray_down_the_shared_edge_of_two_triangles_hits_one() {
        let (a, b) = (Vec3::new(0.0, 0.02, 0.0), Vec3::new(0.5, 0.06, -0.5));
        let (left, right) = (Vec3::new(0.0, 0.05, -0.5), Vec3::new(0.5, 0.03, 0.0));
        let mut missed = 0;
        for k in 0..=1000 {
            let p = a.lerp(b, k as f32 / 1000.0);
            let origin = Vec3::new(p.x, 2.0, p.z);
            let hit = ray_triangle(origin, Vec3::NEG_Y, 5.0, a, left, b)
                .or_else(|| ray_triangle(origin, Vec3::NEG_Y, 5.0, a, b, right));
            missed += usize::from(hit.is_none());
        }
        assert_eq!(missed, 0);
    }
}
