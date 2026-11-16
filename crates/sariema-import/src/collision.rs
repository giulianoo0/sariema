// SPDX-License-Identifier: Apache-2.0

//! Bakes a room's collision pack from the kit and placement its room pack is
//! built from, as the room description's `collision` key asks.
//!
//! Each placed piece becomes one collider, named like a loose draw. A piece
//! whose convex hull fills its box becomes that box; one whose hull hugs its
//! surface becomes the hull; any other piece, and every flat one, becomes its
//! triangles, simplified. Authored values are on the engine's axes: Y up,
//! north is −Z, metres, yaw about +Y with yaw 0 facing +Z.

use std::collections::HashMap;
use std::path::Path;

use glam::{Mat3, Mat4, Vec3};
use meshopt::SimplifyOptions;
use sariema_format::collision::{self as pack, CollisionPack};
use sariema_format::{AlphaMode, Material};
use sariema_physics::{
    ActorCollider, ColliderDesc, LocalBounds, Settings, Shape, World, convex_hull, yaw_matrix,
};
use serde::Deserialize;

use crate::room::{Montagem, RoomSource, instance_primitives, placement, read_json};
use crate::{ImportError, Kit, KitPrimitive, load_kit};

/// The `collision` key of a room description, as `schemas/room.schema.json` defines it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CollisionSource {
    /// Pieces that get no collider.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Pieces baked as their simplified triangles, whatever their shape.
    #[serde(default)]
    pub as_mesh: Vec<String>,
    #[serde(default)]
    pub as_hull: Vec<String>,
    #[serde(default)]
    pub as_box: Vec<String>,
    /// Colliders no piece draws, such as invisible blockers.
    #[serde(default)]
    pub primitives: Vec<PrimitiveSource>,
    #[serde(default)]
    pub triggers: Vec<TriggerSource>,
    #[serde(default)]
    pub actors: Vec<ActorSource>,
    #[serde(default)]
    pub spots: Vec<SpotSource>,
    /// Pairs of spot names a script walks between; `sariema check` searches a path for each.
    #[serde(default)]
    pub routes: Vec<[String; 2]>,
    /// Scripted moves with the collider off; `sariema check` proves where they end and touch.
    #[serde(default)]
    pub traversals: Vec<TraversalSource>,
    #[serde(default)]
    pub bake: BakeSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct BakeSettings {
    /// How far, in metres, simplification may move a mesh collider's surface.
    /// A piece thinner than this is flat and keeps its triangles.
    pub simplify_error: f32,
    /// Most points a hull collider keeps.
    pub hull_points: u32,
    /// Share of its box a piece's hull must fill for the box to stand for it.
    pub box_fill: f32,
    /// Farthest, in metres, a hull's surface may stand off the piece's for the hull to stand for it.
    pub hull_tolerance: f32,
}

impl Default for BakeSettings {
    fn default() -> BakeSettings {
        BakeSettings {
            simplify_error: 0.02,
            hull_points: 32,
            box_fill: 0.85,
            hull_tolerance: 0.05,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeSource {
    Box {
        center: [f32; 3],
        half_extents: [f32; 3],
        #[serde(default)]
        yaw_deg: f32,
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
}

#[derive(Clone, Debug, Deserialize)]
pub struct PrimitiveSource {
    pub name: String,
    #[serde(flatten)]
    pub shape: ShapeSource,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
pub struct TriggerSource {
    pub name: String,
    #[serde(flatten)]
    pub shape: ShapeSource,
}

/// An actor's collider: given whole, or fitted around `visual_bounds` (with
/// a head sphere around `head_bounds` when given).
#[derive(Clone, Debug, Deserialize)]
pub struct ActorSource {
    pub name: String,
    pub radius: Option<f32>,
    pub height: Option<f32>,
    pub head: Option<HeadSource>,
    pub visual_bounds: Option<BoundsSource>,
    pub head_bounds: Option<BoundsSource>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct HeadSource {
    pub radius: f32,
    pub height: f32,
    #[serde(default)]
    pub forward: f32,
}

/// In the actor's frame: feet at the origin, facing +Z.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct BoundsSource {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Clone, Debug, Deserialize)]
pub struct SpotSource {
    pub name: String,
    pub actor: String,
    /// The actor's feet.
    pub at: [f32; 3],
    #[serde(default)]
    pub yaw_deg: f32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TraversalSource {
    pub name: String,
    /// The spot it starts from, whose actor it carries.
    pub from: String,
    /// The last is where the actor stands again.
    pub points: Vec<[f32; 3]>,
    /// Facing at the end.
    #[serde(default)]
    pub yaw_deg: f32,
    #[serde(default)]
    pub contacts: Vec<ContactSource>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ContactSource {
    pub name: String,
    pub at: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Mesh,
    Hull,
    Box,
}

/// The collision pack of the sub-rooms in `source.show`, or `None` when the
/// description has no `collision` key.
pub fn build_collision(
    source: &RoomSource,
    assets_root: &Path,
) -> Result<Option<CollisionPack>, ImportError> {
    let Some(collision) = &source.collision else {
        return Ok(None);
    };
    let kit = load_kit(&assets_root.join(&source.kit))?;
    let montagem: Montagem = read_json(&assets_root.join(&source.montagem))?;
    let schema =
        |why: String| ImportError::Schema(format!("room {}: collision: {why}", source.name));
    check_lists(source, collision, &montagem).map_err(schema)?;

    let mut colliders = Vec::new();
    for room in &source.show {
        let comodo = montagem
            .comodos
            .get(room)
            .ok_or_else(|| ImportError::Schema(format!("montagem has no room {room}")))?;
        let corner = comodo.sw_celulas.map(|cells| cells * montagem.celula);
        let mut copies: HashMap<&str, u32> = HashMap::new();
        for inst in &comodo.instancias {
            let piece = kit
                .piece(&inst.peca)
                .ok_or_else(|| ImportError::PieceMissing {
                    piece: inst.peca.clone(),
                    room: room.clone(),
                })?;
            let copy = copies.entry(&inst.peca).or_default();
            *copy += 1;
            if collision.exclude.contains(&inst.peca) {
                continue;
            }
            let name = match *copy {
                1 => format!("{room}.{}", inst.peca),
                n => format!("{room}.{}.{n}", inst.peca),
            };
            let primitives = instance_primitives(piece, inst, room)?;
            let (vertices, triangles) =
                solid_triangles(&primitives, &kit, &source.additive_materials);
            if triangles.is_empty() {
                continue;
            }
            let named = |list: &[String]| list.contains(&inst.peca);
            let kind = [
                (Kind::Mesh, &collision.as_mesh),
                (Kind::Hull, &collision.as_hull),
                (Kind::Box, &collision.as_box),
            ]
            .into_iter()
            .find(|(_, list)| named(list))
            .map(|(kind, _)| kind);
            let transform = placement(inst, corner, room)?;
            let shape = bake_piece(&vertices, &triangles, transform, kind, &collision.bake)
                .map_err(|why| schema(format!("{name} {why}")))?;
            let hidden = (piece.hidden_at_start && inst.estado.is_none())
                || source.hidden.contains(&inst.peca);
            colliders.push(pack::Collider {
                name,
                shape,
                enabled: !hidden,
            });
        }
    }
    colliders.extend(collision.primitives.iter().map(|p| pack::Collider {
        name: p.name.clone(),
        shape: authored(&p.shape),
        enabled: p.enabled,
    }));
    let triggers = collision
        .triggers
        .iter()
        .map(|t| pack::Trigger {
            name: t.name.clone(),
            shape: authored(&t.shape),
        })
        .collect();
    let actors = collision
        .actors
        .iter()
        .map(actor_body)
        .collect::<Result<_, _>>()
        .map_err(schema)?;
    let spots = collision
        .spots
        .iter()
        .map(|s| {
            let actor = collision
                .actors
                .iter()
                .position(|a| a.name == s.actor)
                .ok_or_else(|| {
                    format!(
                        "spot {} names actor {}, which is not in actors",
                        s.name, s.actor
                    )
                })?;
            Ok(pack::Spot {
                name: s.name.clone(),
                actor: actor as u32,
                position: s.at,
                yaw: s.yaw_deg.to_radians(),
            })
        })
        .collect::<Result<_, String>>()
        .map_err(schema)?;
    let spot_index = |name: &str| collision.spots.iter().position(|s| s.name == name);
    let routes = collision
        .routes
        .iter()
        .map(|[from, to]| match (spot_index(from), spot_index(to)) {
            (Some(a), Some(b)) => Ok([a as u32, b as u32]),
            _ => Err(format!(
                "route {from} → {to} names a spot that is not in spots"
            )),
        })
        .collect::<Result<_, String>>()
        .map_err(schema)?;
    let traversals = collision
        .traversals
        .iter()
        .map(|t| {
            let from = spot_index(&t.from).ok_or_else(|| {
                format!(
                    "traversal {} starts at spot {}, which is not in spots",
                    t.name, t.from
                )
            })?;
            Ok(pack::Traversal {
                name: t.name.clone(),
                from: from as u32,
                points: t.points.clone(),
                yaw: t.yaw_deg.to_radians(),
                contacts: t
                    .contacts
                    .iter()
                    .map(|c| pack::Contact {
                        name: c.name.clone(),
                        point: c.at,
                    })
                    .collect(),
            })
        })
        .collect::<Result<_, String>>()
        .map_err(schema)?;
    let pack = CollisionPack {
        room: source.name.clone(),
        colliders,
        triggers,
        actors,
        spots,
        routes,
        traversals,
    };
    pack::decode(&pack::encode(&pack)).map_err(|e| schema(e.to_string()))?;
    Ok(Some(pack))
}

/// Every piece the lists name must be placed by a shown room, and in one shape list at most.
fn check_lists(
    source: &RoomSource,
    collision: &CollisionSource,
    montagem: &Montagem,
) -> Result<(), String> {
    let placed = |piece: &String| {
        source
            .show
            .iter()
            .filter_map(|room| montagem.comodos.get(room))
            .any(|comodo| comodo.instancias.iter().any(|inst| inst.peca == *piece))
    };
    let lists = [
        ("exclude", &collision.exclude),
        ("as_mesh", &collision.as_mesh),
        ("as_hull", &collision.as_hull),
        ("as_box", &collision.as_box),
    ];
    for (key, list) in lists {
        if let Some(piece) = list.iter().find(|p| !placed(p)) {
            return Err(format!("{key} lists {piece}, which no room in show places"));
        }
    }
    for (i, (key, list)) in lists.iter().enumerate().skip(1) {
        for (other, again) in &lists[i + 1..] {
            if let Some(piece) = list.iter().find(|p| again.contains(p)) {
                return Err(format!("{piece} is in both {key} and {other}"));
            }
        }
    }
    Ok(())
}

/// The primitives' triangles in the piece's frame, positions welded, leaving
/// out additive surfaces (light cones, glows), which block nothing.
fn solid_triangles(
    primitives: &[KitPrimitive],
    kit: &Kit,
    additive: &[String],
) -> (Vec<[f32; 3]>, Vec<u32>) {
    let blocks = |m: &Material| m.alpha != AlphaMode::Additive && !additive.contains(&m.name);
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for p in primitives
        .iter()
        .filter(|p| blocks(&kit.materials[p.material as usize]))
    {
        let base = positions.len() as u32;
        positions.extend(p.mesh.vertices.iter().map(|v| v.pos));
        indices.extend(p.mesh.indices.iter().map(|i| base + i));
    }
    let (count, remap) = meshopt::generate_vertex_remap(&positions, Some(&indices));
    let indices = meshopt::remap_index_buffer(Some(&indices), count, &remap);
    let positions = meshopt::remap_vertex_buffer(&positions, count, &remap);
    (positions, indices)
}

fn bake_piece(
    local: &[[f32; 3]],
    indices: &[u32],
    transform: Mat4,
    kind: Option<Kind>,
    bake: &BakeSettings,
) -> Result<pack::Shape, String> {
    let world: Vec<[f32; 3]> = local
        .iter()
        .map(|&p| transform.transform_point3(Vec3::from(p)).to_array())
        .collect();
    let (lo, hi) = local
        .iter()
        .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(lo, hi), &p| {
            (lo.min(p.into()), hi.max(p.into()))
        });
    let m = Mat3::from_mat4(transform);
    let scale = m.x_axis.length();
    let half = (hi - lo) * 0.5 * scale;
    let flat = 2.0 * half.min_element() <= bake.simplify_error;
    let boxed = || pack::Shape::Box {
        center: transform.transform_point3((lo + hi) * 0.5).to_array(),
        half_extents: half.to_array(),
        rotation: (m * scale.recip()).to_cols_array(),
    };
    let points: Vec<Vec3> = world.iter().map(|&p| Vec3::from(p)).collect();
    let hull = || reduced_hull(&points, bake.hull_points as usize);
    match kind {
        Some(Kind::Mesh) => return Ok(simplified(&world, indices, bake.simplify_error)),
        Some(Kind::Box) if flat => return Err("is flat and cannot be a box".into()),
        Some(Kind::Box) => return Ok(boxed()),
        Some(Kind::Hull) => {
            return hull()
                .filter(|_| !flat)
                .map(|points| pack::Shape::Hull {
                    points: points.iter().map(|p| p.to_array()).collect(),
                })
                .ok_or_else(|| "is flat and cannot be a hull".into());
        }
        None => {}
    }
    if flat {
        return Ok(simplified(&world, indices, bake.simplify_error));
    }
    let Some(hull_points) = hull() else {
        return Ok(simplified(&world, indices, bake.simplify_error));
    };
    let hull_shape = convex_hull(&hull_points).expect("the reduced points span a volume");
    if !hugs(&hull_shape, &points, indices, bake.hull_tolerance) {
        return Ok(simplified(&world, indices, bake.simplify_error));
    }
    if hull_shape.volume() >= bake.box_fill * 8.0 * half.x * half.y * half.z {
        return Ok(boxed());
    }
    Ok(pack::Shape::Hull {
        points: hull_points.iter().map(|p| p.to_array()).collect(),
    })
}

/// The hull of `points` with at most `max` vertices: the farthest point
/// along each of `max` directions spread over the sphere, which is always a
/// vertex of the full hull, so the full hull is never computed.
fn reduced_hull(points: &[Vec3], max: usize) -> Option<Vec<Vec3>> {
    if points.len() <= max {
        return convex_hull(points).map(|h| h.vertices);
    }
    let golden = std::f32::consts::PI * (3.0 - libm::sqrtf(5.0));
    let mut kept: Vec<u32> = (0..max)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / max as f32;
            let r = libm::sqrtf(1.0 - y * y);
            let a = golden * i as f32;
            let dir = Vec3::new(r * libm::cosf(a), y, r * libm::sinf(a));
            (0..points.len() as u32)
                .max_by(|&a, &b| {
                    dir.dot(points[a as usize])
                        .total_cmp(&dir.dot(points[b as usize]))
                        .then(b.cmp(&a))
                })
                .expect("there are more points than directions")
        })
        .collect();
    kept.sort_unstable();
    kept.dedup();
    let extremes: Vec<Vec3> = kept.iter().map(|&i| points[i as usize]).collect();
    convex_hull(&extremes).map(|h| h.vertices)
}

/// Whether every face of `hull` lies within `tolerance` of the piece's
/// surface, sampled at each face's centre and edge midpoints.
fn hugs(hull: &sariema_physics::Hull, points: &[Vec3], indices: &[u32], tolerance: f32) -> bool {
    let triangles = indices.as_chunks::<3>().0;
    let Ok(surface) = World::new(
        Settings::default(),
        &[ColliderDesc {
            name: "piece",
            shape: Shape::Mesh {
                vertices: points,
                triangles,
            },
            enabled: true,
            platform: None,
        }],
    ) else {
        return false;
    };
    hull.triangles.iter().all(|t| {
        let [a, b, c] = t.map(|i| hull.vertices[i as usize]);
        [
            (a + b + c) / 3.0,
            (a + b) * 0.5,
            (b + c) * 0.5,
            (c + a) * 0.5,
        ]
        .into_iter()
        .all(|p| surface.closest_point(p, tolerance).is_some())
    })
}

/// The triangles simplified within `error` metres, with only the vertices they use.
fn simplified(positions: &[[f32; 3]], indices: &[u32], error: f32) -> pack::Shape {
    let kept = meshopt::simplify_decoder(
        indices,
        positions,
        0,
        error,
        SimplifyOptions::ErrorAbsolute,
        None,
    );
    let mut index_of = vec![u32::MAX; positions.len()];
    let mut vertices = Vec::new();
    let mut triangles = Vec::with_capacity(kept.len() / 3);
    for t in kept.as_chunks::<3>().0 {
        triangles.push(t.map(|i| {
            let slot = &mut index_of[i as usize];
            if *slot == u32::MAX {
                *slot = vertices.len() as u32;
                vertices.push(positions[i as usize]);
            }
            *slot
        }));
    }
    pack::Shape::Mesh {
        vertices,
        triangles,
    }
}

fn authored(shape: &ShapeSource) -> pack::Shape {
    match *shape {
        ShapeSource::Box {
            center,
            half_extents,
            yaw_deg,
        } => pack::Shape::Box {
            center,
            half_extents,
            rotation: yaw_matrix(yaw_deg.to_radians()).to_cols_array(),
        },
        ShapeSource::Sphere { center, radius } => pack::Shape::Sphere { center, radius },
        ShapeSource::Capsule { a, b, radius } => pack::Shape::Capsule { a, b, radius },
    }
}

fn actor_body(source: &ActorSource) -> Result<pack::ActorBody, String> {
    let bounds = |b: &BoundsSource| LocalBounds {
        min: Vec3::from(b.min),
        max: Vec3::from(b.max),
    };
    let collider = match (source.radius, source.height, &source.visual_bounds) {
        (Some(radius), Some(height), _) => ActorCollider {
            radius,
            height,
            head: source.head.map(|h| sariema_physics::Head {
                radius: h.radius,
                height: h.height,
                forward: h.forward,
            }),
        },
        (None, None, Some(visual)) => {
            let mut body = bounds(visual);
            let head = source.head_bounds.as_ref().map(bounds);
            if let Some(h) = head {
                body.max.y = body.max.y.min(h.min.y);
            }
            ActorCollider::fit(body, head)
        }
        _ => {
            return Err(format!(
                "actor {} needs radius and height, or visual_bounds to fit them to",
                source.name
            ));
        }
    };
    Ok(pack::ActorBody {
        name: source.name.clone(),
        radius: collider.radius,
        height: collider.height,
        head: collider.head.map(|h| pack::HeadSphere {
            radius: h.radius,
            height: h.height,
            forward: h.forward,
        }),
        visual_bounds: source.visual_bounds.map(|b| sariema_format::Aabb {
            min: b.min,
            max: b.max,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    type Mesh = (Vec<[f32; 3]>, Vec<u32>);

    fn cuboid(mesh: &mut Mesh, lo: Vec3, hi: Vec3) {
        let base = mesh.0.len() as u32;
        for i in 0..8 {
            let pick = |bit: u32, a: f32, b: f32| if i & bit == 0 { a } else { b };
            mesh.0.push([
                pick(1, lo.x, hi.x),
                pick(2, lo.y, hi.y),
                pick(4, lo.z, hi.z),
            ]);
        }
        for q in [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ] {
            mesh.1
                .extend([q[0], q[1], q[2], q[0], q[2], q[3]].map(|i| base + i));
        }
    }

    fn kind_of(mesh: &Mesh) -> (Kind, usize) {
        match bake_piece(
            &mesh.0,
            &mesh.1,
            Mat4::IDENTITY,
            None,
            &BakeSettings::default(),
        )
        .unwrap()
        {
            pack::Shape::Box { .. } => (Kind::Box, 8),
            pack::Shape::Hull { points } => (Kind::Hull, points.len()),
            pack::Shape::Mesh { triangles, .. } => (Kind::Mesh, triangles.len()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_crate_is_a_box() {
        let mut mesh = (Vec::new(), Vec::new());
        cuboid(&mut mesh, Vec3::ZERO, Vec3::new(1.0, 0.6, 0.8));
        assert_eq!(kind_of(&mesh).0, Kind::Box);
    }

    #[test]
    fn a_door_frame_keeps_its_opening() {
        let mut mesh = (Vec::new(), Vec::new());
        cuboid(
            &mut mesh,
            Vec3::new(-0.6, 0.0, 0.0),
            Vec3::new(-0.5, 2.0, 0.1),
        );
        cuboid(
            &mut mesh,
            Vec3::new(0.5, 0.0, 0.0),
            Vec3::new(0.6, 2.0, 0.1),
        );
        cuboid(
            &mut mesh,
            Vec3::new(-0.6, 2.0, 0.0),
            Vec3::new(0.6, 2.2, 0.1),
        );
        assert_eq!(kind_of(&mesh), (Kind::Mesh, 36));
    }

    fn stone(radius: f32) -> Mesh {
        let mut mesh = (Vec::new(), Vec::new());
        let (rings, segments) = (12, 24);
        mesh.0.push([0.0, radius, 0.0]);
        for r in 1..rings {
            let polar = std::f32::consts::PI * r as f32 / rings as f32;
            for s in 0..segments {
                let a = std::f32::consts::TAU * s as f32 / segments as f32;
                mesh.0.push([
                    radius * polar.sin() * a.cos(),
                    radius * polar.cos(),
                    radius * polar.sin() * a.sin(),
                ]);
            }
        }
        mesh.0.push([0.0, -radius, 0.0]);
        let bottom = mesh.0.len() as u32 - 1;
        let at = |r: u32, s: u32| 1 + (r - 1) * segments + s % segments;
        for s in 0..segments {
            mesh.1.extend([0, at(1, s + 1), at(1, s)]);
            mesh.1
                .extend([bottom, at(rings - 1, s), at(rings - 1, s + 1)]);
            for r in 1..rings - 1 {
                mesh.1.extend([
                    at(r, s),
                    at(r, s + 1),
                    at(r + 1, s + 1),
                    at(r, s),
                    at(r + 1, s + 1),
                    at(r + 1, s),
                ]);
            }
        }
        mesh
    }

    #[test]
    fn a_rounded_stone_is_a_hull_of_at_most_the_set_points() {
        let (kind, points) = kind_of(&stone(0.35));
        assert_eq!(kind, Kind::Hull);
        assert!((4..=32).contains(&points), "{points}");
    }

    #[test]
    fn a_boulder_too_round_for_the_set_points_keeps_its_triangles() {
        assert_eq!(kind_of(&stone(1.0)).0, Kind::Mesh);
    }

    #[test]
    fn a_floor_tile_keeps_its_triangles() {
        let mesh = (
            vec![
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 0.0, 2.0],
                [0.0, 0.0, 2.0],
            ],
            vec![0, 2, 1, 0, 3, 2],
        );
        assert_eq!(kind_of(&mesh), (Kind::Mesh, 2));
    }

    #[test]
    fn simplification_merges_a_finely_split_floor() {
        let n = 10u32;
        let mut mesh = (Vec::new(), Vec::new());
        for j in 0..=n {
            for i in 0..=n {
                mesh.0.push([i as f32 * 0.2, 0.0, j as f32 * 0.2]);
            }
        }
        for j in 0..n {
            for i in 0..n {
                let v = |i: u32, j: u32| j * (n + 1) + i;
                mesh.1.extend([
                    v(i, j),
                    v(i, j + 1),
                    v(i + 1, j + 1),
                    v(i, j),
                    v(i + 1, j + 1),
                    v(i + 1, j),
                ]);
            }
        }
        let (kind, triangles) = kind_of(&mesh);
        assert_eq!(kind, Kind::Mesh);
        assert!(triangles < 50, "{triangles}");
    }
}
