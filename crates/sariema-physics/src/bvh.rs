// SPDX-License-Identifier: Apache-2.0

//! A bounding volume hierarchy over the static parts, built once.
//!
//! Median split on the longest axis of the centroids, ties broken by item
//! index, so the same input always builds the same tree. [`BvhBuild`] builds
//! it a share of its work at a time, for a host that cannot spend a whole
//! room's tree in one frame.

use std::cmp::Ordering;

use crate::math::Bounds;

#[derive(Clone, Debug, PartialEq)]
struct Node {
    bounds: Bounds,
    /// Leaf: first index into `items`. Inner: index of the left child; the
    /// right child follows it.
    first: u32,
    /// Items in a leaf; 0 for an inner node.
    count: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
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
/// Items a node's split sorts at once before merging the runs.
const RUN: usize = 1024;
/// Work units a sorted run costs per item, about its comparisons.
const RUN_COST: usize = 10;

/// A [`Bvh`] built in shares: each node's bounds folded, then its items
/// sorted on the split axis run by run and merged, a bounded number of
/// items a call. It gives the tree [`Bvh::build`] gives: the order sorted
/// is total (the item index breaks ties), so any sort finds it.
#[derive(Debug)]
pub struct BvhBuild {
    items: Vec<u32>,
    nodes: Vec<Node>,
    pending: Vec<(usize, usize, usize)>,
    node: Option<NodeWork>,
}

#[derive(Debug)]
struct NodeWork {
    node: usize,
    start: usize,
    end: usize,
    stage: Stage,
}

#[derive(Debug)]
enum Stage {
    Fold {
        at: usize,
        total: Bounds,
        centroids: Bounds,
    },
    Keys {
        axis: usize,
        keys: Vec<(f32, u32)>,
    },
    Sort(Sorter),
    Write {
        at: usize,
        keys: Vec<(f32, u32)>,
    },
}

/// A bottom-up merge sort of `(key, item)` pairs that stops and resumes.
#[derive(Debug)]
struct Sorter {
    keys: Vec<(f32, u32)>,
    out: Vec<(f32, u32)>,
    /// Width of the sorted runs being merged; 0 while sorting the runs.
    width: usize,
    /// Start of the run (or pair of runs) at hand.
    lo: usize,
    l: usize,
    r: usize,
    o: usize,
}

fn order(a: &(f32, u32), b: &(f32, u32)) -> Ordering {
    a.0.total_cmp(&b.0).then(a.1.cmp(&b.1))
}

impl Sorter {
    fn new(keys: Vec<(f32, u32)>) -> Sorter {
        Sorter {
            out: vec![(0.0, 0); keys.len()],
            keys,
            width: 0,
            lo: 0,
            l: 0,
            r: 0,
            o: 0,
        }
    }

    fn pair(&mut self) {
        let n = self.keys.len();
        self.l = self.lo;
        self.r = (self.lo + self.width).min(n);
        self.o = self.lo;
    }

    /// Sorts on until `units` are spent; true once sorted, in `keys`.
    fn advance(&mut self, units: &mut usize) -> bool {
        let n = self.keys.len();
        while self.width == 0 {
            if self.lo >= n {
                self.width = RUN;
                self.lo = 0;
                self.pair();
                break;
            }
            if *units == 0 {
                return false;
            }
            let end = (self.lo + RUN).min(n);
            self.keys[self.lo..end].sort_unstable_by(order);
            *units = units.saturating_sub((end - self.lo) * RUN_COST);
            self.lo = end;
        }
        while self.width < n {
            let mid = (self.lo + self.width).min(n);
            let hi = (self.lo + 2 * self.width).min(n);
            while self.o < hi {
                if *units == 0 {
                    return false;
                }
                *units -= 1;
                let left = self.r >= hi
                    || (self.l < mid && order(&self.keys[self.l], &self.keys[self.r]).is_lt());
                if left {
                    self.out[self.o] = self.keys[self.l];
                    self.l += 1;
                } else {
                    self.out[self.o] = self.keys[self.r];
                    self.r += 1;
                }
                self.o += 1;
            }
            self.lo = hi;
            if self.lo >= n {
                std::mem::swap(&mut self.keys, &mut self.out);
                self.width *= 2;
                self.lo = 0;
            }
            self.pair();
        }
        true
    }
}

impl BvhBuild {
    /// A tree over `count` items, none of its work done.
    pub fn new(count: usize) -> BvhBuild {
        let mut build = BvhBuild {
            items: (0..count as u32).collect(),
            nodes: Vec::with_capacity(count.div_ceil(LEAF) * 2),
            pending: Vec::new(),
            node: None,
        };
        if count > 0 {
            build.nodes.push(Node {
                bounds: Bounds::EMPTY,
                first: 0,
                count: 0,
            });
            build.pending.push((0, 0, count));
        }
        build
    }

    /// Builds on until about `units` are spent, an item folded, keyed,
    /// merged or written a unit; true once the tree is built.
    pub fn advance(&mut self, bounds: &[Bounds], mut units: usize) -> bool {
        loop {
            let work = match self.node.take() {
                Some(work) => work,
                None => match self.pending.pop() {
                    Some((node, start, end)) => NodeWork {
                        node,
                        start,
                        end,
                        stage: Stage::Fold {
                            at: start,
                            total: Bounds::EMPTY,
                            centroids: Bounds::EMPTY,
                        },
                    },
                    None => return true,
                },
            };
            if units == 0 {
                self.node = Some(work);
                return false;
            }
            self.node = self.step(bounds, work, &mut units);
        }
    }

    /// Takes the tree once [`BvhBuild::advance`] says it is built.
    pub fn finish(self, bounds: &[Bounds]) -> Bvh {
        let item_bounds = self.items.iter().map(|&i| bounds[i as usize]).collect();
        Bvh {
            nodes: self.nodes,
            items: self.items,
            item_bounds,
        }
    }

    /// Moves `work` on within `units`; what is left of it.
    fn step(&mut self, bounds: &[Bounds], work: NodeWork, units: &mut usize) -> Option<NodeWork> {
        let NodeWork {
            node,
            start,
            end,
            stage,
        } = work;
        let next = match stage {
            Stage::Fold {
                mut at,
                mut total,
                mut centroids,
            } => {
                let stop = end.min(at.saturating_add(*units));
                for &i in &self.items[at..stop] {
                    let b = bounds[i as usize];
                    total = total.union(b);
                    centroids = centroids.with(b.center());
                }
                *units -= stop - at;
                at = stop;
                if at < end {
                    Stage::Fold {
                        at,
                        total,
                        centroids,
                    }
                } else {
                    self.nodes[node].bounds = total;
                    if end - start <= LEAF {
                        self.nodes[node].first = start as u32;
                        self.nodes[node].count = (end - start) as u32;
                        return None;
                    }
                    let extent = centroids.max - centroids.min;
                    let axis = if extent.x >= extent.y && extent.x >= extent.z {
                        0
                    } else if extent.y >= extent.z {
                        1
                    } else {
                        2
                    };
                    Stage::Keys {
                        axis,
                        keys: Vec::with_capacity(end - start),
                    }
                }
            }
            Stage::Keys { axis, mut keys } => {
                let at = start + keys.len();
                let stop = end.min(at.saturating_add(*units));
                keys.extend(
                    self.items[at..stop]
                        .iter()
                        .map(|&i| (bounds[i as usize].center()[axis], i)),
                );
                *units -= stop - at;
                if stop < end {
                    Stage::Keys { axis, keys }
                } else {
                    Stage::Sort(Sorter::new(keys))
                }
            }
            Stage::Sort(mut sorter) => {
                if sorter.advance(units) {
                    Stage::Write {
                        at: start,
                        keys: sorter.keys,
                    }
                } else {
                    Stage::Sort(sorter)
                }
            }
            Stage::Write { mut at, keys } => {
                let stop = end.min(at.saturating_add(*units));
                for (item, &(_, i)) in self.items[at..stop].iter_mut().zip(&keys[at - start..]) {
                    *item = i;
                }
                *units -= stop - at;
                at = stop;
                if at < end {
                    Stage::Write { at, keys }
                } else {
                    let mid = start + (end - start) / 2;
                    let left = self.nodes.len();
                    for _ in 0..2 {
                        self.nodes.push(Node {
                            bounds: Bounds::EMPTY,
                            first: 0,
                            count: 0,
                        });
                    }
                    self.nodes[node].first = left as u32;
                    self.pending.push((left + 1, mid, end));
                    self.pending.push((left, start, mid));
                    return None;
                }
            }
        };
        Some(NodeWork {
            node,
            start,
            end,
            stage: next,
        })
    }
}

impl Bvh {
    pub fn build(bounds: &[Bounds]) -> Bvh {
        let mut build = BvhBuild::new(bounds.len());
        while !build.advance(bounds, usize::MAX) {}
        build.finish(bounds)
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

    /// The tree as it was built in one go before shares, kept to compare.
    fn whole(bounds: &[Bounds]) -> Bvh {
        let mut items: Vec<u32> = (0..bounds.len() as u32).collect();
        let mut nodes = Vec::new();
        if items.is_empty() {
            return Bvh::default();
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

    #[test]
    fn a_tree_built_a_share_at_a_time_is_the_tree_built_whole() {
        let mut seed = 7u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        // Coarse centres, so many items tie on an axis and the index decides.
        let boxes: Vec<Bounds> = (0..5000)
            .map(|_| {
                let c = Vec3::new(
                    (next() * 40.0).floor(),
                    (next() * 4.0).floor(),
                    (next() * 30.0).floor(),
                );
                let h = Vec3::splat(0.1 + next());
                Bounds {
                    min: c - h,
                    max: c + h,
                }
            })
            .collect();
        let expected = whole(&boxes);
        assert_eq!(Bvh::build(&boxes), expected);
        for units in [1, 7, 333, 4096] {
            let mut build = BvhBuild::new(boxes.len());
            let mut shares = 1;
            while !build.advance(&boxes, units) {
                shares += 1;
            }
            assert!(shares > 5000 / units.max(1), "{shares} shares of {units}");
            assert_eq!(build.finish(&boxes), expected, "in shares of {units}");
        }
        assert_eq!(Bvh::build(&[]), whole(&[]));
    }
}
