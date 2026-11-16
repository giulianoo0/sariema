// SPDX-License-Identifier: Apache-2.0

//! A bounding volume hierarchy over the static parts, built once.
//!
//! Median split on the longest axis of the centroids, ties broken by item
//! index, so the same input always builds the same tree.

use crate::math::Bounds;

#[derive(Clone, Debug)]
struct Node {
    bounds: Bounds,
    /// Leaf: first index into `items`. Inner: index of the left child; the
    /// right child follows it.
    first: u32,
    /// Items in a leaf; 0 for an inner node.
    count: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    items: Vec<u32>,
    /// Bounds of `items[i]`.
    item_bounds: Vec<Bounds>,
}

/// Items per leaf at most.
const LEAF: usize = 4;
/// A median split halves the items, so 64 levels hold any `u32` count.
const STACK: usize = 64;

impl Bvh {
    pub fn build(bounds: &[Bounds]) -> Bvh {
        let mut items: Vec<u32> = (0..bounds.len() as u32).collect();
        let mut nodes = Vec::with_capacity(bounds.len().div_ceil(LEAF) * 2);
        if items.is_empty() {
            return Bvh {
                nodes,
                items,
                item_bounds: Vec::new(),
            };
        }
        nodes.push(Node {
            bounds: Bounds::EMPTY,
            first: 0,
            count: 0,
        });
        let mut pending = vec![(0usize, 0usize, items.len())];
        while let Some((node, start, end)) = pending.pop() {
            let span = &mut items[start..end];
            let total = span
                .iter()
                .fold(Bounds::EMPTY, |b, &i| b.union(bounds[i as usize]));
            nodes[node].bounds = total;
            if span.len() <= LEAF {
                nodes[node].first = start as u32;
                nodes[node].count = span.len() as u32;
                continue;
            }
            let centroids = span
                .iter()
                .fold(Bounds::EMPTY, |b, &i| b.with(bounds[i as usize].center()));
            let extent = centroids.max - centroids.min;
            let axis = if extent.x >= extent.y && extent.x >= extent.z {
                0
            } else if extent.y >= extent.z {
                1
            } else {
                2
            };
            span.sort_unstable_by(|&a, &b| {
                let ka = bounds[a as usize].center()[axis];
                let kb = bounds[b as usize].center()[axis];
                ka.total_cmp(&kb).then(a.cmp(&b))
            });
            let mid = start + span.len() / 2;
            let left = nodes.len();
            for _ in 0..2 {
                nodes.push(Node {
                    bounds: Bounds::EMPTY,
                    first: 0,
                    count: 0,
                });
            }
            nodes[node].first = left as u32;
            pending.push((left + 1, mid, end));
            pending.push((left, start, mid));
        }
        let item_bounds = items.iter().map(|&i| bounds[i as usize]).collect();
        Bvh {
            nodes,
            items,
            item_bounds,
        }
    }

    /// Calls `visit` with every item whose bounds overlap `area`.
    pub fn query(&self, area: &Bounds, mut visit: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = [0u32; STACK];
        let mut top = 1;
        while top > 0 {
            top -= 1;
            let node = &self.nodes[stack[top] as usize];
            if !node.bounds.overlaps(area) {
                continue;
            }
            if node.count > 0 {
                let span = node.first as usize..(node.first + node.count) as usize;
                for (item, b) in self.items[span.clone()].iter().zip(&self.item_bounds[span]) {
                    if b.overlaps(area) {
                        visit(*item);
                    }
                }
            } else {
                stack[top] = node.first + 1;
                stack[top + 1] = node.first;
                top += 2;
            }
        }
    }

    /// Bounds of everything in the tree.
    pub fn bounds(&self) -> Bounds {
        self.nodes.first().map_or(Bounds::EMPTY, |n| n.bounds)
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    #[test]
    fn query_returns_exactly_the_overlapping_items() {
        let boxes: Vec<Bounds> = (0..200)
            .map(|i| {
                let x = (i % 20) as f32;
                let z = (i / 20) as f32;
                Bounds {
                    min: Vec3::new(x, 0.0, z),
                    max: Vec3::new(x + 0.5, 1.0, z + 0.5),
                }
            })
            .collect();
        let bvh = Bvh::build(&boxes);
        let area = Bounds {
            min: Vec3::new(3.2, 0.5, 4.2),
            max: Vec3::new(6.1, 0.6, 5.1),
        };
        let mut found = Vec::new();
        bvh.query(&area, |i| found.push(i));
        found.sort_unstable();
        let expected: Vec<u32> = (0..200u32)
            .filter(|&i| boxes[i as usize].overlaps(&area))
            .collect();
        assert_eq!(found, expected);
        assert!(!found.is_empty());
    }
}
