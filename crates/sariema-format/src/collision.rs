// SPDX-License-Identifier: Apache-2.0

//! The collision pack of a room: static colliders, triggers, the actors'
//! collision bodies, named action spots and the routes scripts walk between
//! them.
//!
//! A pack is the 4-byte [`MAGIC`], the [`VERSION`] as a little-endian `u32`,
//! then a postcard body holding a [`CollisionPack`]. Coordinates follow the
//! room pack: right-handed, Y up, north is −Z, metres; yaw is about +Y with
//! yaw 0 facing +Z. Rotation matrices are column-major.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Aabb;

pub const MAGIC: [u8; 4] = *b"SRMC";
pub const VERSION: u32 = 1;

const HEADER_LEN: usize = MAGIC.len() + size_of::<u32>();

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CollisionPack {
    pub room: String,
    pub colliders: Vec<Collider>,
    pub triggers: Vec<Trigger>,
    pub actors: Vec<ActorBody>,
    pub spots: Vec<Spot>,
    /// Pairs of indices into `spots` that a script walks between.
    pub routes: Vec<[u32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collider {
    /// `<room>.<piece>` for the first copy of a piece, `<room>.<piece>.<n>` for
    /// the n-th, like loose draws; the name given for an authored primitive.
    pub name: String,
    pub shape: Shape,
    /// False for a piece that starts hidden; the game enables it with the piece.
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Shape {
    Box {
        center: [f32; 3],
        half_extents: [f32; 3],
        /// Takes the box's axes to world axes; orthonormal.
        rotation: [f32; 9],
    },
    Sphere {
        center: [f32; 3],
        radius: f32,
    },
    Capsule {
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
    },
    /// The convex hull of the points.
    Hull {
        points: Vec<[f32; 3]>,
    },
    /// Two-sided triangles.
    Mesh {
        vertices: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trigger {
    pub name: String,
    pub shape: Shape,
}

/// An actor's collider: a body capsule standing on its feet and an optional head sphere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActorBody {
    pub name: String,
    pub radius: f32,
    /// Feet to the top of the body capsule; at least twice the radius.
    pub height: f32,
    pub head: Option<HeadSphere>,
    /// What the actor's model covers, in its own frame (feet at the origin,
    /// facing +Z), for checking the collider against it.
    pub visual_bounds: Option<Aabb>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadSphere {
    pub radius: f32,
    /// Height of the centre above the feet.
    pub height: f32,
    /// Offset of the centre ahead of the body's axis.
    pub forward: f32,
}

/// Where an actor stands for an action: its feet and facing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub name: String,
    /// Index into [`CollisionPack::actors`].
    pub actor: u32,
    pub position: [f32; 3],
    pub yaw: f32,
}

impl CollisionPack {
    /// Triangles of every collider, boxes counting 12 and round shapes none;
    /// hulls count their points, a bound on their faces.
    pub fn triangles(&self) -> usize {
        self.colliders
            .iter()
            .map(|c| match &c.shape {
                Shape::Box { .. } => 12,
                Shape::Sphere { .. } | Shape::Capsule { .. } => 0,
                Shape::Hull { points } => 2 * points.len(),
                Shape::Mesh { triangles, .. } => triangles.len(),
            })
            .sum()
    }

    pub fn spot(&self, name: &str) -> Option<&Spot> {
        self.spots.iter().find(|s| s.name == name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollisionError {
    BadMagic,
    Version { found: u32 },
    Truncated,
    Invalid(String),
}

impl CollisionError {
    pub fn code(&self) -> &'static str {
        match self {
            CollisionError::BadMagic => "E_PACK_BAD_MAGIC",
            CollisionError::Version { .. } => "E_PACK_VERSION",
            CollisionError::Truncated => "E_PACK_TRUNCATED",
            CollisionError::Invalid(_) => "E_SCHEMA",
        }
    }
}

impl fmt::Display for CollisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CollisionError::BadMagic => {
                write!(f, "not a collision pack: the file does not start with SRMC")
            }
            CollisionError::Version { found } => write!(
                f,
                "collision pack version {found} is not supported (expected {VERSION})"
            ),
            CollisionError::Truncated => write!(f, "collision pack is truncated"),
            CollisionError::Invalid(why) => write!(f, "collision pack is invalid: {why}"),
        }
    }
}

impl std::error::Error for CollisionError {}

/// Writes [`MAGIC`], [`VERSION`] and the postcard body. Does not validate.
pub fn encode(pack: &CollisionPack) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    // As in `crate::encode`: every length is known and a Vec cannot run out of room.
    postcard::to_extend(pack, out).expect("postcard cannot fail to serialize a CollisionPack")
}

/// Decodes a pack and validates it: numbers finite, sizes positive, bodies at
/// least as tall as they are wide, rotations orthonormal, hulls of at least
/// four points, every index in range and every name unique within its list.
/// Never panics.
pub fn decode(bytes: &[u8]) -> Result<CollisionPack, CollisionError> {
    let seen = bytes.len().min(MAGIC.len());
    if bytes[..seen] != MAGIC[..seen] {
        return Err(CollisionError::BadMagic);
    }
    let Some((header, body)) = bytes.split_first_chunk::<HEADER_LEN>() else {
        return Err(CollisionError::Truncated);
    };
    let found = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if found != VERSION {
        return Err(CollisionError::Version { found });
    }
    let (pack, rest) = postcard::take_from_bytes::<CollisionPack>(body).map_err(|e| match e {
        postcard::Error::DeserializeUnexpectedEnd => CollisionError::Truncated,
        other => CollisionError::Invalid(other.to_string()),
    })?;
    if !rest.is_empty() {
        return Err(CollisionError::Invalid(format!(
            "{} unexpected bytes after the pack",
            rest.len()
        )));
    }
    validate(&pack)?;
    Ok(pack)
}

fn finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}

fn positive(x: f32) -> bool {
    x.is_finite() && x > 0.0
}

fn check_shape(shape: &Shape) -> Result<(), String> {
    match shape {
        Shape::Box {
            center,
            half_extents,
            rotation: m,
        } => {
            if !finite(center) || !half_extents.iter().all(|&h| positive(h)) {
                return Err("a box needs a finite centre and positive half extents".into());
            }
            let col = |c: usize| [m[3 * c], m[3 * c + 1], m[3 * c + 2]];
            let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
            let orthonormal = (0..3).all(|i| {
                (0..3).all(|j| {
                    let want = if i == j { 1.0 } else { 0.0 };
                    (dot(col(i), col(j)) - want).abs() < 1e-3
                })
            });
            if !orthonormal {
                return Err("a box rotation is not orthonormal".into());
            }
        }
        Shape::Sphere { center, radius } => {
            if !finite(center) || !positive(*radius) {
                return Err("a sphere needs a finite centre and a positive radius".into());
            }
        }
        Shape::Capsule { a, b, radius } => {
            if !finite(a) || !finite(b) || !positive(*radius) {
                return Err("a capsule needs finite ends and a positive radius".into());
            }
        }
        Shape::Hull { points } => {
            if points.len() < 4 || !points.iter().all(|p| finite(p)) {
                return Err("a hull needs at least four finite points".into());
            }
        }
        Shape::Mesh {
            vertices,
            triangles,
        } => {
            if !vertices.iter().all(|v| finite(v)) {
                return Err("a mesh vertex is not finite".into());
            }
            if let Some(t) = triangles
                .iter()
                .find(|t| t.iter().any(|&i| i as usize >= vertices.len()))
            {
                return Err(format!(
                    "a mesh triangle uses vertex {} but there are {} vertices",
                    t.iter().max().unwrap_or(&0),
                    vertices.len()
                ));
            }
        }
    }
    Ok(())
}

fn unique<'a>(what: &str, names: impl Iterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(format!("two {what} are named {name}"));
        }
    }
    Ok(())
}

fn validate(pack: &CollisionPack) -> Result<(), CollisionError> {
    let invalid = CollisionError::Invalid;
    for c in &pack.colliders {
        check_shape(&c.shape).map_err(|why| invalid(format!("collider {}: {why}", c.name)))?;
    }
    for t in &pack.triggers {
        check_shape(&t.shape).map_err(|why| invalid(format!("trigger {}: {why}", t.name)))?;
    }
    for a in &pack.actors {
        let head_ok = a
            .head
            .as_ref()
            .is_none_or(|h| positive(h.radius) && h.height.is_finite() && h.forward.is_finite());
        let bounds_ok = a.visual_bounds.as_ref().is_none_or(|b| {
            finite(&b.min) && finite(&b.max) && (0..3).all(|k| b.min[k] <= b.max[k])
        });
        if !(positive(a.radius)
            && a.height.is_finite()
            && a.height >= 2.0 * a.radius
            && head_ok
            && bounds_ok)
        {
            return Err(invalid(format!(
                "actor {}: radius and head radius must be positive, height at least twice the radius, bounds ordered",
                a.name
            )));
        }
    }
    for s in &pack.spots {
        if s.actor as usize >= pack.actors.len() {
            return Err(invalid(format!(
                "spot {} uses actor {} but there are {} actors",
                s.name,
                s.actor,
                pack.actors.len()
            )));
        }
        if !finite(&s.position) || !s.yaw.is_finite() {
            return Err(invalid(format!(
                "spot {}: position or yaw is not finite",
                s.name
            )));
        }
    }
    if let Some(r) = pack
        .routes
        .iter()
        .find(|r| r.iter().any(|&i| i as usize >= pack.spots.len()))
    {
        return Err(invalid(format!(
            "route {r:?} uses a spot beyond the {} spots",
            pack.spots.len()
        )));
    }
    unique("colliders", pack.colliders.iter().map(|c| c.name.as_str())).map_err(invalid)?;
    unique("triggers", pack.triggers.iter().map(|t| t.name.as_str())).map_err(invalid)?;
    unique("actors", pack.actors.iter().map(|a| a.name.as_str())).map_err(invalid)?;
    unique("spots", pack.spots.iter().map(|s| s.name.as_str())).map_err(invalid)?;
    Ok(())
}
