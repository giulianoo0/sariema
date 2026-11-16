// SPDX-License-Identifier: Apache-2.0

//! Light culling and clustering on the CPU. Everything here is a pure function
//! of the lights, the camera and the screen, independent of how the result
//! reaches the GPU.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use glam::{Mat4, Vec2, Vec3, Vec4, vec3};
use sariema_format::{Light, LightKind};

use crate::Quality;

/// Low bits of a cluster cell that hold its light count; the high bits hold
/// the offset of its first entry in the index list.
pub(crate) const COUNT_BITS: u32 = 8;
pub(crate) const MAX_CLUSTER_LIGHTS: u32 = (1 << COUNT_BITS) - 1;
pub(crate) const MAX_INDEX_ENTRIES: u32 = 1 << (32 - COUNT_BITS);

/// Screen tiles by logarithmic depth slices between view depths `near` and
/// `far`. Tile (0, 0) is the top-left corner.
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    pub width: u32,
    pub height: u32,
    pub tile_px: u32,
    pub tiles_x: u32,
    pub tiles_y: u32,
    pub slices: u32,
    pub near: f32,
    pub far: f32,
}

impl Grid {
    /// The smallest tiles of at least `quality.cluster_tile_px` that keep the
    /// grid within `quality.cluster_budget`. Its slices span every positive
    /// depth until [`Grid::sliced`].
    pub fn new(width: u32, height: u32, quality: &Quality) -> Grid {
        let slices = quality.depth_slices;
        let count =
            |tile: u32| width.div_ceil(tile) as u64 * height.div_ceil(tile) as u64 * slices as u64;
        // The count never grows with the tile, so bisection finds the smallest
        // that fits, or else a tile as large as the screen.
        let (mut lo, mut hi) = (quality.cluster_tile_px, width.max(height));
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if count(mid) <= quality.cluster_budget as u64 {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        Grid {
            width,
            height,
            tile_px: lo,
            tiles_x: width.div_ceil(lo),
            tiles_y: height.div_ceil(lo),
            slices,
            near: f32::MIN_POSITIVE,
            far: f32::MAX,
        }
    }

    pub fn sliced(&self, near: f32, far: f32) -> Grid {
        Grid {
            near,
            far,
            ..self.clone()
        }
    }

    pub fn len(&self) -> u32 {
        self.tiles_x * self.tiles_y * self.slices
    }

    /// `(scale, bias)` such that a view depth `z` lies in slice
    /// `floor(ln(z) * scale + bias)`.
    pub fn slicing(&self) -> (f32, f32) {
        let scale = self.slices as f32 / (self.far / self.near).ln();
        (scale, -self.near.ln() * scale)
    }

    fn slice(&self, depth: f32) -> u32 {
        let (scale, bias) = self.slicing();
        to_cell(depth.ln() * scale + bias, self.slices)
    }

    /// The clusters a view-space sphere's bounding box overlaps.
    fn span(&self, proj: Mat4, sphere: &Sphere) -> Option<Span> {
        let full = Span {
            x: (0, self.tiles_x - 1),
            y: (0, self.tiles_y - 1),
            slice: (0, self.slices - 1),
        };
        let Sphere {
            center: c,
            radius: r,
        } = *sphere;
        if !r.is_finite() {
            return Some(full);
        }
        let near_z = (c.z + r).min(-self.near);
        let far_z = (c.z - r).max(-self.far);
        if near_z < far_z {
            return None;
        }
        // With z < 0, x / -z is monotonic in x and in z, so the box's
        // projection is bounded by the projections of its corners.
        let (mut lo, mut hi) = (Vec2::INFINITY, Vec2::NEG_INFINITY);
        for x in [c.x - r, c.x + r] {
            for y in [c.y - r, c.y + r] {
                for z in [near_z, far_z] {
                    let ndc = proj.project_point3(vec3(x, y, z)).truncate();
                    lo = lo.min(ndc);
                    hi = hi.max(ndc);
                }
            }
        }
        if hi.x < -1.0 || lo.x > 1.0 || hi.y < -1.0 || lo.y > 1.0 {
            return None;
        }
        let tile = self.tile_px as f32;
        let column = |ndc: f32| to_cell((ndc * 0.5 + 0.5) * self.width as f32 / tile, self.tiles_x);
        let row = |ndc: f32| to_cell((0.5 - ndc * 0.5) * self.height as f32 / tile, self.tiles_y);
        Some(Span {
            x: (column(lo.x), column(hi.x)),
            y: (row(hi.y), row(lo.y)),
            slice: (self.slice(-near_z), self.slice(-far_z)),
        })
    }

    fn clusters(&self, span: &Span) -> impl Iterator<Item = usize> {
        let (tiles_x, tiles_y) = (self.tiles_x, self.tiles_y);
        let Span { x, y, slice } = *span;
        (slice.0..=slice.1).flat_map(move |s| {
            (y.0..=y.1).flat_map(move |ty| {
                (x.0..=x.1).map(move |tx| ((s * tiles_y + ty) * tiles_x + tx) as usize)
            })
        })
    }
}

/// `floor(v)` clamped to `0..cells`.
fn to_cell(v: f32, cells: u32) -> u32 {
    (v.floor().max(0.0) as u32).min(cells - 1)
}

/// Inclusive ranges of tile columns, tile rows and depth slices.
#[derive(Clone, Copy, Debug)]
struct Span {
    x: (u32, u32),
    y: (u32, u32),
    slice: (u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}

/// From a spot's position towards its target; straight down if they coincide.
pub(crate) fn spot_axis(position: [f32; 3], target: [f32; 3]) -> Vec3 {
    (Vec3::from(target) - Vec3::from(position)).normalize_or(Vec3::NEG_Y)
}

/// View depths of the near and far planes of `proj`.
pub(crate) fn depth_range(proj: Mat4) -> (f32, f32) {
    let unproject = proj.inverse();
    let depth = |ndc_z| -unproject.project_point3(vec3(0.0, 0.0, ndc_z)).z;
    (depth(0.0), depth(1.0))
}

/// The view depths the view-space `spheres` reach, within `proj`'s near and
/// far planes; all of them if none of the spheres is there.
pub(crate) fn light_depths(proj: Mat4, spheres: &[Sphere]) -> (f32, f32) {
    let (near, far) = depth_range(proj);
    let nearest = spheres
        .iter()
        .map(|s| -s.center.z - s.radius)
        .fold(f32::INFINITY, f32::min);
    let farthest = spheres
        .iter()
        .map(|s| s.radius - s.center.z)
        .fold(f32::NEG_INFINITY, f32::max);
    let (lo, hi) = (nearest.max(near), farthest.min(far));
    if lo < hi { (lo, hi) } else { (near, far) }
}

/// How much `light` matters at `point`: `importance × intensity / distance²`.
pub(crate) fn priority(light: &Light, point: Vec3) -> f32 {
    let distance2 = point.distance_squared(light.position.into());
    light.importance * light.intensity / distance2.max(f32::MIN_POSITIVE)
}

/// World-space sphere around everything `light` can reach. A range of 0 or
/// less means no cutoff, as three.js treats `distance = 0`.
pub(crate) fn reach(light: &Light) -> Sphere {
    let position = Vec3::from(light.position);
    let range = if light.range > 0.0 {
        light.range
    } else {
        f32::INFINITY
    };
    let LightKind::Spot { target, cone_deg } = light.kind else {
        return Sphere {
            center: position,
            radius: range,
        };
    };
    let half = (cone_deg * 0.5).to_radians();
    let axis = spot_axis(light.position, target);
    if !range.is_finite() || half >= FRAC_PI_2 {
        Sphere {
            center: position,
            radius: range,
        }
    } else if half > FRAC_PI_4 {
        // The rim circle's sphere also holds the apex and the cap.
        Sphere {
            center: position + axis * range * half.cos(),
            radius: range * half.sin(),
        }
    } else {
        let radius = range / (2.0 * half.cos());
        Sphere {
            center: position + axis * radius,
            radius,
        }
    }
}

/// Lights chosen for a frame, highest priority first.
#[derive(Debug, Default)]
pub(crate) struct Visible {
    /// Indices into the room's lights.
    pub lights: Vec<usize>,
    /// Their reach in view space, in the same order.
    pub spheres: Vec<Sphere>,
    pub in_frustum: u32,
    pub dropped: u32,
}

/// The lights whose reach meets the view frustum, keeping at most `max` by
/// [`priority`] at the eye.
pub(crate) fn visible_lights(
    lights: &[Light],
    view: Mat4,
    proj: Mat4,
    eye: Vec3,
    max: u32,
) -> Visible {
    let planes = frustum_planes(proj);
    let mut found: Vec<(f32, usize, Sphere)> = lights
        .iter()
        .enumerate()
        .filter_map(|(i, light)| {
            let world = reach(light);
            let sphere = Sphere {
                center: view.transform_point3(world.center),
                radius: world.radius,
            };
            let inside = planes
                .iter()
                .all(|p| p.dot(sphere.center.extend(1.0)) >= -sphere.radius);
            inside.then_some((priority(light, eye), i, sphere))
        })
        .collect();
    let in_frustum = found.len() as u32;
    // Stable, so equal priorities keep the room's order.
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    found.truncate(max as usize);
    Visible {
        lights: found.iter().map(|f| f.1).collect(),
        spheres: found.iter().map(|f| f.2).collect(),
        in_frustum,
        dropped: in_frustum - found.len() as u32,
    }
}

/// View-space planes (normal xyz, offset w) with the inside where `p · (x, 1) >= 0`.
fn frustum_planes(proj: Mat4) -> [Vec4; 6] {
    let row = |i| proj.row(i);
    // NDC depth runs 0..1, so the near plane is the third row alone.
    [
        row(3) + row(0),
        row(3) - row(0),
        row(3) + row(1),
        row(3) - row(1),
        row(2),
        row(3) - row(2),
    ]
    .map(|p| p / p.truncate().length())
}

/// The per-frame cluster lists.
#[derive(Debug, Default)]
pub(crate) struct Bins {
    /// One per cluster: offset into `indices` << [`COUNT_BITS`] | count.
    pub cells: Vec<u32>,
    /// Positions in the sphere list given to [`bin`].
    pub indices: Vec<u32>,
    /// Most lights overlapping one cluster, counting those past the cap.
    pub max_overlapping: u32,
    /// Clusters that had more lights than the cap.
    pub overflows: u32,
}

/// Lists, for each cluster, the view-space spheres whose bounding box
/// overlaps it. A cluster keeps the first `max_per_cluster` it meets, so
/// `spheres` must come in priority order.
pub(crate) fn bin(grid: &Grid, proj: Mat4, spheres: &[Sphere], max_per_cluster: u32) -> Bins {
    let spans: Vec<Option<Span>> = spheres.iter().map(|s| grid.span(proj, s)).collect();
    let mut counts = vec![0u32; grid.len() as usize];
    for span in spans.iter().flatten() {
        for c in grid.clusters(span) {
            counts[c] += 1;
        }
    }
    let mut offset = 0;
    let cells: Vec<u32> = counts
        .iter()
        .map(|&n| {
            let kept = n.min(max_per_cluster);
            let cell = offset << COUNT_BITS | kept;
            offset += kept;
            cell
        })
        .collect();
    let mut indices = vec![0; offset as usize];
    let mut filled = vec![0u32; counts.len()];
    for (light, span) in spans.iter().enumerate() {
        for c in span.iter().flat_map(|span| grid.clusters(span)) {
            if filled[c] < max_per_cluster {
                indices[((cells[c] >> COUNT_BITS) + filled[c]) as usize] = light as u32;
                filled[c] += 1;
            }
        }
    }
    Bins {
        max_overlapping: counts.iter().copied().max().unwrap_or(0),
        overflows: counts.iter().filter(|&&n| n > max_per_cluster).count() as u32,
        cells,
        indices,
    }
}

#[cfg(test)]
mod tests {
    use glam::camera::rh::proj::directx::perspective;

    use super::*;

    const W: u32 = 960;
    const H: u32 = 540;

    fn proj() -> Mat4 {
        perspective(60f32.to_radians(), W as f32 / H as f32, 0.05, 100.0)
    }

    fn grid() -> Grid {
        let quality = Quality {
            cluster_budget: 10_000,
            ..Quality::low()
        };
        Grid::new(W, H, &quality).sliced(0.05, 100.0)
    }

    /// `(tile x, tile y, slice, lights)` of every non-empty cluster.
    fn occupied(grid: &Grid, bins: &Bins) -> Vec<(u32, u32, u32, Vec<u32>)> {
        let mask = (1 << COUNT_BITS) - 1;
        (0..grid.len())
            .filter_map(|i| {
                let cell = bins.cells[i as usize];
                let (offset, count) = ((cell >> COUNT_BITS) as usize, (cell & mask) as usize);
                let tile = i % (grid.tiles_x * grid.tiles_y);
                (count > 0).then(|| {
                    (
                        tile % grid.tiles_x,
                        tile / grid.tiles_x,
                        i / (grid.tiles_x * grid.tiles_y),
                        bins.indices[offset..offset + count].to_vec(),
                    )
                })
            })
            .collect()
    }

    fn sphere(x: f32, y: f32, z: f32, radius: f32) -> Sphere {
        Sphere {
            center: vec3(x, y, z),
            radius,
        }
    }

    #[test]
    fn grid_grows_tiles_to_fit_budget() {
        let quality = Quality {
            cluster_budget: 100,
            cluster_tile_px: 16,
            depth_slices: 4,
            ..Quality::low()
        };
        let g = Grid::new(W, H, &quality);
        assert!(g.len() <= 100 && g.tile_px > 16, "{g:?}");
        assert_eq!(grid().tile_px, 64);
    }

    #[test]
    fn tile_is_the_smallest_that_fits_the_budget() {
        for (budget, tile, slices) in [(100, 16, 4), (1536, 64, 16), (3072, 8, 16), (5, 1, 1)] {
            let quality = Quality {
                cluster_budget: budget,
                cluster_tile_px: tile,
                depth_slices: slices,
                ..Quality::low()
            };
            let g = Grid::new(W, H, &quality);
            assert!(g.len() <= budget, "{g:?}");
            let one_smaller = W.div_ceil(g.tile_px - 1) * H.div_ceil(g.tile_px - 1) * slices;
            assert!(g.tile_px == tile || one_smaller > budget, "{g:?}");
        }
    }

    #[test]
    fn depth_range_comes_from_the_projection() {
        let (near, far) = depth_range(perspective(1.0, 1.5, 0.3, 40.0));
        assert!(
            (near - 0.3).abs() < 1e-5 && (far - 40.0).abs() < 1e-2,
            "{near} {far}"
        );
    }

    #[test]
    fn slices_span_only_the_depths_lights_reach() {
        let spheres = [sphere(0.0, 0.0, -12.0, 2.0), sphere(1.0, 0.0, -20.0, 1.0)];
        assert_eq!(light_depths(proj(), &spheres), (10.0, 21.0));
        let (near, far) = depth_range(proj());
        let past_both_planes = sphere(0.0, 0.0, -50.0, f32::INFINITY);
        assert_eq!(light_depths(proj(), &[past_both_planes]), (near, far));
        assert_eq!(light_depths(proj(), &[]), (near, far));
    }

    #[test]
    fn light_on_the_left_is_in_no_right_half_cluster() {
        let g = grid();
        let bins = bin(&g, proj(), &[sphere(-4.0, 0.0, -10.0, 1.0)], 8);
        let cells = occupied(&g, &bins);
        assert!(!cells.is_empty());
        let first_right_tile = (W / 2).div_ceil(g.tile_px);
        assert!(cells.iter().all(|c| c.0 < first_right_tile), "{cells:?}");
    }

    #[test]
    fn light_beyond_the_far_slice_is_in_no_near_slice() {
        let g = grid();
        let bins = bin(&g, proj(), &[sphere(0.0, 0.0, -120.0, 30.0)], 8);
        let cells = occupied(&g, &bins);
        assert!(!cells.is_empty());
        assert!(cells.iter().all(|c| c.2 >= g.slices / 2), "{cells:?}");
    }

    #[test]
    fn light_behind_the_camera_is_in_no_cluster() {
        let g = grid();
        let bins = bin(&g, proj(), &[sphere(0.0, 0.0, 5.0, 1.0)], 8);
        assert!(occupied(&g, &bins).is_empty());
    }

    #[test]
    fn full_cluster_keeps_the_first_lights() {
        let g = grid();
        let same = sphere(0.0, 0.0, -10.0, 0.5);
        let bins = bin(&g, proj(), &[same, same, same], 2);
        let cells = occupied(&g, &bins);
        assert!(cells.iter().all(|c| c.3 == [0, 1]), "{cells:?}");
        assert_eq!(bins.overflows as usize, cells.len());
        assert_eq!(bins.max_overlapping, 3);
    }

    fn point(x: f32, z: f32, intensity: f32) -> Light {
        Light {
            name: String::new(),
            kind: LightKind::Point,
            color: [1.0; 3],
            intensity,
            position: [x, 0.0, z],
            range: 2.0,
            flicker: None,
            casts_shadow: false,
            mobility: sariema_format::Mobility::Static,
            importance: 1.0,
        }
    }

    #[test]
    fn culling_drops_lights_out_of_view_and_keeps_the_strongest() {
        let lights = [
            point(0.0, -10.0, 1.0),
            point(0.0, 10.0, 9.0),
            point(1.0, -10.0, 4.0),
        ];
        let v = visible_lights(&lights, Mat4::IDENTITY, proj(), Vec3::ZERO, 1);
        assert_eq!((v.in_frustum, v.dropped), (2, 1));
        assert_eq!(v.lights, [2]);
    }

    #[test]
    fn wide_spot_reach_holds_apex_and_cap() {
        let spot = Light {
            kind: LightKind::Spot {
                target: [0.0, 0.0, 0.0],
                cone_deg: 120.0,
            },
            position: [0.0, 4.0, 0.0],
            range: 5.0,
            ..point(0.0, 0.0, 1.0)
        };
        let s = reach(&spot);
        let rim = vec3(5.0 * 60f32.to_radians().sin(), 4.0 - 5.0 * 0.5, 0.0);
        for p in [vec3(0.0, 4.0, 0.0), vec3(0.0, -1.0, 0.0), rim] {
            assert!(p.distance(s.center) <= s.radius + 1e-4, "{p} outside {s:?}");
        }
        assert!(s.radius < 5.0);
    }
}
