// SPDX-License-Identifier: Apache-2.0

//! Collider shapes as callers give them, and their breakdown into the parts
//! the queries test: triangles and rounded segments, plus the face planes of
//! closed convex shapes so a query entirely inside one is still found.

use std::fmt;

use glam::{Mat3, Vec3};

use crate::hull::convex_hull;
use crate::narrow::{Part, Round, Tri};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape<'a> {
    /// `rotation` takes the box's axes to world axes and must be orthonormal.
    Box {
        center: Vec3,
        half_extents: Vec3,
        rotation: Mat3,
    },
    Sphere {
        center: Vec3,
        radius: f32,
    },
    /// The points within `radius` of segment `ab`.
    Capsule {
        a: Vec3,
        b: Vec3,
        radius: f32,
    },
    /// The convex hull of the points.
    Hull {
        points: &'a [Vec3],
    },
    /// Two-sided triangles.
    Mesh {
        vertices: &'a [Vec3],
        triangles: &'a [[u32; 3]],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShapeError {
    /// A size or radius that is not finite and positive.
    Size,
    /// A hull whose points span no volume.
    FlatHull,
    /// A mesh triangle naming a vertex that does not exist.
    Index { triangle: usize },
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShapeError::Size => write!(f, "a size or radius is not a positive number"),
            ShapeError::FlatHull => write!(f, "the hull's points span no volume"),
            ShapeError::Index { triangle } => {
                write!(f, "triangle {triangle} names a vertex that does not exist")
            }
        }
    }
}

/// A plane `n · x = d` with outward unit normal `n`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub n: Vec3,
    pub d: f32,
}

pub struct Parts {
    pub parts: Vec<Part>,
    /// Face planes when the shape is a closed convex solid.
    pub planes: Vec<Plane>,
}

const BOX_FACES: [[usize; 4]; 6] = [
    [0, 2, 3, 1],
    [4, 5, 7, 6],
    [0, 1, 5, 4],
    [2, 6, 7, 3],
    [0, 4, 6, 2],
    [1, 3, 7, 5],
];

fn positive(x: f32) -> bool {
    x.is_finite() && x > 0.0
}

pub fn parts_of(shape: &Shape) -> Result<Parts, ShapeError> {
    match *shape {
        Shape::Box {
            center,
            half_extents: h,
            rotation,
        } => {
            if !(positive(h.x) && positive(h.y) && positive(h.z)) {
                return Err(ShapeError::Size);
            }
            let corner = |i: usize| {
                let local = Vec3::new(
                    if i & 1 == 0 { -h.x } else { h.x },
                    if i & 2 == 0 { -h.y } else { h.y },
                    if i & 4 == 0 { -h.z } else { h.z },
                );
                center + rotation * local
            };
            let corners: [Vec3; 8] = std::array::from_fn(corner);
            let faces: Vec<[Vec3; 4]> = BOX_FACES.iter().map(|f| f.map(|i| corners[i])).collect();
            Ok(solid(&faces))
        }
        Shape::Sphere { center, radius } => round(center, center, radius),
        Shape::Capsule { a, b, radius } => round(a, b, radius),
        Shape::Hull { points } => {
            let hull = convex_hull(points).ok_or(ShapeError::FlatHull)?;
            let tris: Vec<[Vec3; 3]> = hull
                .triangles
                .iter()
                .map(|t| t.map(|i| hull.vertices[i as usize]))
                .collect();
            let mut out = Parts {
                parts: Vec::with_capacity(tris.len()),
                planes: Vec::with_capacity(tris.len()),
            };
            for [a, b, c] in tris {
                if let Some(t) = Tri::new(a, b, c) {
                    out.planes.push(Plane {
                        n: t.n,
                        d: t.n.dot(a),
                    });
                    out.parts.push(Part::Tri(t));
                }
            }
            Ok(out)
        }
        Shape::Mesh {
            vertices,
            triangles,
        } => {
            let mut parts = Vec::with_capacity(triangles.len());
            for (i, t) in triangles.iter().enumerate() {
                let v = |k: usize| {
                    vertices
                        .get(t[k] as usize)
                        .copied()
                        .ok_or(ShapeError::Index { triangle: i })
                };
                if let Some(tri) = Tri::new(v(0)?, v(1)?, v(2)?) {
                    parts.push(Part::Tri(tri));
                }
            }
            Ok(Parts {
                parts,
                planes: Vec::new(),
            })
        }
    }
}

fn round(a: Vec3, b: Vec3, r: f32) -> Result<Parts, ShapeError> {
    if !positive(r) || !a.is_finite() || !b.is_finite() {
        return Err(ShapeError::Size);
    }
    Ok(Parts {
        parts: vec![Part::Round(Round { a, b, r })],
        planes: Vec::new(),
    })
}

/// Two triangles and one plane per counter-clockwise quad face.
fn solid(faces: &[[Vec3; 4]]) -> Parts {
    let mut out = Parts {
        parts: Vec::with_capacity(faces.len() * 2),
        planes: Vec::with_capacity(faces.len()),
    };
    for &[a, b, c, d] in faces {
        if let Some(t) = Tri::new(a, b, c) {
            out.planes.push(Plane {
                n: t.n,
                d: t.n.dot(a),
            });
            out.parts.push(Part::Tri(t));
        }
        out.parts.extend(Tri::new(a, c, d).map(Part::Tri));
    }
    out
}

/// When `p` and `q` both lie inside the solid bounded by `planes`, the
/// separation (negative) and outward normal of the nearest face for a query
/// of radius `r`.
pub fn inside(planes: &[Plane], p: Vec3, q: Vec3, r: f32) -> Option<(f32, Vec3)> {
    let mut best: Option<(f32, Vec3)> = None;
    for plane in planes {
        let s = (plane.n.dot(p) - plane.d).max(plane.n.dot(q) - plane.d);
        if s > 0.0 {
            return None;
        }
        if best.is_none_or(|(b, _)| s > b) {
            best = Some((s, plane.n));
        }
    }
    best.map(|(s, n)| (s - r, n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_faces_point_outward() {
        let parts = parts_of(&Shape::Box {
            center: Vec3::new(1.0, 2.0, 3.0),
            half_extents: Vec3::new(0.5, 1.0, 2.0),
            rotation: Mat3::IDENTITY,
        })
        .unwrap();
        assert_eq!(parts.parts.len(), 12);
        assert_eq!(parts.planes.len(), 6);
        for p in &parts.planes {
            assert!(p.n.dot(Vec3::new(1.0, 2.0, 3.0)) < p.d, "{p:?}");
        }
        let (sep, n) = inside(
            &parts.planes,
            Vec3::new(1.4, 2.0, 3.0),
            Vec3::new(1.3, 2.5, 3.0),
            0.1,
        )
        .unwrap();
        assert!((sep - (-0.1 - 0.1)).abs() < 1e-6, "{sep}");
        assert_eq!(n, Vec3::X);
        assert_eq!(
            inside(
                &parts.planes,
                Vec3::new(1.6, 2.0, 3.0),
                Vec3::new(1.0, 2.0, 3.0),
                0.1
            ),
            None
        );
    }

    #[test]
    fn bad_shapes_are_errors() {
        let sphere = Shape::Sphere {
            center: Vec3::ZERO,
            radius: 0.0,
        };
        assert_eq!(parts_of(&sphere).err(), Some(ShapeError::Size));
        let mesh = Shape::Mesh {
            vertices: &[Vec3::ZERO],
            triangles: &[[0, 0, 1]],
        };
        assert_eq!(
            parts_of(&mesh).err(),
            Some(ShapeError::Index { triangle: 0 })
        );
    }
}
