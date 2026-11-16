// SPDX-License-Identifier: Apache-2.0

//! The colliders and the geometric queries over them. Static parts sit in a
//! BVH in world space; a platform's parts stay in its own frame and queries
//! are moved into that frame.

use std::ops::Range;

use glam::{Mat3, Vec3};

use crate::actor::Probe;
use crate::bvh::{Bvh, BvhBuild};
use crate::geom::ray_triangle;
use crate::math::{Bounds, yaw_matrix};
use crate::narrow::{Contact, Part, Round, Tolerance, contact, sweep};
use crate::shape::{Plane, Shape, ShapeError, inside, parts_of};

/// Pose of a platform: where its local origin is and how far it is turned about +Y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub translation: Vec3,
    pub yaw: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct Kinematic {
    pub pose: Pose,
    pub rotation: Mat3,
    pub local_bounds: Bounds,
    /// Set by the step that moved the platform: the pose it moved from.
    pub moved_from: Option<Pose>,
}

#[derive(Clone, Debug)]
pub(crate) struct Collider {
    pub name: String,
    pub enabled: bool,
    pub parts: Range<u32>,
    pub planes: Range<u32>,
    /// World bounds, at the current pose for a platform.
    pub bounds: Bounds,
    pub kinematic: Option<Kinematic>,
}

/// Which colliders a query sees, besides the enabled flag.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Filter {
    pub exclude: Option<u32>,
    pub only: Option<u32>,
    pub skip_platforms: bool,
}

impl Filter {
    pub const ALL: Filter = Filter {
        exclude: None,
        only: None,
        skip_platforms: false,
    };

    pub fn except(collider: Option<u32>) -> Filter {
        Filter {
            exclude: collider,
            ..Filter::ALL
        }
    }
}

/// The first thing a sweep touches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Hit {
    pub t: f32,
    pub normal: Vec3,
    pub point: Vec3,
    pub collider: u32,
    pub start_separation: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RayHit {
    pub distance: f32,
    pub normal: Vec3,
    pub collider: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Scene {
    pub colliders: Vec<Collider>,
    parts: Vec<Part>,
    part_collider: Vec<u32>,
    planes: Vec<Plane>,
    bvh: Bvh,
    solid_bvh: Bvh,
    solids: Vec<u32>,
    platforms: Vec<u32>,
    pub tol: Tolerance,
}

fn part_bounds(part: &Part) -> Bounds {
    match part {
        Part::Tri(t) => Bounds::of_points([t.a, t.b, t.c]),
        Part::Round(r) => Bounds::of_points([r.a, r.b]).grown(r.r),
    }
}

fn round_bounds(r: &Round) -> Bounds {
    Bounds::of_points([r.a, r.b]).grown(r.r)
}

/// A shape to add, static or as a platform at `pose` (the shape then in the platform's frame).
pub(crate) struct NewCollider<'a> {
    pub name: &'a str,
    pub shape: Shape<'a>,
    pub enabled: bool,
    pub platform: Option<Pose>,
}

/// A [`Scene`] built a share at a time: its colliders one by one
/// ([`SceneBuild::add`]), then the trees over them ([`SceneBuild::advance`]).
/// It gives the scene [`Scene::new`] gives.
#[derive(Debug)]
pub(crate) struct SceneBuild {
    scene: Scene,
    tree: Option<(Vec<Bounds>, BvhBuild)>,
}

impl SceneBuild {
    pub fn new(tol: Tolerance, colliders: usize) -> SceneBuild {
        SceneBuild {
            scene: Scene {
                colliders: Vec::with_capacity(colliders),
                parts: Vec::new(),
                part_collider: Vec::new(),
                planes: Vec::new(),
                bvh: Bvh::default(),
                solid_bvh: Bvh::default(),
                solids: Vec::new(),
                platforms: Vec::new(),
                tol,
            },
            tree: None,
        }
    }

    /// Adds the next collider; none may follow the first
    /// [`SceneBuild::advance`].
    pub fn add(&mut self, c: &NewCollider) -> Result<(), ShapeError> {
        let scene = &mut self.scene;
        let parts = parts_of(&c.shape)?;
        let id = scene.colliders.len() as u32;
        let first = scene.parts.len() as u32;
        let local = parts
            .parts
            .iter()
            .fold(Bounds::EMPTY, |b, p| b.union(part_bounds(p)));
        scene
            .part_collider
            .extend(std::iter::repeat_n(id, parts.parts.len()));
        scene.parts.extend(parts.parts);
        let first_plane = scene.planes.len() as u32;
        scene.planes.extend(parts.planes);
        let kinematic = c.platform.map(|pose| Kinematic {
            pose,
            rotation: yaw_matrix(pose.yaw),
            local_bounds: local,
            moved_from: None,
        });
        let bounds = match &kinematic {
            Some(k) => transformed(local, k.rotation, k.pose.translation),
            None => local,
        };
        if kinematic.is_some() {
            scene.platforms.push(id);
        } else if scene.planes.len() as u32 > first_plane {
            scene.solids.push(id);
        }
        scene.colliders.push(Collider {
            name: c.name.to_owned(),
            enabled: c.enabled,
            parts: first..scene.parts.len() as u32,
            planes: first_plane..scene.planes.len() as u32,
            bounds,
            kinematic,
        });
        Ok(())
    }

    /// Builds the tree over the static parts on, about `units` of its work;
    /// the scene once it is built.
    pub fn advance(&mut self, units: usize) -> Option<Scene> {
        let scene = &self.scene;
        let (parts, tree) = self.tree.get_or_insert_with(|| {
            let parts: Vec<Bounds> = (scene.parts.iter().zip(&scene.part_collider))
                .map(|(p, &c)| {
                    if scene.colliders[c as usize].kinematic.is_some() {
                        Bounds::EMPTY
                    } else {
                        part_bounds(p)
                    }
                })
                .collect();
            let tree = BvhBuild::new(parts.len());
            (parts, tree)
        });
        if !tree.advance(parts, units) {
            return None;
        }
        let (parts, tree) = self.tree.take()?;
        let empty = SceneBuild::new(self.scene.tol, 0).scene;
        let mut scene = std::mem::replace(&mut self.scene, empty);
        scene.bvh = tree.finish(&parts);
        let solid_bounds: Vec<Bounds> = scene
            .solids
            .iter()
            .map(|&c| scene.colliders[c as usize].bounds)
            .collect();
        scene.solid_bvh = Bvh::build(&solid_bounds);
        Some(scene)
    }
}

impl Scene {
    pub fn new(colliders: &[NewCollider], tol: Tolerance) -> Result<Scene, (usize, ShapeError)> {
        let mut build = SceneBuild::new(tol, colliders.len());
        for (i, c) in colliders.iter().enumerate() {
            build.add(c).map_err(|e| (i, e))?;
        }
        loop {
            if let Some(scene) = build.advance(usize::MAX) {
                return Ok(scene);
            }
        }
    }

    pub fn static_bounds(&self) -> Bounds {
        self.bvh.bounds()
    }

    pub fn platforms(&self) -> &[u32] {
        &self.platforms
    }

    pub fn set_pose(&mut self, collider: u32, pose: Pose) {
        let c = &mut self.colliders[collider as usize];
        let Some(k) = &mut c.kinematic else { return };
        if k.moved_from.is_none() {
            k.moved_from = Some(k.pose);
        }
        k.pose = pose;
        k.rotation = yaw_matrix(pose.yaw);
        c.bounds = transformed(k.local_bounds, k.rotation, pose.translation);
    }

    /// Forgets where the platforms moved from, once their riders were carried.
    pub fn settle_platforms(&mut self) {
        for &p in &self.platforms {
            if let Some(k) = &mut self.colliders[p as usize].kinematic {
                k.moved_from = None;
            }
        }
    }

    fn usable(&self, collider: u32, filter: Filter) -> bool {
        filter.exclude != Some(collider)
            && filter.only.is_none_or(|o| o == collider)
            && self.colliders[collider as usize].enabled
    }

    fn platforms_for(&self, filter: Filter) -> &[u32] {
        if filter.skip_platforms {
            &[]
        } else {
            &self.platforms
        }
    }

    /// First contact of `probe` moving by `motion`.
    pub fn sweep(&self, probe: &Probe, motion: Vec3, filter: Filter) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        let mut keep = |hit: Hit| {
            if best.is_none_or(|b| hit.t < b.t) {
                best = Some(hit);
            }
        };
        for round in probe.rounds() {
            let area = round_bounds(round)
                .union(round_bounds(&Round {
                    a: round.a + motion,
                    b: round.b + motion,
                    r: round.r,
                }))
                .grown(self.tol.contact);
            self.bvh.query(&area, |i| {
                let collider = self.part_collider[i as usize];
                if !self.usable(collider, filter) {
                    return;
                }
                if let Some(s) = sweep(
                    round.a,
                    round.b,
                    round.r,
                    motion,
                    &self.parts[i as usize],
                    self.tol,
                ) {
                    keep(Hit {
                        t: s.t,
                        normal: s.normal,
                        point: s.point,
                        collider,
                        start_separation: s.start_separation,
                    });
                }
            });
            for &collider in self.platforms_for(filter) {
                let c = &self.colliders[collider as usize];
                if !self.usable(collider, filter) || !c.bounds.overlaps(&area) {
                    continue;
                }
                let k = c.kinematic.as_ref().expect("a platform");
                let to_local = k.rotation.transpose();
                let (a, b) = (
                    to_local * (round.a - k.pose.translation),
                    to_local * (round.b - k.pose.translation),
                );
                let m = to_local * motion;
                for part in &self.parts[c.parts.start as usize..c.parts.end as usize] {
                    if let Some(s) = sweep(a, b, round.r, m, part, self.tol) {
                        keep(Hit {
                            t: s.t,
                            normal: k.rotation * s.normal,
                            point: k.rotation * s.point + k.pose.translation,
                            collider,
                            start_separation: s.start_separation,
                        });
                    }
                }
            }
        }
        best
    }

    /// Calls `found` with every collider part within `margin` of `probe`, and
    /// with every solid that wholly contains one of its rounds.
    pub fn contacts(
        &self,
        probe: &Probe,
        margin: f32,
        filter: Filter,
        mut found: impl FnMut(u32, Contact),
    ) {
        for round in probe.rounds() {
            let area = round_bounds(round).grown(margin);
            self.bvh.query(&area, |i| {
                let collider = self.part_collider[i as usize];
                if !self.usable(collider, filter) {
                    return;
                }
                let c = contact(
                    round.a,
                    round.b,
                    round.r,
                    &self.parts[i as usize],
                    self.tol.flat,
                );
                if c.separation < margin {
                    found(collider, c);
                }
            });
            self.solid_bvh.query(&area, |i| {
                let collider = self.solids[i as usize];
                if !self.usable(collider, filter) {
                    return;
                }
                let planes = &self.colliders[collider as usize].planes;
                let planes = &self.planes[planes.start as usize..planes.end as usize];
                if let Some((separation, normal)) = inside(planes, round.a, round.b, round.r) {
                    let point = round.a - normal * (separation + round.r);
                    found(
                        collider,
                        Contact {
                            separation,
                            normal,
                            point,
                        },
                    );
                }
            });
            for &collider in self.platforms_for(filter) {
                let c = &self.colliders[collider as usize];
                if !self.usable(collider, filter) || !c.bounds.overlaps(&area) {
                    continue;
                }
                let k = c.kinematic.as_ref().expect("a platform");
                let to_local = k.rotation.transpose();
                let (a, b) = (
                    to_local * (round.a - k.pose.translation),
                    to_local * (round.b - k.pose.translation),
                );
                let world = |c: Contact| Contact {
                    separation: c.separation,
                    normal: k.rotation * c.normal,
                    point: k.rotation * c.point + k.pose.translation,
                };
                for part in &self.parts[c.parts.start as usize..c.parts.end as usize] {
                    let hit = contact(a, b, round.r, part, self.tol.flat);
                    if hit.separation < margin {
                        found(collider, world(hit));
                    }
                }
                let planes = &self.planes[c.planes.start as usize..c.planes.end as usize];
                if let Some((separation, normal)) = inside(planes, a, b, round.r) {
                    found(
                        collider,
                        world(Contact {
                            separation,
                            normal,
                            point: a - normal * (separation + round.r),
                        }),
                    );
                }
            }
        }
    }

    /// The deepest overlap of `probe`, if anything overlaps it by more than the
    /// contact tolerance.
    pub fn deepest(&self, probe: &Probe, filter: Filter) -> Option<(u32, Contact)> {
        let mut best: Option<(u32, Contact)> = None;
        self.contacts(probe, -self.tol.contact, filter, |collider, c| {
            if best.is_none_or(|(_, b)| c.separation < b.separation) {
                best = Some((collider, c));
            }
        });
        best
    }

    /// First surface along the ray from `origin` in unit direction `dir`, within `max`.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32, filter: Filter) -> Option<RayHit> {
        let mut best: Option<RayHit> = None;
        let end = origin + dir * max;
        let area = Bounds::of_points([origin, end]);
        let tol = self.tol;
        let ray_part = |part: &Part, o: Vec3, d: Vec3| -> Option<(f32, Vec3)> {
            match part {
                Part::Tri(t) => ray_triangle(o, d, max, t.a, t.b, t.c).map(|dist| {
                    let n = if d.dot(t.n) > 0.0 { -t.n } else { t.n };
                    (dist, n)
                }),
                Part::Round(_) => {
                    sweep(o, o, 0.0, d * max, part, tol).map(|s| (s.t * max, s.normal))
                }
            }
        };
        self.bvh.query(&area, |i| {
            let collider = self.part_collider[i as usize];
            if !self.usable(collider, filter) {
                return;
            }
            if let Some((distance, normal)) = ray_part(&self.parts[i as usize], origin, dir)
                && best.is_none_or(|b| distance < b.distance)
            {
                best = Some(RayHit {
                    distance,
                    normal,
                    collider,
                });
            }
        });
        for &collider in self.platforms_for(filter) {
            let c = &self.colliders[collider as usize];
            if !self.usable(collider, filter) || !c.bounds.overlaps(&area) {
                continue;
            }
            let k = c.kinematic.as_ref().expect("a platform");
            let to_local = k.rotation.transpose();
            let (o, d) = (to_local * (origin - k.pose.translation), to_local * dir);
            for part in &self.parts[c.parts.start as usize..c.parts.end as usize] {
                if let Some((distance, normal)) = ray_part(part, o, d)
                    && best.is_none_or(|b| distance < b.distance)
                {
                    best = Some(RayHit {
                        distance,
                        normal: k.rotation * normal,
                        collider,
                    });
                }
            }
        }
        best
    }

    /// Every collider part touched by `probe` swept along `motion`, with the
    /// fraction of the motion at first touch, unordered and per part.
    pub fn sweep_all(
        &self,
        probe: &Probe,
        motion: Vec3,
        filter: Filter,
        mut found: impl FnMut(u32, f32),
    ) {
        for round in probe.rounds() {
            let area = round_bounds(round)
                .union(round_bounds(&Round {
                    a: round.a + motion,
                    b: round.b + motion,
                    r: round.r,
                }))
                .grown(self.tol.contact);
            self.bvh.query(&area, |i| {
                let collider = self.part_collider[i as usize];
                if !self.usable(collider, filter) {
                    return;
                }
                let part = &self.parts[i as usize];
                let start = contact(round.a, round.b, round.r, part, self.tol.flat);
                if start.separation < self.tol.contact {
                    found(collider, 0.0);
                } else if let Some(s) = sweep(round.a, round.b, round.r, motion, part, self.tol) {
                    found(collider, s.t);
                }
            });
        }
    }
}

/// Bounds of `local` turned by `rotation` and moved by `translation`.
fn transformed(local: Bounds, rotation: Mat3, translation: Vec3) -> Bounds {
    if local.is_empty() {
        return local;
    }
    Bounds::of_points((0..8).map(|i| {
        let p = Vec3::new(
            if i & 1 == 0 { local.min.x } else { local.max.x },
            if i & 2 == 0 { local.min.y } else { local.max.y },
            if i & 4 == 0 { local.min.z } else { local.max.z },
        );
        rotation * p + translation
    }))
}
