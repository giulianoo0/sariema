// SPDX-License-Identifier: Apache-2.0

//! The walkable grid of one actor shape and path search on it.
//!
//! Like Recast's heightfield (walkable spans per column, a climb limit between
//! neighbours, erosion by the agent radius), but each candidate floor is
//! tested by placing the actor's own collider there, raised by the step
//! height so ledges it can step onto do not block, instead of eroding voxels.
//! A* on 8 neighbours with no corner cutting, then string pulling; every kept
//! segment is checked by sweeping the raised collider along it. Cells keep one
//! skin of clearance and segments are swept one skin thinner, so a path
//! between cells never fails on a graze the controller would slide past.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use glam::Vec3;

use crate::actor::ActorCollider;
use crate::controller::Derived;
use crate::math::{Bounds, yaw_of};
use crate::scene::{Filter, Scene};
use crate::settings::Settings;

pub(crate) struct NavCtx<'a> {
    pub scene: &'a Scene,
    pub settings: &'a Settings,
    pub derived: &'a Derived,
}

const STATIC: Filter = Filter {
    exclude: None,
    only: None,
    skip_platforms: true,
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Open {
    f: f32,
    node: u32,
}

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Open) -> Ordering {
        other.f.total_cmp(&self.f).then(other.node.cmp(&self.node))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Open) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub(crate) struct NavGrid {
    pub collider: ActorCollider,
    origin: (f32, f32),
    cell: f32,
    nx: usize,
    nz: usize,
    /// Nodes of column `c` are `column[c]..column[c + 1]`.
    column: Vec<u32>,
    /// Feet height of each node.
    height: Vec<f32>,
    node_column: Vec<u32>,
    /// Nodes joined by links, either way, share an island; a search between
    /// two islands has no path and is refused before it starts.
    island: Vec<u32>,
    dirty: Vec<Bounds>,
    g: Vec<f32>,
    parent: Vec<u32>,
    seen: Vec<u32>,
    closed: Vec<u32>,
    search: u32,
    heap: BinaryHeap<Open>,
    chain: Vec<u32>,
    points: Vec<Vec3>,
}

const DIRS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

impl NavGrid {
    pub fn build(ctx: &NavCtx, collider: ActorCollider) -> NavGrid {
        let mut grid = NavGrid::start(ctx, collider);
        let mut layers = Vec::new();
        grid.grow(ctx, &mut layers, usize::MAX);
        grid.finish(ctx);
        grid
    }

    /// A grid none of whose columns is built yet.
    pub fn start(ctx: &NavCtx, collider: ActorCollider) -> NavGrid {
        let bounds = ctx.scene.static_bounds();
        let cell = ctx.settings.nav.cell_size;
        let (nx, nz) = if bounds.is_empty() {
            (0, 0)
        } else {
            (
                ((bounds.max.x - bounds.min.x) / cell).ceil() as usize + 1,
                ((bounds.max.z - bounds.min.z) / cell).ceil() as usize + 1,
            )
        };
        let mut grid = NavGrid {
            collider,
            origin: (bounds.min.x, bounds.min.z),
            cell,
            nx,
            nz,
            column: Vec::with_capacity(nx * nz + 1),
            height: Vec::new(),
            node_column: Vec::new(),
            island: Vec::new(),
            dirty: Vec::new(),
            g: Vec::new(),
            parent: Vec::new(),
            seen: Vec::new(),
            closed: Vec::new(),
            search: 0,
            heap: BinaryHeap::new(),
            chain: Vec::new(),
            points: Vec::new(),
        };
        grid.column.push(0);
        grid
    }

    /// Builds up to `columns` more columns; true once every column is.
    pub fn grow(&mut self, ctx: &NavCtx, layers: &mut Vec<f32>, columns: usize) -> bool {
        let built = self.column.len() - 1;
        let end = (self.nx * self.nz).min(built.saturating_add(columns));
        for c in built..end {
            self.floors(ctx, c, layers);
            self.node_column
                .extend(std::iter::repeat_n(c as u32, layers.len()));
            self.height.extend_from_slice(layers);
            self.column.push(self.height.len() as u32);
        }
        end == self.nx * self.nz
    }

    /// Joins the columns built into islands and readies the search.
    pub fn finish(&mut self, ctx: &NavCtx) {
        self.scratch(ctx.settings.controller.step_height);
    }

    fn scratch(&mut self, step: f32) {
        let n = self.height.len();
        self.label_islands(step);
        self.g = vec![0.0; n];
        self.parent = vec![u32::MAX; n];
        self.seen = vec![0; n];
        self.closed = vec![0; n];
        self.search = 0;
        self.heap = BinaryHeap::with_capacity(n);
    }

    fn centre(&self, c: usize) -> (f32, f32) {
        (
            self.origin.0 + (c % self.nx) as f32 * self.cell,
            self.origin.1 + (c / self.nx) as f32 * self.cell,
        )
    }

    /// The feet heights where the actor fits in column `c`, lowest first.
    fn floors(&self, ctx: &NavCtx, c: usize, out: &mut Vec<f32>) {
        out.clear();
        let bounds = ctx.scene.static_bounds();
        let (x, z) = self.centre(c);
        let skin = ctx.settings.collision.skin;
        let max_layers = ctx.settings.nav.max_layers as usize;
        let mut y = bounds.max.y + skin;
        for _ in 0..max_layers * 4 {
            let reach = y - bounds.min.y + skin;
            let Some(hit) = ctx
                .scene
                .raycast(Vec3::new(x, y, z), Vec3::NEG_Y, reach, STATIC)
            else {
                break;
            };
            let floor = y - hit.distance;
            let feet = floor + skin;
            if hit.normal.y >= ctx.derived.cos_max_slope
                && out.last().is_none_or(|&last| last - feet > skin)
                && self.uncovered(ctx, Vec3::new(x, feet, z))
                && self.fits(ctx, Vec3::new(x, feet, z))
            {
                out.push(feet);
                if out.len() == max_layers {
                    break;
                }
            }
            y = floor - skin;
        }
        out.reverse();
    }

    /// Whether nothing lies right above `feet` within the step height, where
    /// the raised fit test does not look: a floor under a low solid, such as
    /// the ground inside a dais, is no floor.
    fn uncovered(&self, ctx: &NavCtx, feet: Vec3) -> bool {
        let lift = ctx.settings.controller.step_height;
        ctx.scene.raycast(feet, Vec3::Y, lift, STATIC).is_none()
    }

    fn fits(&self, ctx: &NavCtx, feet: Vec3) -> bool {
        let probe = self
            .collider
            .probe(feet, 0.0, ctx.settings.controller.step_height);
        ctx.scene.deepest(&probe, STATIC).is_none()
    }

    pub fn mark_dirty(&mut self, area: Bounds) {
        self.dirty.push(area);
    }

    fn refresh(&mut self, ctx: &NavCtx) {
        if self.dirty.is_empty() {
            return;
        }
        let reach = self.collider.radius.max(
            self.collider
                .head
                .map_or(0.0, |h| h.radius + h.forward.abs()),
        ) + self.cell;
        let areas: Vec<Bounds> = self.dirty.drain(..).map(|b| b.grown(reach)).collect();
        let mut column = Vec::with_capacity(self.column.len());
        let mut height = Vec::with_capacity(self.height.len());
        let mut node_column = Vec::with_capacity(self.node_column.len());
        let mut layers = Vec::new();
        column.push(0);
        for c in 0..self.nx * self.nz {
            let (x, z) = self.centre(c);
            let touched = areas
                .iter()
                .any(|a| x >= a.min.x && x <= a.max.x && z >= a.min.z && z <= a.max.z);
            if touched {
                self.floors(ctx, c, &mut layers);
            } else {
                layers.clear();
                layers.extend_from_slice(
                    &self.height[self.column[c] as usize..self.column[c + 1] as usize],
                );
            }
            node_column.extend(std::iter::repeat_n(c as u32, layers.len()));
            height.extend_from_slice(&layers);
            column.push(height.len() as u32);
        }
        self.column = column;
        self.height = height;
        self.node_column = node_column;
        self.scratch(ctx.settings.controller.step_height);
    }

    fn label_islands(&mut self, step: f32) {
        fn root(of: &mut [u32], mut n: u32) -> u32 {
            while of[n as usize] != n {
                of[n as usize] = of[of[n as usize] as usize];
                n = of[n as usize];
            }
            n
        }
        let mut of: Vec<u32> = (0..self.height.len() as u32).collect();
        for node in 0..self.height.len() as u32 {
            let col = self.node_column[node as usize] as usize;
            let (ix, iz) = ((col % self.nx) as i32, (col / self.nx) as i32);
            for (dx, dz) in DIRS {
                if let Some(next) = self.link(ix + dx, iz + dz, self.height[node as usize], step) {
                    let (a, b) = (root(&mut of, node), root(&mut of, next));
                    of[a.max(b) as usize] = a.min(b);
                }
            }
        }
        self.island = (0..self.height.len() as u32)
            .map(|n| root(&mut of, n))
            .collect();
    }

    fn column_at(&self, x: f32, z: f32) -> Option<usize> {
        let ix = ((x - self.origin.0) / self.cell).round();
        let iz = ((z - self.origin.1) / self.cell).round();
        (ix >= 0.0 && iz >= 0.0 && (ix as usize) < self.nx && (iz as usize) < self.nz)
            .then(|| iz as usize * self.nx + ix as usize)
    }

    /// The node of column `(ix, iz)` within a step of `y`, the closest one.
    fn link(&self, ix: i32, iz: i32, y: f32, step: f32) -> Option<u32> {
        if ix < 0 || iz < 0 || ix as usize >= self.nx || iz as usize >= self.nz {
            return None;
        }
        let c = iz as usize * self.nx + ix as usize;
        (self.column[c]..self.column[c + 1])
            .filter(|&n| (self.height[n as usize] - y).abs() <= step)
            .min_by(|&a, &b| {
                let da = (self.height[a as usize] - y).abs();
                let db = (self.height[b as usize] - y).abs();
                da.total_cmp(&db).then(a.cmp(&b))
            })
    }

    fn position(&self, node: u32) -> Vec3 {
        let (x, z) = self.centre(self.node_column[node as usize] as usize);
        Vec3::new(x, self.height[node as usize], z)
    }

    /// The node within the snap radius closest to `p`, preferring those on `island`.
    fn nearest(&self, ctx: &NavCtx, p: Vec3, island: Option<u32>) -> Option<u32> {
        let snap = ctx.settings.nav.snap_radius;
        let reach_y = ctx.settings.controller.step_height + ctx.settings.controller.ground_snap;
        let r = (snap / self.cell).ceil() as i32;
        let ix = ((p.x - self.origin.0) / self.cell).round() as i32;
        let iz = ((p.z - self.origin.1) / self.cell).round() as i32;
        let mut best: Option<(bool, f32, u32)> = None;
        for dz in -r..=r {
            for dx in -r..=r {
                let (cx, cz) = (ix + dx, iz + dz);
                if cx < 0 || cz < 0 || cx as usize >= self.nx || cz as usize >= self.nz {
                    continue;
                }
                let c = cz as usize * self.nx + cx as usize;
                for n in self.column[c]..self.column[c + 1] {
                    let d = self.position(n) - p;
                    let flat = d.x * d.x + d.z * d.z;
                    let key = (
                        island.is_some_and(|i| self.island[n as usize] != i),
                        flat + d.y * d.y,
                        n,
                    );
                    if flat <= snap * snap
                        && d.y.abs() <= reach_y
                        && best.is_none_or(|b| {
                            (key.0, key.1, key.2).partial_cmp(&(b.0, b.1, b.2))
                                == Some(Ordering::Less)
                        })
                    {
                        best = Some(key);
                    }
                }
            }
        }
        best.map(|(_, _, n)| n)
    }

    fn astar(&mut self, step: f32, start: u32, goal: u32) -> bool {
        self.search = self.search.wrapping_add(1);
        if self.search == 0 {
            self.seen.fill(0);
            self.closed.fill(0);
            self.search = 1;
        }
        let search = self.search;
        let goal_col = self.node_column[goal as usize] as usize;
        let (gx, gz) = ((goal_col % self.nx) as i32, (goal_col / self.nx) as i32);
        let cell = self.cell;
        let estimate = |col: usize| {
            let dx = ((col % self.nx) as i32 - gx).abs() as f32;
            let dz = ((col / self.nx) as i32 - gz).abs() as f32;
            cell * (dx.max(dz) + (std::f32::consts::SQRT_2 - 1.0) * dx.min(dz))
        };
        self.heap.clear();
        self.g[start as usize] = 0.0;
        self.parent[start as usize] = u32::MAX;
        self.seen[start as usize] = search;
        self.heap.push(Open {
            f: estimate(self.node_column[start as usize] as usize),
            node: start,
        });
        while let Some(Open { node, .. }) = self.heap.pop() {
            if self.closed[node as usize] == search {
                continue;
            }
            self.closed[node as usize] = search;
            if node == goal {
                return true;
            }
            let col = self.node_column[node as usize] as usize;
            let (ix, iz) = ((col % self.nx) as i32, (col / self.nx) as i32);
            let y = self.height[node as usize];
            let here = self.g[node as usize];
            for (dx, dz) in DIRS {
                let Some(next) = self.link(ix + dx, iz + dz, y, step) else {
                    continue;
                };
                if dx != 0
                    && dz != 0
                    && (self.link(ix + dx, iz, y, step).is_none()
                        || self.link(ix, iz + dz, y, step).is_none())
                {
                    continue;
                }
                if self.closed[next as usize] == search {
                    continue;
                }
                let flat = if dx != 0 && dz != 0 {
                    cell * std::f32::consts::SQRT_2
                } else {
                    cell
                };
                let dy = self.height[next as usize] - y;
                let cost = here + (flat * flat + dy * dy).sqrt();
                if self.seen[next as usize] != search || cost < self.g[next as usize] {
                    self.seen[next as usize] = search;
                    self.g[next as usize] = cost;
                    self.parent[next as usize] = node;
                    self.heap.push(Open {
                        f: cost + estimate(self.node_column[next as usize] as usize),
                        node: next,
                    });
                }
            }
        }
        false
    }

    /// Whether the grid has floor all along `a`→`b` without steps taller than
    /// the step height, and the raised collider sweeps along it untouched.
    fn clear(&self, ctx: &NavCtx, a: Vec3, b: Vec3) -> bool {
        let step = ctx.settings.controller.step_height;
        let d = b - a;
        let len = (d.x * d.x + d.z * d.z).sqrt();
        let samples = ((len / (self.cell * 0.5)).ceil() as usize).max(1);
        let mut y = a.y;
        for k in 1..=samples {
            let p = a + d * (k as f32 / samples as f32);
            let Some(c) = self.column_at(p.x, p.z) else {
                return false;
            };
            let ix = (c % self.nx) as i32;
            let iz = (c / self.nx) as i32;
            let Some(n) = self.link(ix, iz, y, step) else {
                return false;
            };
            y = self.height[n as usize];
        }
        let probe = self.collider.probe(a, yaw_of(d.x, d.z), step);
        ctx.scene
            .sweep(&probe.inflated(-ctx.settings.collision.skin), d, STATIC)
            .is_none()
    }

    /// Writes into `out` the waypoints from `from` to `to`, ending at `to`.
    pub fn find(&mut self, ctx: &NavCtx, from: Vec3, to: Vec3, out: &mut Vec<Vec3>) -> bool {
        out.clear();
        self.refresh(ctx);
        let Some(start) = self.nearest(ctx, from, None) else {
            return false;
        };
        let Some(goal) = self.nearest(ctx, to, Some(self.island[start as usize])) else {
            return false;
        };
        if self.island[start as usize] != self.island[goal as usize] {
            return false;
        }
        if !self.astar(ctx.settings.controller.step_height, start, goal) {
            return false;
        }
        self.chain.clear();
        let mut n = goal;
        while n != u32::MAX {
            self.chain.push(n);
            n = self.parent[n as usize];
        }
        self.chain.reverse();
        let mut points = std::mem::take(&mut self.points);
        points.clear();
        points.push(from);
        let inner = self.chain.len().saturating_sub(1);
        points.extend(
            self.chain
                .iter()
                .take(inner)
                .skip(1)
                .map(|&n| self.position(n)),
        );
        points.push(to);
        let mut anchor = 0;
        let mut i = 2;
        while i < points.len() {
            if self.clear(ctx, points[anchor], points[i]) {
                i += 1;
            } else {
                anchor = i - 1;
                out.push(points[anchor]);
                i = anchor + 2;
            }
        }
        out.push(to);
        let mut prev = from;
        let ok = out.iter().all(|&p| {
            let ok = self.clear(ctx, prev, p);
            prev = p;
            ok
        });
        self.points = points;
        if !ok {
            out.clear();
        }
        ok
    }

    /// Walkable cells for drawing: centre and feet height of every node.
    pub fn cells(&self) -> impl Iterator<Item = Vec3> + '_ {
        (0..self.height.len() as u32).map(|n| self.position(n))
    }
}
