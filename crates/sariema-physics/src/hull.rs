// SPDX-License-Identifier: Apache-2.0

//! Convex hull of a point set, by incremental insertion: each point outside
//! the current hull removes the faces it sees and closes the hole with a fan
//! to the horizon. O(n²), meant for the few dozen points of a prop collider.

use glam::Vec3;

/// A closed convex polyhedron with outward, counter-clockwise triangles.
#[derive(Clone, Debug, PartialEq)]
pub struct Hull {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
}

impl Hull {
    pub fn volume(&self) -> f32 {
        let o = self.vertices[0];
        self.triangles
            .iter()
            .map(|&[a, b, c]| {
                let (a, b, c) = (
                    self.vertices[a as usize] - o,
                    self.vertices[b as usize] - o,
                    self.vertices[c as usize] - o,
                );
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }
}

/// The hull of `points`, or `None` when they span no volume. A point closer
/// than `1e-5` × the set's extent to a face's plane counts as on it.
pub fn convex_hull(points: &[Vec3]) -> Option<Hull> {
    let (lo, hi) = points
        .iter()
        .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(lo, hi), &p| {
            (lo.min(p), hi.max(p))
        });
    let extent = (hi - lo).max_element();
    if !extent.is_finite() || extent <= 0.0 {
        return None;
    }
    let eps = extent * 1e-5;
    let i0 =
        (0..points.len()).min_by(|&a, &b| points[a].x.total_cmp(&points[b].x).then(a.cmp(&b)))?;
    let farthest = |score: &dyn Fn(Vec3) -> f32| {
        (0..points.len()).max_by(|&a, &b| {
            score(points[a])
                .total_cmp(&score(points[b]))
                .then(b.cmp(&a))
        })
    };
    let p0 = points[i0];
    let i1 = farthest(&|p| p.distance_squared(p0))?;
    let p1 = points[i1];
    let line = (p1 - p0).normalize_or_zero();
    let i2 = farthest(&|p| (p - p0).cross(line).length_squared())?;
    let p2 = points[i2];
    let plane = (p1 - p0).cross(p2 - p0).normalize_or_zero();
    let i3 = farthest(&|p| (p - p0).dot(plane).abs())?;
    if (points[i3] - p0).dot(plane).abs() <= eps || plane == Vec3::ZERO {
        return None;
    }

    let mut faces: Vec<[usize; 3]> = vec![[i0, i1, i2], [i0, i3, i1], [i1, i3, i2], [i2, i3, i0]];
    if (points[i3] - p0).dot(plane) > 0.0 {
        for f in &mut faces {
            f.swap(1, 2);
        }
    }
    let normal = |f: &[usize; 3]| {
        (points[f[1]] - points[f[0]])
            .cross(points[f[2]] - points[f[0]])
            .normalize_or_zero()
    };
    let mut visible = Vec::new();
    let mut horizon = Vec::new();
    for (i, &p) in points.iter().enumerate() {
        if [i0, i1, i2, i3].contains(&i) {
            continue;
        }
        visible.clear();
        visible.extend(
            faces
                .iter()
                .enumerate()
                .filter(|(_, f)| (p - points[f[0]]).dot(normal(f)) > eps)
                .map(|(k, _)| k),
        );
        if visible.is_empty() {
            continue;
        }
        horizon.clear();
        for &k in &visible {
            let f = faces[k];
            for e in 0..3 {
                let (a, b) = (f[e], f[(e + 1) % 3]);
                let shared = visible.iter().any(|&j| {
                    let g = faces[j];
                    (0..3).any(|x| g[x] == b && g[(x + 1) % 3] == a)
                });
                if !shared {
                    horizon.push((a, b));
                }
            }
        }
        let mut k = 0;
        faces.retain(|_| {
            let keep = !visible.contains(&k);
            k += 1;
            keep
        });
        faces.extend(horizon.iter().map(|&(a, b)| [a, b, i]));
    }

    let mut used: Vec<usize> = faces.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    let index_of = |i: usize| used.binary_search(&i).expect("a used vertex") as u32;
    Some(Hull {
        vertices: used.iter().map(|&i| points[i]).collect(),
        triangles: faces.iter().map(|f| f.map(index_of)).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hull_of_a_cube_with_inner_points() {
        let mut points = Vec::new();
        for i in 0..8 {
            points.push(Vec3::new(
                (i & 1) as f32,
                ((i >> 1) & 1) as f32,
                ((i >> 2) & 1) as f32,
            ));
        }
        points.push(Vec3::splat(0.5));
        points.push(Vec3::new(0.2, 0.7, 0.3));
        points.push(Vec3::new(0.5, 0.0, 0.5));
        let hull = convex_hull(&points).unwrap();
        assert_eq!(hull.vertices.len(), 8);
        assert_eq!(hull.triangles.len(), 12);
        assert!((hull.volume() - 1.0).abs() < 1e-5, "{}", hull.volume());
        let centre = Vec3::splat(0.5);
        for &[a, b, c] in &hull.triangles {
            let (a, b, c) = (
                hull.vertices[a as usize],
                hull.vertices[b as usize],
                hull.vertices[c as usize],
            );
            assert!((centre - a).dot((b - a).cross(c - a)) < 0.0, "inward face");
        }
    }

    #[test]
    fn flat_points_have_no_hull() {
        let points = [Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 1.0)];
        assert_eq!(convex_hull(&points), None);
    }
}
